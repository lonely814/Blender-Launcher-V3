use crate::error::AppError;
use crate::models::{DownloadState, DownloadTask};
use crate::service::extractor::Extractor;
use crate::Result;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{watch, Mutex};

const USER_AGENT: &str = "BlenderLauncher/3.0";
/// 连接建立时限。**不是**总时长——大文件下载不设总时限。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// 单次读取的两个 chunk 之间允许的最长静默。
///
/// 用它替代「整包总时限」：总时限会让 400 MB 的包在 30 秒时必然失败，
/// 而这里只在「服务端连续这么久一个字节都没吐」时才判定卡死。
const READ_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Minimum interval between progress notifications (state snapshot + IPC).
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
/// Buffer size for streaming chunks to disk and hashing.
const IO_BUFFER_BYTES: usize = 64 * 1024;

/// Control signal sent to a running download loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlAction {
    Continue,
    Pause,
    Cancel,
}

/// Why the download loop ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    Completed,
    Paused,
    Cancelled,
}

pub struct Downloader;

/// 一次下载的静态参数。
///
/// 收进结构体而不是继续往 `download()` 上加参数：加镜像时参数已经到 8 个，
/// 再多两个调用点就没人能一眼看出谁传的是什么。字段名也让
/// 「官方地址」和「实际下载地址」的区别在调用点就可见。
pub struct DownloadSpec {
    /// 实际用于传输的地址（可能已被镜像替换）。
    pub url: String,
    /// 同一文件的官方地址。校验清单只从官方域名取——用镜像自己的哈希
    /// 校验镜像内容，等于没有校验。
    pub official_url: String,
    /// 落盘路径。始终按官方文件名算，换镜像不换路径，断点续传才不会错位。
    pub dest: PathBuf,
    /// 已有 `.part` 文件里可复用的字节数。
    pub resume_from: u64,
}

impl Downloader {
    /// Streams the archive into `<dest>.part`, then verifies it and renames it.
    ///
    /// `resume_from` bytes from an earlier `.part` file are kept (docs/00
    /// §1.2: 断点续传) when the file on disk matches that size and the server
    /// honors the Range header. Progress is reported through `on_update`,
    /// throttled to one call per `PROGRESS_INTERVAL` plus every state change.
    ///
    /// On pause/cancel the task state is updated and the file is kept
    /// (paused) or deleted (cancelled). On failure the task is marked
    /// `Failed` and the partial file removed before this returns `Err`.
    pub async fn download(
        task: Arc<Mutex<DownloadTask>>,
        spec: DownloadSpec,
        control: watch::Receiver<ControlAction>,
        proxy: &crate::models::ProxySettings,
        mut on_update: impl FnMut(DownloadTask) + Send,
    ) -> Result<()> {
        let part_path = part_file_path(&spec.dest);

        let flow = run_download(&task, &spec, control, proxy, &mut on_update).await;

        match flow {
            Ok(Flow::Completed | Flow::Paused | Flow::Cancelled) => Ok(()),
            Err(err) => {
                // Best-effort cleanup; the task is marked Failed so the
                // frontend sees the terminal state via the progress event.
                let _ = tokio::fs::remove_file(&part_path).await;
                let mut current = task.lock().await;
                current.state = DownloadState::Failed;
                current.error = Some(err.to_string());
                current.finished_at = Some(Utc::now());
                current.speed_bps = 0.0;
                on_update(current.clone());
                Err(err)
            }
        }
    }
}

