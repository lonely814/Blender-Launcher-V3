use crate::models::{BuildBranch, BuildFork, DownloadState, DownloadTask};
use crate::service::downloader::{filename_from_url, ControlAction, DownloadSpec, Downloader};
use crate::state::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};
use tokio::sync::watch;

/// Pushes a fresh task state into the snapshot list (docs/01 §3.2).
fn update_snapshot(downloads: &Mutex<Vec<DownloadTask>>, task: &DownloadTask) {
    let mut list = downloads.lock().unwrap();
    match list.iter_mut().find(|slot| slot.id == task.id) {
        Some(slot) => *slot = task.clone(),
        None => list.push(task.clone()),
    }
}

/// Emits the docs/01 §4 events for a task update.
fn emit_update(app_handle: &tauri::AppHandle, task: &DownloadTask) {
    let _ = app_handle.emit("download_progress", task);
    match task.state {
        DownloadState::Completed => {
            let _ = app_handle.emit(
                "download_completed",
                serde_json::json!({ "buildId": task.build_id }),
            );
        }
        DownloadState::Failed => {
            let _ = app_handle.emit(
                "download_failed",
                serde_json::json!({
                    "buildId": task.build_id,
                    "error": task.error.clone().unwrap_or_default()
                }),
            );
        }
        _ => {}
    }
}

/// Shared spawn path for start/resume: registers the control channel,
/// seeds the snapshot, and runs the download loop in the background.
#[allow(clippy::too_many_arguments)]
fn spawn_download(
    downloads: Arc<Mutex<Vec<DownloadTask>>>,
    controls: Arc<Mutex<HashMap<String, watch::Sender<ControlAction>>>>,
    permits: Arc<tokio::sync::Semaphore>,
    app_handle: tauri::AppHandle,
    mut task: DownloadTask,
    url: String,
    // 同一构建的官方地址：校验清单只从官方域名取，避免镜像自证。
    official_url: String,
    dest: PathBuf,
    resume_from: u64,
    proxy: crate::models::ProxySettings,
) {
    let task_id = task.id.clone();
    // 排队中的任务统一显示 queued；拿到许可后进入 downloading。
    task.state = DownloadState::Queued;
    let arc_task = Arc::new(tokio::sync::Mutex::new(task));
    let part_path = crate::service::downloader::part_file_path(&dest);

    // Seed the snapshot before the loop starts reporting.
    let seed = arc_task
        .try_lock()
        .expect("fresh task is not locked")
        .clone();
    update_snapshot(&downloads, &seed);

    let (control_tx, mut control_rx) = watch::channel(ControlAction::Continue);
    controls.lock().unwrap().insert(task_id.clone(), control_tx);

    let update_downloads = Arc::clone(&downloads);
    let update_controls = Arc::clone(&controls);
    let update_app = app_handle.clone();
    let on_update = move |task: DownloadTask| {
        update_snapshot(&update_downloads, &task);
        emit_update(&update_app, &task);
        // Paused tasks keep no live channel; resume re-registers one.
        if matches!(
            task.state,
            DownloadState::Completed
                | DownloadState::Failed
                | DownloadState::Cancelled
                | DownloadState::Paused
        ) {
            update_controls.lock().unwrap().remove(&task.id);
        }
    };

    tokio::spawn(async move {
        // docs/01 §6.3: 下载队列并发上限由 semaphore 控制，排队期间不发请求。
        let Ok(_permit) = permits.acquire_owned().await else {
            return; // semaphore closed: shutting down
        };

        // 排队期间收到暂停/取消 → 按取消收尾（尚未开始传输，无从断点保留）。
        if *control_rx.borrow_and_update() != ControlAction::Continue {
            let _ = tokio::fs::remove_file(&part_path).await;
            let mut current = arc_task.lock().await;
            current.state = DownloadState::Cancelled;
            current.finished_at = Some(chrono::Utc::now());
            on_update(current.clone());
            return;
        }

        if let Err(err) = Downloader::download(
            arc_task,
            DownloadSpec {
                url,
                official_url,
                dest,
                resume_from,
            },
            control_rx,
            &proxy,
            on_update,
        )
        .await
        {
            tracing::warn!(task_id = %task_id, error = %err, "download ended with error");
        }
    });
}

/// 构建所属的类别目录（与原版 build_library_folders 一致）。
fn category_dir(fork: BuildFork, branch: BuildBranch) -> &'static str {
    match (fork, branch) {
        (BuildFork::Bforartists, _) => "bforartists",
        (BuildFork::Upbge, BuildBranch::Stable) => "upbge-stable",
        (BuildFork::Upbge, _) => "upbge-weekly",
        (BuildFork::Official, BuildBranch::Stable) => "stable",
        (BuildFork::Official, BuildBranch::Daily) => "daily",
        (BuildFork::Official, BuildBranch::Experimental) => "experimental",
        (BuildFork::Official, BuildBranch::Patch) => "patch",
    }
}

