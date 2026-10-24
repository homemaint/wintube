use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

const DEARROW_API_BASE: &str = "https://dearrow.ajay.app/api";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeArrowTitle {
    pub title: String,
    pub original: bool,
    pub votes: i64,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeArrowThumbnail {
    pub timestamp: Option<f64>,
    pub original: bool,
    pub votes: i64,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeArrowBranding {
    pub titles: Vec<DeArrowTitle>,
    pub thumbnails: Vec<DeArrowThumbnail>,
    pub random_time: Option<f64>,
}

/// DeArrow client for crowdsourced anti-clickbait titles and honest thumbnails.
#[derive(Clone)]
pub struct DeArrowService {
    http: reqwest::Client,
}

impl DeArrowService {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Fetches crowdsourced title and non-clickbait thumbnail timestamp.
    pub async fn get_branding(&self, video_id: &str) -> anyhow::Result<Option<DeArrowBranding>> {
        let url = format!("{DEARROW_API_BASE}/v1/branding?videoID={video_id}");
        debug!("Querying DeArrow for video {}", video_id);

        let response = self.http.get(&url).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let branding: DeArrowBranding = response.json().await?;
        Ok(Some(branding))
    }

    /// Returns the cleanest title: prefers highest-voted crowdsourced title, falls back to original.
    pub fn pick_clean_title<'a>(&self, original_title: &'a str, branding: Option<&'a DeArrowBranding>) -> &'a str {
        if let Some(b) = branding {
            if let Some(first) = b.titles.first() {
                if !first.title.is_empty() {
                    return &first.title;
                }
            }
        }
        original_title
    }
}
