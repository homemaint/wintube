use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("network: {0}")]
    Network(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("sidecar missing: {0}")]
    SidecarMissing(String),
    #[error("yt-dlp failed: {0}")]
    YtDlp(String),
    #[error("ffmpeg failed: {0}")]
    Ffmpeg(String),
    #[error("internal: {0}")]
    Internal(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
