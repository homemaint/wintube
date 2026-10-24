use std::sync::Arc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, error, info, warn};

const SPONSORBLOCK_API_BASE: &str = "https://sponsor.ajay.app/api";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SponsorCategory {
    Sponsor,
    SelfPromo,
    Interaction,
    Intro,
    Outro,
    Preview,
    MusicOfftopic,
    Filler,
    Other(String),
}

impl SponsorCategory {
    pub fn as_str(&self) -> &str {
        match self {
            SponsorCategory::Sponsor => "sponsor",
            SponsorCategory::SelfPromo => "selfpromo",
            SponsorCategory::Interaction => "interaction",
            SponsorCategory::Intro => "intro",
            SponsorCategory::Outro => "outro",
            SponsorCategory::Preview => "preview",
            SponsorCategory::MusicOfftopic => "music_offtopic",
            SponsorCategory::Filler => "filler",
            SponsorCategory::Other(s) => s.as_str(),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "sponsor" => SponsorCategory::Sponsor,
            "selfpromo" => SponsorCategory::SelfPromo,
            "interaction" => SponsorCategory::Interaction,
            "intro" => SponsorCategory::Intro,
            "outro" => SponsorCategory::Outro,
            "preview" => SponsorCategory::Preview,
            "music_offtopic" => SponsorCategory::MusicOfftopic,
            "filler" => SponsorCategory::Filler,
            other => SponsorCategory::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkipAction {
    AutoSkip,
    ShowPrompt,
    MuteAudio,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SponsorSegment {
    pub uuid: String,
    pub category: SponsorCategory,
    pub action_type: String,
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Debug, Deserialize)]
struct RawSegmentResponse {
    #[serde(rename = "videoID")]
    video_id: String,
    segments: Vec<RawSegmentItem>,
}

#[derive(Debug, Deserialize)]
struct RawSegmentItem {
    #[serde(rename = "UUID")]
    uuid: String,
    category: String,
    #[serde(rename = "actionType")]
    action_type: String,
    #[serde(rename = "segment")]
    segment: (f64, f64),
}

/// SponsorBlock service with privacy-preserving SHA-256 prefix lookup.
/// Mirrors `com.liskovsoft.smartyoutubetv2.common.app.presenters.PlaybackPresenter.ContentBlockController`
#[derive(Clone)]
pub struct SponsorBlockService {
    http: reqwest::Client,
}

impl SponsorBlockService {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Fetches skip segments for a video using a 4-byte (8 hex character) SHA-256 hash prefix
    /// to preserve user privacy (the SponsorBlock server does not learn the exact video watched).
    pub async fn fetch_segments(&self, video_id: &str) -> anyhow::Result<Vec<SponsorSegment>> {
        let mut hasher = Sha256::new();
        hasher.update(video_id.as_bytes());
        let hash_result = hex::encode(hasher.finalize());
        let hash_prefix = &hash_result[..8];

        let categories = r#"["sponsor","selfpromo","interaction","intro","outro","preview","music_offtopic"]"#;
        let url = format!("{SPONSORBLOCK_API_BASE}/skipSegments/{hash_prefix}?categories={categories}");

        debug!("Fetching SponsorBlock hash prefix: {}", hash_prefix);

        let response = self.http.get(&url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }

        let payload: Vec<RawSegmentResponse> = response.json().await?;

        for item in payload {
            if item.video_id == video_id {
                let segments = item
                    .segments
                    .into_iter()
                    .map(|s| SponsorSegment {
                        uuid: s.uuid,
                        category: SponsorCategory::from_str(&s.category),
                        action_type: s.action_type,
                        start_time: s.segment.0,
                        end_time: s.segment.1,
                    })
                    .collect();
                return Ok(segments);
            }
        }

        Ok(Vec::new())
    }

    /// Checks if a given timestamp falls inside any active sponsor segment.
    pub fn find_active_segment<'a>(&self, current_time: f64, segments: &'a [SponsorSegment]) -> Option<&'a SponsorSegment> {
        segments.iter().find(|seg| current_time >= seg.start_time && current_time < seg.end_time)
    }
}
