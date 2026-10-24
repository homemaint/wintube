use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnertubeContext {
    pub client: ClientContext,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<RequestContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientContext {
    pub client_name: String,
    pub client_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utc_offset_minutes: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visitor_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf_of_user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_safety_mode: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_ssl: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal_experiment_flags: Option<Vec<String>>,
}

// ----------------- Video / Stream Info Models -----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub playability_status: PlayabilityStatus,
    pub video_details: Option<VideoDetails>,
    pub streaming_data: Option<StreamingData>,
    pub captions: Option<CaptionsContainer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayabilityStatus {
    pub status: String,
    pub reason: Option<String>,
    pub playable_in_embed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDetails {
    pub video_id: String,
    pub title: String,
    pub length_seconds: String,
    pub channel_id: String,
    pub author: String,
    pub short_description: Option<String>,
    pub view_count: Option<String>,
    pub is_live_content: Option<bool>,
    pub thumbnail: Option<ThumbnailContainer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailContainer {
    pub thumbnails: Vec<ThumbnailItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThumbnailItem {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingData {
    #[serde(rename = "expiresInSeconds")]
    pub expires_in_seconds: Option<String>,
    #[serde(default)]
    pub formats: Vec<FormatStream>,
    #[serde(default)]
    pub adaptive_formats: Vec<FormatStream>,
    pub hls_manifest_url: Option<String>,
    pub dash_manifest_url: Option<String>,
}

// ----------------- Feed, Search & Next Models -----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoCardItem {
    pub video_id: String,
    pub title: String,
    pub channel_name: String,
    pub channel_id: Option<String>,
    pub view_count_text: String,
    pub published_time_text: String,
    pub length_text: String,
    pub thumbnail_url: Option<String>,
    pub is_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentItem {
    pub author: String,
    pub text: String,
    pub published_time: String,
    pub likes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterItem {
    pub title: String,
    pub start_time_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VideoNextDetails {
    pub related_videos: Vec<VideoCardItem>,
    pub comments: Vec<CommentItem>,
    pub chapters: Vec<ChapterItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatStream {
    pub itag: u32,
    pub url: Option<String>,
    pub mime_type: String,
    pub bitrate: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub quality: Option<String>,
    pub quality_label: Option<String>,
    pub fps: Option<u32>,
    pub audio_quality: Option<String>,
    pub approx_duration_ms: Option<String>,
    pub audio_sample_rate: Option<String>,
    pub audio_channels: Option<u32>,
    pub average_bitrate: Option<u64>,
    pub content_length: Option<String>,
    pub init_range: Option<ByteRange>,
    pub index_range: Option<ByteRange>,
    pub signature_cipher: Option<String>,
    pub cipher: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ByteRange {
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionsContainer {
    pub player_captions_tracklist_renderer: Option<PlayerCaptionsTracklistRenderer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCaptionsTracklistRenderer {
    pub caption_tracks: Option<Vec<CaptionTrack>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionTrack {
    pub base_url: String,
    pub name: TextRunContainer,
    pub vss_id: String,
    pub language_code: String,
    pub kind: Option<String>,
    pub is_translatable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRunContainer {
    pub runs: Option<Vec<TextRun>>,
    pub simple_text: Option<String>,
}

impl TextRunContainer {
    pub fn as_text(&self) -> String {
        if let Some(ref text) = self.simple_text {
            return text.clone();
        }
        if let Some(ref runs) = self.runs {
            return runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>().join("");
        }
        String::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
}

// ----------------- Normalized Domain Models for WinTube -----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoMetadata {
    pub video_id: String,
    pub title: String,
    pub author: String,
    pub channel_id: String,
    pub duration_seconds: u64,
    pub view_count: u64,
    pub description: String,
    pub thumbnails: Vec<ThumbnailItem>,
    pub is_live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedStream {
    pub itag: u32,
    pub url: String,
    pub mime_type: String,
    pub codecs: String,
    pub quality_label: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
    pub bitrate: u64,
    pub is_audio: bool,
    pub is_video: bool,
    pub audio_sample_rate: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoPlaybackInfo {
    pub metadata: VideoMetadata,
    pub video_streams: Vec<ResolvedStream>,
    pub audio_streams: Vec<ResolvedStream>,
    pub hls_manifest_url: Option<String>,
    pub dash_manifest_url: Option<String>,
}
