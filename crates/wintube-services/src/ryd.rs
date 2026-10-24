use serde::{Deserialize, Serialize};
use tracing::debug;

const RYD_API_BASE: &str = "https://returnyoutubedislikeapi.com";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DislikeStats {
    pub id: String,
    pub date_created: Option<String>,
    pub likes: u64,
    pub dislikes: u64,
    pub rating: f64,
    pub view_count: u64,
    pub deleted: bool,
}

/// ReturnYouTubeDislike (RYD) API Client.
/// Fetches restored dislike counts and community ratings concurrently.
#[derive(Clone)]
pub struct ReturnYouTubeDislikeService {
    http: reqwest::Client,
}

impl ReturnYouTubeDislikeService {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Queries the RYD database for real dislike numbers.
    pub async fn get_votes(&self, video_id: &str) -> anyhow::Result<DislikeStats> {
        let url = format!("{RYD_API_BASE}/votes?videoId={video_id}");
        debug!("Querying ReturnYouTubeDislike for video {}", video_id);

        let response = self.http.get(&url).send().await?;
        let stats: DislikeStats = response.json().await?;
        Ok(stats)
    }
}
