//! Video / format / comment models shared by UI and backend.
//! Inspired by SmartTube's MediaServiceCore interfaces (MediaItemFormatInfo,
//! MediaFormat) but reshaped for a desktop YouTube-like canvas.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub channel_id: Option<String>,
    pub channel_avatar: Option<String>,
    pub description: Option<String>,
    pub duration: Option<u64>, // seconds
    pub view_count: Option<u64>,
    pub like_count: Option<u64>,
    pub thumbnail: String,
    pub thumbnail_hq: Option<String>,
    pub upload_date: Option<String>,
    pub is_live: bool,
    pub is_upcoming: bool,
    pub tags: Vec<String>,
    pub subtitles: Vec<Subtitle>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoCard {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub views: Option<String>,
    pub age: Option<String>,
    pub duration: Option<u64>,
    pub thumbnail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subtitle {
    pub lang: String,
    pub label: String,
    pub ext: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Format {
    pub format_id: String,
    pub url: String,
    pub ext: String, // mp4/webm/m4a/mp3
    pub codec_video: Option<String>, // avc1, vp9, av01
    pub codec_audio: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub tbr: Option<f64>,
    pub filesize: Option<u64>,
    pub filesize_approx: Option<u64>,
    pub container: Option<String>,
    pub is_video_only: bool,
    pub is_audio_only: bool,
    pub is_progressive: bool,
    pub is_hdr: bool,
    pub http_headers: Option<serde_json::Value>,
    pub protocol: Option<String>,
    pub quality_label: Option<String>, // "2160p"
}

impl Format {
    pub fn prefs_at(&self) -> Option<u64> {
        self.filesize.or(self.filesize_approx)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayInfo {
    pub video: Video,
    pub formats: Vec<Format>,
    pub best_progressive_url: Option<String>,
    pub dash_url: Option<String>,
    pub hls_url: Option<String>,
    pub storyboard_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub author_avatar: Option<String>,
    pub text: String,
    pub likes: Option<u64>,
    pub published: Option<String>,
    pub replies: Vec<Comment>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub subscribers: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Downloading,
    Merging,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadJob {
    pub id: Uuid,
    pub video_id: String,
    pub title: String,
    pub quality_label: String, // "1080p" etc
    pub format_string: String,
    pub status: JobStatus,
    pub progress: f64,     // 0..=1
    pub speed_bps: Option<u64>,
    pub eta_secs: Option<u64>,
    pub output_path: Option<String>,
    pub error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub download_dir: String,
    pub default_quality: String, // "auto" | "4320p" | ...
    pub default_codec: String,   // "av1" | "vp9" | "avc" | "auto"
    pub auto_sponsorblock: bool,
    pub theme: String, // "dark" | "light" | "system"
    pub hw_accel: bool,
    pub check_updates: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        let dir = dirs::download_dir()
            .map(|p| p.join("Wintube").to_string_lossy().to_string())
            .unwrap_or_else(|| "Downloads/Wintube".to_string());
        Self {
            download_dir: dir,
            default_quality: "auto".into(),
            default_codec: "auto".into(),
            auto_sponsorblock: true,
            theme: "dark".into(),
            hw_accel: true,
            check_updates: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub thumbnail: String,
    pub position: u64, // seconds
    pub duration: Option<u64>,
    pub watched_at: chrono::DateTime<chrono::Utc>,
}
