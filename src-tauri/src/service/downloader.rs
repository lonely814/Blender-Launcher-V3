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
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
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

impl Downloader {
    /// Streams `url` into `<dest>.part`, then verifies it (sha256 sidecar
    /// when available) and renames it to `dest`.
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
        url: String,
        dest: PathBuf,
        resume_from: u64,
        control: watch::Receiver<ControlAction>,
        proxy: &crate::models::ProxySettings,
        mut on_update: impl FnMut(DownloadTask) + Send,
    ) -> Result<()> {
        let part_path = part_file_path(&dest);

        let flow = run_download(
            &task,
            &url,
            &dest,
            resume_from,
            control,
            proxy,
            &mut on_update,
        )
        .await;

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
    url: &str,
    dest: &Path,
    resume_from: u64,
    mut control: watch::Receiver<ControlAction>,
    proxy: &crate::models::ProxySettings,
    on_update: &mut (impl FnMut(DownloadTask) + Send),
) -> Result<Flow> {
    // part 路径由 dest 唯一决定，不再作为参数传入——多一个参数就多一处
    // 调用方可能传错的机会，而它们本来就不可能不一致。
    let part_path = part_file_path(dest);
    let part_path = part_path.as_path();
    // 与 scraper 共用同一个构造点，保证代理配置对爬取和下载同时生效。
    let client = crate::service::http::build_client(proxy, USER_AGENT, CONNECT_TIMEOUT)?;

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

        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| AppError::Network(err.to_string()))?
        else {
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

    verify_sha256_sidecar(&client, url, part_path).await?;
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

/// Verifies the downloaded file against the `<url>.sha256` sidecar.
/// Missing or unparsable sidecars are skipped (logged, not fatal).
async fn verify_sha256_sidecar(client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let response = match client.get(format!("{url}.sha256")).send().await {
        Ok(response) => response,
        Err(err) => {
            tracing::warn!(error = %err, "sha256 sidecar request failed, skipping verification");
            return Ok(());
        }
    };
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "no sha256 sidecar, skipping verification");
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    let expected = text.split_whitespace().next().unwrap_or("").to_lowercase();
    let is_hex = expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit());
    if !is_hex {
        tracing::warn!("unparsable sha256 sidecar, skipping verification");
        return Ok(());
    }

    let actual = file_sha256(path).await?;
    if actual != expected {
        return Err(AppError::DownloadFailed(format!(
            "sha256 mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
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
            url,
            dest.clone(),
            0,
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