/// 实际用于下载的地址：配了镜像且镜像有这一版就用镜像。
///
/// 返回 `(下载地址, 是否走了镜像)`。文件名在两个站上相同，所以
/// 落盘路径始终按**官方**地址算——换镜像不该改变文件落在哪，
/// 否则同一个构建用镜像下一半再切回来就续不上了。
fn download_url(
    build: &crate::models::Build,
    mirror: Option<crate::models::DownloadMirror>,
) -> String {
    mirror
        .and_then(|mirror| mirror.rewrite(&build.url))
        .unwrap_or_else(|| build.url.clone())
}

/// 开始下载 (docs/01 §3.2: 立即返回 task_id，进度走事件)
#[tauri::command]
pub async fn start_download(
    build_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let (url, official_url, filename, install_dir, category, proxy) = {
        let builds = state.builds.lock().unwrap();
        let settings = state.settings.lock().unwrap();
        match builds.iter().find(|b| b.id == build_id) {
            Some(build) => (
                download_url(build, settings.download_mirror),
                build.url.clone(),
                // 落盘名固定取**官方**地址：换镜像不该换路径，
                // 否则用镜像下一半再切回官方就续不上（part 文件对不上）。
                filename_from_url(&build.url),
                settings.install_dir.clone(),
                category_dir(build.fork, build.branch),
                settings.proxy.clone(),
            ),
            None => return Err(format!("Build not found: {build_id}")),
        }
    };

    let task = DownloadTask::new(build_id, 0);
    let task_id = task.id.clone();
    // 原版结构：<library>/<类别>/<归档>，解压到 <类别>/ 下。
    let dest = install_dir.join(category).join(filename);
    spawn_download(
        Arc::clone(&state.downloads),
        Arc::clone(&state.download_controls),
        Arc::clone(&state.download_permits),
        app_handle,
        task,
        url,
        official_url,
        dest,
        0,
        proxy,
    );
    Ok(task_id)
}

#[tauri::command]
pub async fn pause_download(task_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let sender = state
        .download_controls
        .lock()
        .unwrap()
        .get(&task_id)
        .cloned();
    match sender {
        Some(control) => {
            let _ = control.send(ControlAction::Pause);
            Ok(())
        }
        None => Err(format!("Download not running: {task_id}")),
    }
}

/// 断点续传：从已保存的 .part 字节继续 (docs/00 §1.2)
#[tauri::command]
pub async fn resume_download(
    task_id: String,
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let (task, url, official_url, filename, install_dir, category, proxy) = {
        let downloads = state.downloads.lock().unwrap();
        let builds = state.builds.lock().unwrap();
        let settings = state.settings.lock().unwrap();

        let task = downloads
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
            .ok_or_else(|| format!("Download not found: {task_id}"))?;
        if task.state != DownloadState::Paused {
            return Err(format!(
                "Download {task_id} is not paused (state: {:?})",
                task.state
            ));
        }
        let build = builds
            .iter()
            .find(|b| b.id == task.build_id)
            .ok_or_else(|| format!("Build not found: {}", task.build_id))?;
        (
            task,
            download_url(build, settings.download_mirror),
            build.url.clone(),
            filename_from_url(&build.url),
            settings.install_dir.clone(),
            category_dir(build.fork, build.branch),
            settings.proxy.clone(),
        )
    };

    let resume_from = task.progress_bytes;
    // 与 start_download 一致：落盘名永远取官方地址末段，镜像只换传输源。
    let dest = install_dir.join(category).join(filename);
    spawn_download(
        Arc::clone(&state.downloads),
        Arc::clone(&state.download_controls),
        Arc::clone(&state.download_permits),
        app_handle,
        task,
        url,
        official_url,
        dest,
        resume_from,
        proxy,
    );
    Ok(())
}

#[tauri::command]
pub async fn cancel_download(task_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let sender = state
        .download_controls
        .lock()
        .unwrap()
        .get(&task_id)
        .cloned();
    if let Some(control) = sender {
        // Running: the download loop deletes the .part file and emits.
        let _ = control.send(ControlAction::Cancel);
        return Ok(());
    }

    // Paused (no live loop): mark cancelled and clean up here.
    let (task, part_path) = {
        let downloads = state.downloads.lock().unwrap();
        let builds = state.builds.lock().unwrap();
        let settings = state.settings.lock().unwrap();

        let task = downloads
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
            .ok_or_else(|| format!("Download not found: {task_id}"))?;
        let part_path = builds.iter().find(|b| b.id == task.build_id).map(|build| {
            crate::service::downloader::part_file_path(
                &settings
                    .install_dir
                    .join(category_dir(build.fork, build.branch))
                    .join(filename_from_url(&build.url)),
            )
        });
        (task, part_path)
    };

    if let Some(part) = part_path {
        let _ = tokio::fs::remove_file(part).await;
    }
    let mut cancelled = task;
    cancelled.state = DownloadState::Cancelled;
    cancelled.finished_at = Some(chrono::Utc::now());
    update_snapshot(&state.downloads, &cancelled);
    Ok(())
}

#[tauri::command]
pub async fn get_active_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    let downloads = state.downloads.lock().unwrap();
    Ok(downloads.clone())
}
