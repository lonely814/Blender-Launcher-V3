use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub build_id: String,
    pub state: DownloadState,
    pub progress_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: f64,
    pub eta_seconds: u64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadState {
    Queued,
    Downloading,
    Extracting,
    Verifying,
    Completed,
    Failed,
    Cancelled,
    Paused,
}

impl DownloadTask {
    pub fn new(build_id: String, total_bytes: u64) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            build_id,
            state: DownloadState::Queued,
            progress_bytes: 0,
            total_bytes,
            speed_bps: 0.0,
            eta_seconds: 0,
            started_at: Utc::now(),
            finished_at: None,
            error: None,
        }
    }

    pub fn progress_percent(&self) -> f32 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.progress_bytes as f32 / self.total_bytes as f32) * 100.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_progress_calculation() {
        let mut task = DownloadTask::new("build-1".to_string(), 1000);
        task.progress_bytes = 500;
        assert_eq!(task.progress_percent(), 50.0);
    }
}
