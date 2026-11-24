use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Network error: {0}")]
    Network(String),

    #[error("File system error: {0}")]
    FileSystem(#[from] std::io::Error),

    #[error("Build not found: {0}")]
    BuildNotFound(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Extract failed: {0}")]
    ExtractFailed(String),

    #[error("Launch failed: {0}")]
    LaunchFailed(String),

    #[error("Invalid settings: {0}")]
    InvalidSettings(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Internal error: {0}")]
    InternalError(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl From<AppError> for String {
    fn from(err: AppError) -> String {
        err.to_string()
    }
}