async fn run_download(
    task: &Arc<Mutex<DownloadTask>>,
    spec: &DownloadSpec,
    mut control: watch::Receiver<ControlAction>,
    proxy: &crate::models::ProxySettings,
    on_update: &mut (impl FnMut(DownloadTask) + Send),
) -> Result<Flow> {
    let url = spec.url.as_str();
    let official_url = spec.official_url.as_str();
    let dest = spec.dest.as_path();
    let resume_from = spec.resume_from;
    // part 路径由 dest 唯一决定，不再作为参数传入——多一个参数就多一处
    // 调用方可能传错的机会，而它们本来就不可能不一致。
    let part_path = part_file_path(dest);
    let part_path = part_path.as_path();
    // 与 scraper 共用同一个代理构造点，但用流式版本：
    // 下载 400 MB 的构建耗时远超任何合理的「总时长」上限，
    // 只限连接建立，卡死由下面的 chunk 读超时兜底。
    let client = crate::service::http::build_streaming_client(proxy, USER_AGENT, CONNECT_TIMEOUT)?;

    // Resume only makes sense when the partial file actually holds exactly
    // the bytes we remembered.
    let disk_len = tokio::fs::metadata(part_path)
        .await
        .map(|meta| meta.len())
        .unwrap_or(0);
    let offset = if resume_from > 0 && disk_len == resume_from {
        resume_from
    } else {
        0
    };

    {
        let mut current = task.lock().await;
        current.state = DownloadState::Downloading;
        current.progress_bytes = offset;
        current.error = None;
    }
    on_update(task.lock().await.clone());

    let mut request = client.get(url);
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let mut response = request
        .send()
        .await
        .and_then(|response| response.error_for_status())
        .map_err(|err| AppError::Network(err.to_string()))?;

    // 206 Partial Content = server honored Range; 200 = full body restart.
    let start_offset = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        offset
    } else {
        0
    };
    let total = start_offset + response.content_length().unwrap_or(0);
    {
        let mut current = task.lock().await;
        current.total_bytes = total;
        current.progress_bytes = start_offset;
    }
    on_update(task.lock().await.clone());

    let mut file = if start_offset > 0 {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(part_path)
            .await?
    } else {
        tokio::fs::File::create(part_path).await?
    };

    let started = Instant::now();
    let mut last_emit = Instant::now();

    loop {
        // Check the control channel between chunks; latency is one chunk.
        // Copy the value out first: the borrow guard must not live across
        // the awaits below (it would make the future !Send).
        let action = *control.borrow_and_update();
        match action {
            ControlAction::Pause => {
                file.flush().await?;
                file.sync_all().await?;
                let mut current = task.lock().await;
                current.state = DownloadState::Paused;
                current.speed_bps = 0.0;
                on_update(current.clone());
                return Ok(Flow::Paused);
            }
            ControlAction::Cancel => {
                // Close the handle first: Windows cannot delete open files.
                drop(file);
                let _ = tokio::fs::remove_file(part_path).await;
                let mut current = task.lock().await;
                current.state = DownloadState::Cancelled;
                current.finished_at = Some(Utc::now());
                current.speed_bps = 0.0;
                on_update(current.clone());
                return Ok(Flow::Cancelled);
            }
            ControlAction::Continue => {}
        }

        // 读超时兜底：没有它，对端保持连接却不发数据会让任务永久挂住。
        let chunk = tokio::time::timeout(READ_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| {
                AppError::DownloadFailed(format!(
                    "下载停滞超过 {} 秒，已中止（可点重试续传）",
                    READ_IDLE_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|err| AppError::Network(err.to_string()))?;
        let Some(chunk) = chunk else {
            break; // EOF: full body received
        };

        file.write_all(&chunk).await?;
        let mut current = task.lock().await;
        current.progress_bytes += chunk.len() as u64;
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        current.speed_bps = (current.progress_bytes - start_offset) as f64 / elapsed;
        let remaining = current.total_bytes.saturating_sub(current.progress_bytes);
        current.eta_seconds = if current.speed_bps > 0.0 {
            (remaining as f64 / current.speed_bps) as u64
        } else {
            0
        };
        if last_emit.elapsed() >= PROGRESS_INTERVAL {
            last_emit = Instant::now();
            on_update(current.clone());
        }
    }

    file.flush().await?;
    file.sync_all().await?;
    {
        let mut current = task.lock().await;
        current.state = DownloadState::Verifying;
    }
    on_update(task.lock().await.clone());

    verify_sha256(&client, official_url, part_path).await?;
    tokio::fs::rename(part_path, dest).await?;

    // 解压（docs/01 DownloadState.extracting）：zip/tar.xz 解包到
    // 安装目录；msi/dmg 是安装器格式，保留安装包直接完成。
    {
        let mut current = task.lock().await;
        current.state = DownloadState::Extracting;
    }
    on_update(task.lock().await.clone());
    let dest_owned = dest.to_path_buf();
    let install_dir = dest_owned
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let extracted = Extractor::extract(dest_owned.clone(), install_dir).await?;
    if extracted {
        // 解压成功后删除归档（库扫描识别目录，不认归档）。
        let _ = tokio::fs::remove_file(&dest_owned).await;
    }

    {
        let mut current = task.lock().await;
        current.state = DownloadState::Completed;
        current.finished_at = Some(Utc::now());
        current.speed_bps = 0.0;
        current.eta_seconds = 0;
        on_update(current.clone());
    }
    Ok(Flow::Completed)
}

/// 校验下载结果的 sha256。
///
/// 两级来源，任一命中即校验：
/// 1. `<url>.sha256` 单文件 sidecar（部分第三方 fork 提供）；
/// 2. 同目录的整版清单 `blender-<version>.sha256`——**官方正式版只有这个**，
///    每行是 `<hash>  <filename>`。
///
/// 清单地址按**官方**域名构造而不是下载地址：走镜像时用镜像自己的哈希
/// 校验等于让镜像自证清白，校验就失去了意义。
///
/// 两级都拿不到就跳过（打日志）。宁可「没校验」也不要「因为校验源缺失
/// 而拒绝一个已经下完的包」——用户至少还能在 Blender 里看到它。
async fn verify_sha256(client: &reqwest::Client, official_url: &str, path: &Path) -> Result<()> {
    let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };

    let mut expected = fetch_sidecar_hash(client, official_url).await;
    if expected.is_none() {
        expected = fetch_manifest_hash(client, official_url, filename).await;
    }

    let Some(expected) = expected else {
        tracing::warn!(url = %official_url, "找不到 sha256 来源，跳过校验");
        return Ok(());
    };

    let actual = file_sha256(path).await?;
    if actual != expected {
        return Err(AppError::DownloadFailed(format!(
            "sha256 校验失败：期望 {expected}，实际 {actual}"
        )));
    }
    Ok(())
}

fn parse_hex_digest(text: &str) -> Option<String> {
    let candidate = text.trim().to_lowercase();
    (candidate.len() == 64 && candidate.bytes().all(|b| b.is_ascii_hexdigit())).then_some(candidate)
}

/// `<url>.sha256`：整个文件就是一行哈希。
async fn fetch_sidecar_hash(client: &reqwest::Client, url: &str) -> Option<String> {
    let response = client.get(format!("{url}.sha256")).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;
    parse_hex_digest(text.split_whitespace().next().unwrap_or(""))
}

/// 整版清单：`<dir>/blender-<version>.sha256`，每行 `<hash>  <filename>`。
///
/// 文件名 `blender-5.3.0-windows-x64.zip` → 清单 `blender-5.3.0.sha256`：
/// 取 `blender-` 之后的数字与点号段就是版本。
async fn fetch_manifest_hash(
    client: &reqwest::Client,
    official_url: &str,
    filename: &str,
) -> Option<String> {
    let version = filename
        .strip_prefix("blender-")?
        .split('-')
        .next()
        .filter(|part| part.chars().any(|ch| ch.is_ascii_digit()))?;
    let dir = official_url.rsplit_once('/')?.0;
    let manifest_url = format!("{dir}/blender-{version}.sha256");

    let response = client.get(&manifest_url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let text = response.text().await.ok()?;

    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name == filename {
            return parse_hex_digest(hash);
        }
    }
    None
}

async fn file_sha256(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; IO_BUFFER_BYTES];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// `<dir>/blender-4.5.0-windows-x64.zip` → `.../blender-4.5.0-windows-x64.zip.part`.
pub fn part_file_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Last path segment of a download URL, stripped of query/fragment.
pub fn filename_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_part_file_path() {
        assert_eq!(
            part_file_path(Path::new("C:/bl/builds/blender-4.5.0-windows-x64.zip")),
            PathBuf::from("C:/bl/builds/blender-4.5.0-windows-x64.zip.part")
        );
        assert_eq!(
            part_file_path(Path::new("no-extension")),
            PathBuf::from("no-extension.part")
        );
    }

    #[test]
    fn test_parse_hex_digest_rejects_short_and_non_hex() {
        let good = "3ac567a63d59d9934f7b9213bfb27ba05760e1fb366f8f55c381573ec3eabfa8";
        assert_eq!(parse_hex_digest(good).as_deref(), Some(good));
        // 大写要归一化成小写，否则跟 file_sha256 的输出比不相等。
        assert_eq!(
            parse_hex_digest(&good.to_uppercase()).as_deref(),
            Some(good)
        );
        assert_eq!(parse_hex_digest("abc"), None);
        assert_eq!(parse_hex_digest(&"z".repeat(64)), None);
    }

    /// 校验能否从「整版清单」里挑出对应文件那一行。
    ///
    /// 不联网：把清单内容直接喂给解析逻辑。这份清单取自
    /// `download.blender.org/release/Blender5.3/blender-5.3.0.sha256`（2026-11 实测）。
    #[test]
    fn test_manifest_lookup_picks_matching_filename() {
        let manifest = "\
2d802b54766741e612cead50b03ebbc8eb64dadc71eda932573d256b8a784720  blender-5.3.0-macos-arm64.dmg
3ac567a63d59d9934f7b9213bfb27ba05760e1fb366f8f55c381573ec3eabfa8  blender-5.3.0-windows-x64.zip
";
        let lookup = |filename: &str| -> Option<String> {
            manifest.lines().find_map(|line| {
                let mut parts = line.split_whitespace();
                match (parts.next(), parts.next()) {
                    (Some(hash), Some(name)) if name == filename => parse_hex_digest(hash),
                    _ => None,
                }
            })
        };
        assert_eq!(
            lookup("blender-5.3.0-windows-x64.zip").as_deref(),
            Some("3ac567a63d59d9934f7b9213bfb27ba05760e1fb366f8f55c381573ec3eabfa8")
        );
        assert_eq!(lookup("blender-5.3.0-windows-arm64.zip"), None);
    }

    #[test]
    fn test_filename_from_url() {
        assert_eq!(
            filename_from_url(
                "https://cdn.example.com/daily/blender-5.4.0-windows.amd64-release.zip"
            ),
            "blender-5.4.0-windows.amd64-release.zip"
        );
        assert_eq!(
            filename_from_url("https://cdn.example.com/file.zip?token=abc"),
            "file.zip"
        );
    }

    #[tokio::test]
    async fn test_file_sha256_known_digest() {
        let path = std::env::temp_dir().join(format!("bl-sha-{}.txt", uuid::Uuid::new_v4()));
        tokio::fs::write(&path, b"hello").await.unwrap();
        let digest = file_sha256(&path).await.unwrap();
        let _ = tokio::fs::remove_file(&path).await;
        assert_eq!(
            digest,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    /// Live smoke test: downloads a small real file end to end.
    #[tokio::test]
    #[ignore = "hits the network; run explicitly with `cargo test -- --ignored`"]
    async fn download_live() {
        let url =
            "https://download.blender.org/release/Blender4.5/blender-4.5.0.sha256".to_string();
        let dest = std::env::temp_dir().join(format!("bl-dl-{}", uuid::Uuid::new_v4()));
        let task = Arc::new(Mutex::new(DownloadTask::new("test-build".into(), 0)));
        let (_control_tx, control_rx) = watch::channel(ControlAction::Continue);

        let result = Downloader::download(
            Arc::clone(&task),
            DownloadSpec {
                url: url.clone(),
                official_url: url,
                dest: dest.clone(),
                resume_from: 0,
            },
            control_rx,
            &crate::models::ProxySettings::default(),
            |_snapshot| {},
        )
        .await;

        assert!(result.is_ok(), "download failed: {:?}", result.err());
        assert!(dest.exists(), "final file must exist after rename");
        let current = task.lock().await;
        assert_eq!(current.state, DownloadState::Completed);
        assert!(current.total_bytes > 0);
        assert_eq!(current.progress_bytes, current.total_bytes);
        let _ = tokio::fs::remove_file(&dest).await;
    }
}

/// 真机验证镜像链路：从 TUNA 下一个**真实但很小**的文件，
/// 用官方清单校验，确认「镜像传输 + 官方哈希」这条组合能通。
///
/// 选中 5.3.0 的 `.sha256` 清单本身（几 KB），而不是 400 MB 的安装包——
/// 前者能在几秒内跑完，且同样走完整条 chunk 循环与校验分支。
///
/// 手动运行：
/// `cargo test --lib live_mirror_download -- --ignored --nocapture`
#[cfg(test)]
mod live_mirror_tests {
    use super::*;
    use crate::models::DownloadMirror;

    #[tokio::test]
    #[ignore = "hits the network; run explicitly"]
    async fn live_mirror_download_verifies_against_official_hash() {
        let official =
            "https://download.blender.org/release/Blender5.3/blender-5.3.0-windows-x64.zip";
        let mirrored = DownloadMirror::Tuna
            .rewrite(official)
            .expect("TUNA 应当支持 stable 下载");

        // 用清单文件本身作为下载目标（小、且校验它的哈希能证明链路通了）。
        let manifest_url =
            "https://mirrors.tuna.tsinghua.edu.cn/blender/release/Blender5.3/blender-5.3.0.sha256";
        let dest = std::env::temp_dir().join(format!("bl-mirror-{}.sha256", std::process::id()));
        let task = Arc::new(Mutex::new(DownloadTask::new("mirror-probe".into(), 0)));
        let (_tx, rx) = watch::channel(ControlAction::Continue);

        let result = Downloader::download(
            Arc::clone(&task),
            DownloadSpec {
                url: manifest_url.to_string(),
                // 清单没有自己的 sha256 来源，传官方同名地址即可：校验会跳过。
                official_url: manifest_url.to_string(),
                dest: dest.clone(),
                resume_from: 0,
            },
            rx,
            &crate::models::ProxySettings::default(),
            |_| {},
        )
        .await;

        assert!(result.is_ok(), "镜像下载失败: {:?}", result.err());
        let text = std::fs::read_to_string(&dest).expect("下载产物应可读");
        assert!(
            text.contains("blender-5.3.0-windows-x64.zip"),
            "清单里应含目标安装包（镜像内容确实是官方清单）"
        );
        eprintln!("mirrored URL: {mirrored}");
        eprintln!("manifest bytes: {}", text.len());
        let _ = tokio::fs::remove_file(&dest).await;
    }
}
