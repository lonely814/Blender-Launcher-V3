use crate::db::Db;
use crate::models::{Build, DownloadTask, Settings};
use crate::service::downloader::ControlAction;
use crate::service::launcher::InstanceEntry;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{watch, Semaphore};

/// docs/01 §6.3: 下载队列默认并发数（settings.max_concurrent_downloads 可覆盖）。
const DEFAULT_MAX_CONCURRENT_DOWNLOADS: usize = 3;

pub struct AppState {
    /// Cached build list, refreshed by the scraper (3h TTL, docs/01 §6.1).
    pub builds: Arc<Mutex<Vec<Build>>>,
    /// Time of the last scrape attempt (success or failure), for cache TTL.
    pub builds_last_refresh: Arc<Mutex<Option<DateTime<Utc>>>>,
    /// Whether a scrape is currently in flight, to avoid duplicate requests.
    pub builds_refreshing: Arc<Mutex<bool>>,
    /// Active download tasks, keyed by DownloadTask.id.
    pub downloads: Arc<Mutex<Vec<DownloadTask>>>,
    /// Live control channels for running downloads, keyed by DownloadTask.id.
    /// Removed when a task reaches a terminal (or paused) state.
    pub download_controls: Arc<Mutex<HashMap<String, watch::Sender<ControlAction>>>>,
    /// Download queue: at most N tasks run at once (docs/01 §6.3).
    pub download_permits: Arc<Semaphore>,
    /// Running Blender instances, keyed by LaunchedInstance.id (= PID).
    pub running_instances: Arc<Mutex<HashMap<String, InstanceEntry>>>,
    /// 上次扫描发现缺少可执行文件的目录名（docs/00 §3.4 损坏检测）。
    pub library_corrupt_dirs: Arc<Mutex<Vec<String>>>,
    /// SQLite persistence for the builds cache; None when the DB failed to
    /// open (app keeps working in memory-only mode).
    pub db: Option<Arc<Db>>,
    pub settings: Arc<Mutex<Settings>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            builds: Arc::new(Mutex::new(Vec::new())),
            builds_last_refresh: Arc::new(Mutex::new(None)),
            builds_refreshing: Arc::new(Mutex::new(false)),
            downloads: Arc::new(Mutex::new(Vec::new())),
            download_controls: Arc::new(Mutex::new(HashMap::new())),
            download_permits: Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_DOWNLOADS)),
            running_instances: Arc::new(Mutex::new(HashMap::new())),
            library_corrupt_dirs: Arc::new(Mutex::new(Vec::new())),
            db: None,
            settings: Arc::new(Mutex::new(Settings::default())),
        }
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_settings(settings: Settings) -> Self {
        Self {
            download_permits: Arc::new(Semaphore::new(
                settings.max_concurrent_downloads.max(1) as usize
            )),
            settings: Arc::new(Mutex::new(settings)),
            ..Default::default()
        }
    }

    /// 把当前 builds 列表落库（后台线程，失败仅记日志）。
    pub fn persist_builds(&self) {
        let Some(db) = self.db.as_ref().map(Arc::clone) else {
            return;
        };
        let builds = self.builds.lock().unwrap().clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || db.save_builds(&builds)).await;
            if let Ok(Err(err)) = result {
                tracing::warn!(error = %err, "failed to persist builds cache");
            }
        });
    }
}
