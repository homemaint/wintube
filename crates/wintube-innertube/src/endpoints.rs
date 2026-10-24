use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT, REFERER};
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::client::AppClient;
use crate::cipher::CipherEngine;
use crate::models::{
    PlayerResponse, ResolvedStream, VideoCardItem, VideoMetadata, VideoNextDetails, VideoPlaybackInfo,
};

const INNERTUBE_API_BASE: &str = "https://www.youtube.com/youtubei/v1";

#[derive(thiserror::Error, Debug)]
pub enum InnertubeError {
    #[error("HTTP request error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("Serialization / JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("API Playability error: {0}")]
    Unplayable(String),
    #[error("Stream not found or cipher deciphering failed")]
    StreamNotFound,
    #[error("General error: {0}")]
    Custom(String),
}

/// Core asynchronous Innertube Client.
/// Handles multi-client context spoofing (`TVHTML5`, `ANDROID_VR`, `IOS`, `WEB`),
/// visitor data negotiation, and direct stream extraction.
#[derive(Clone)]
pub struct InnertubeClient {
    http: reqwest::Client,
    cipher: Arc<CipherEngine>,
    visitor_data: Arc<tokio::sync::RwLock<Option<String>>>,
    po_token: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl InnertubeClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .tcp_nodelay(true)
            .build()
            .expect("Failed to initialize reqwest client");

        Self {
            http,
            cipher: Arc::new(CipherEngine::new()),
            visitor_data: Arc::new(tokio::sync::RwLock::new(None)),
            po_token: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Sets or updates the Proof-of-Origin (poToken) for botguard bypass
    pub async fn set_po_token(&self, token: String) {
        let mut po = self.po_token.write().await;
        *po = Some(token);
    }

    /// Builds the JSON context payload for a specific client profile
    async fn build_context_json(&self, client: AppClient) -> Value {
        let visitor_data = self.visitor_data.read().await.clone();

        json!({
            "client": {
                "clientName": client.client_name(),
                "clientVersion": client.client_version(),
                "hl": "en",
                "gl": "US",
                "utcOffsetMinutes": 0,
                "visitorData": visitor_data,
            },
            "user": {
                "lockedSafetyMode": false
            },
            "request": {
                "useSsl": true
            }
        })
    }

    /// Primary video player endpoint (`/player`).
    /// Tries `ANDROID_VR` (high success rate, unthrottled) and automatically
    /// cascades to `TVHTML5` and `WEB` if needed.
    pub async fn get_video_playback_info(&self, video_id: &str) -> Result<VideoPlaybackInfo, InnertubeError> {
        let clients_to_try = [
            AppClient::AndroidVr,
            AppClient::TvHtml5,
            AppClient::Web,
            AppClient::Ios,
        ];

        let mut last_err = None;

        for &client in &clients_to_try {
            debug!("Attempting /player extraction using client {:?}", client);
            match self.fetch_player_with_client(video_id, client).await {
                Ok(info) => return Ok(info),
                Err(err) => {
                    warn!("Client {:?} failed for video {}: {:?}", client, video_id, err);
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| InnertubeError::StreamNotFound))
    }

    async fn fetch_player_with_client(
        &self,
        video_id: &str,
        client: AppClient,
    ) -> Result<VideoPlaybackInfo, InnertubeError> {
        let url = format!("{INNERTUBE_API_BASE}/player?prettyPrint=false");
        let context = self.build_context_json(client).await;

        let mut body = json!({
            "context": context,
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });

        // Pass poToken if present
        if let Some(ref po) = *self.po_token.read().await {
            body["serviceIntegrityDimensions"] = json!({
                "poToken": po
            });
        }

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(client.user_agent()));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(referer) = client.referer() {
            headers.insert(REFERER, HeaderValue::from_static(referer));
        }

        let response = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let text = response.text().await?;
        debug!("Player response length for {}: {} bytes", video_id, text.len());

        let player_res: PlayerResponse = match serde_json::from_str(&text) {
            Ok(p) => p,
            Err(e) => {
                debug!("Direct PlayerResponse struct deserialization failed for {}: {}. Trying resilient Value parser.", video_id, e);
                let v: Value = serde_json::from_str(&text)?;
                return self.parse_player_from_value(video_id, &v);
            }
        };

        // Check Playability
        if player_res.playability_status.status != "OK" {
            let reason = player_res
                .playability_status
                .reason
                .unwrap_or_else(|| player_res.playability_status.status.clone());
            return Err(InnertubeError::Unplayable(reason));
        }

        let details = player_res.video_details.ok_or(InnertubeError::StreamNotFound)?;
        let streaming = player_res.streaming_data.ok_or(InnertubeError::StreamNotFound)?;

        let mut video_streams = Vec::new();
        let mut audio_streams = Vec::new();

        for format in streaming.adaptive_formats.iter().chain(streaming.formats.iter()) {
            let resolved_url = if let Some(ref u) = format.url {
                Some(u.clone())
            } else if let Some(ref cipher_str) = format.signature_cipher.as_ref().or(format.cipher.as_ref()) {
                self.cipher.decrypt_signature_cipher(cipher_str)
            } else {
                None
            };

            if let Some(stream_url) = resolved_url {
                let is_audio = format.mime_type.starts_with("audio/");
                let is_video = format.mime_type.starts_with("video/");

                let stream = ResolvedStream {
                    itag: format.itag,
                    url: stream_url,
                    mime_type: format.mime_type.clone(),
                    codecs: extract_codecs(&format.mime_type),
                    quality_label: format.quality_label.clone(),
                    width: format.width,
                    height: format.height,
                    fps: format.fps,
                    bitrate: format.bitrate,
                    is_audio,
                    is_video,
                    audio_sample_rate: format.audio_sample_rate.clone(),
                };

                if is_audio {
                    audio_streams.push(stream);
                } else if is_video {
                    video_streams.push(stream);
                }
            }
        }

        let metadata = VideoMetadata {
            video_id: details.video_id,
            title: details.title,
            author: details.author,
            channel_id: details.channel_id,
            duration_seconds: details.length_seconds.parse().unwrap_or(0),
            view_count: details.view_count.as_deref().unwrap_or("0").parse().unwrap_or(0),
            description: details.short_description.unwrap_or_default(),
            thumbnails: details.thumbnail.map(|t| t.thumbnails).unwrap_or_default(),
            is_live: details.is_live_content.unwrap_or(false),
        };

        Ok(VideoPlaybackInfo {
            metadata,
            video_streams,
            audio_streams,
            hls_manifest_url: streaming.hls_manifest_url,
            dash_manifest_url: streaming.dash_manifest_url,
        })
    }

    /// Resilient JSON Value fallback parser for `/player` responses when YouTube
    /// schema contains unexpected extra or missing fields.
    fn parse_player_from_value(&self, video_id: &str, v: &Value) -> Result<VideoPlaybackInfo, InnertubeError> {
        let status = v.pointer("/playabilityStatus/status")
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");
        if status != "OK" {
            let reason = v.pointer("/playabilityStatus/reason")
                .and_then(|r| r.as_str())
                .unwrap_or(status);
            return Err(InnertubeError::Unplayable(reason.to_string()));
        }

        let title = v.pointer("/videoDetails/title").and_then(|s| s.as_str()).unwrap_or("YouTube Video").to_string();
        let author = v.pointer("/videoDetails/author").and_then(|s| s.as_str()).unwrap_or("Creator").to_string();
        let channel_id = v.pointer("/videoDetails/channelId").and_then(|s| s.as_str()).unwrap_or("").to_string();
        let duration_seconds = v.pointer("/videoDetails/lengthSeconds").and_then(|s| s.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0);
        let view_count = v.pointer("/videoDetails/viewCount").and_then(|s| s.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0);
        let short_description = v.pointer("/videoDetails/shortDescription").and_then(|s| s.as_str()).unwrap_or("").to_string();

        let metadata = VideoMetadata {
            video_id: video_id.to_string(),
            title,
            author,
            channel_id,
            duration_seconds,
            view_count,
            description: short_description,
            thumbnails: Vec::new(),
            is_live: false,
        };

        let mut video_streams = Vec::new();
        let mut audio_streams = Vec::new();

        let empty_vec = Vec::new();
        let adaptive = v.pointer("/streamingData/adaptiveFormats").and_then(|a| a.as_array()).unwrap_or(&empty_vec);
        let progressive = v.pointer("/streamingData/formats").and_then(|a| a.as_array()).unwrap_or(&empty_vec);

        for format in adaptive.iter().chain(progressive.iter()) {
            let itag = format.get("itag").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let mime_type = format.get("mimeType").and_then(|m| m.as_str()).unwrap_or("").to_string();
            let bitrate = format.get("bitrate").and_then(|b| b.as_u64()).unwrap_or(0);

            let resolved_url = if let Some(u) = format.get("url").and_then(|u| u.as_str()) {
                Some(u.to_string())
            } else if let Some(cipher_str) = format.get("signatureCipher").or_else(|| format.get("cipher")).and_then(|c| c.as_str()) {
                self.cipher.decrypt_signature_cipher(cipher_str)
            } else {
                None
            };

            if let Some(stream_url) = resolved_url {
                let is_audio = mime_type.starts_with("audio/");
                let is_video = mime_type.starts_with("video/");
                let width = format.get("width").and_then(|w| w.as_u64()).map(|w| w as u32);
                let height = format.get("height").and_then(|h| h.as_u64()).map(|h| h as u32);
                let fps = format.get("fps").and_then(|f| f.as_u64()).map(|f| f as u32);
                let quality_label = format.get("qualityLabel").and_then(|q| q.as_str()).map(|q| q.to_string());
                let audio_sample_rate = format.get("audioSampleRate").and_then(|a| a.as_str()).map(|a| a.to_string());

                let stream = ResolvedStream {
                    itag,
                    url: stream_url,
                    mime_type: mime_type.clone(),
                    codecs: extract_codecs(&mime_type),
                    quality_label,
                    width,
                    height,
                    fps,
                    bitrate,
                    is_audio,
                    is_video,
                    audio_sample_rate,
                };

                if is_audio {
                    audio_streams.push(stream);
                } else if is_video {
                    video_streams.push(stream);
                }
            }
        }

        if video_streams.is_empty() && audio_streams.is_empty() {
            return Err(InnertubeError::StreamNotFound);
        }

        let hls_manifest_url = v.pointer("/streamingData/hlsManifestUrl").and_then(|u| u.as_str()).map(|s| s.to_string());
        let dash_manifest_url = v.pointer("/streamingData/dashManifestUrl").and_then(|u| u.as_str()).map(|s| s.to_string());

        Ok(VideoPlaybackInfo {
            metadata,
            video_streams,
            audio_streams,
            hls_manifest_url,
            dash_manifest_url,
        })
    }

    /// Search queries with filters and pagination continuation tokens.
    pub async fn search(&self, query: &str) -> Result<Value, InnertubeError> {
        let url = format!("{INNERTUBE_API_BASE}/search?prettyPrint=false");
        let context = self.build_context_json(AppClient::Web).await;

        let body = json!({
            "context": context,
            "query": query,
        });

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(AppClient::Web.user_agent()));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let json_val = response.json::<Value>().await?;
        Ok(json_val)
    }

    /// Browse endpoint for Home, Trending, Subscriptions, and Channels.
    pub async fn browse(&self, browse_id: &str) -> Result<Value, InnertubeError> {
        let url = format!("{INNERTUBE_API_BASE}/browse?prettyPrint=false");
        let context = self.build_context_json(AppClient::Web).await;

        let body = json!({
            "context": context,
            "browseId": browse_id,
        });

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(AppClient::Web.user_agent()));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let json_val = response.json::<Value>().await?;
        Ok(json_val)
    }

    /// High-level Search returning strongly-typed `VideoCardItem` results.
    pub async fn search_videos(&self, query: &str) -> Result<Vec<VideoCardItem>, InnertubeError> {
        let val = self.search(query).await?;
        let mut results = Vec::new();
        extract_video_renderers(&val, &mut results);
        Ok(results)
    }

    /// High-level Trending / Home feed returning strongly-typed `VideoCardItem` cards.
    pub async fn get_home_or_trending_feed(&self) -> Result<Vec<VideoCardItem>, InnertubeError> {
        // First try FEwhat_to_watch or FEtrending
        if let Ok(val) = self.browse("FEtrending").await {
            let mut results = Vec::new();
            extract_video_renderers(&val, &mut results);
            if !results.is_empty() {
                return Ok(results);
            }
        }
        // Fallback to curated popular query if anonymous FEwhat_to_watch requires visitor history
        self.search_videos("4K HDR nature technology science").await
    }

    /// Search Autocomplete suggestions from YouTube Suggest API.
    pub async fn search_suggestions(&self, query: &str) -> Result<Vec<String>, InnertubeError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let url = format!(
            "https://suggestqueries-clients6.youtube.com/complete/search?client=youtube&ds=yt&q={}",
            url::form_urlencoded::byte_serialize(query.as_bytes()).collect::<String>()
        );
        let text = self.http.get(&url).send().await?.text().await?;
        // Response is JSONP: window.google.ac.h(["query",[["suggestion 1",0],[...]]])
        let mut suggestions = Vec::new();
        if let (Some(start), Some(end)) = (text.find('['), text.rfind(']')) {
            if let Ok(arr) = serde_json::from_str::<Value>(&text[start..=end]) {
                if let Some(items) = arr.get(1).and_then(|v| v.as_array()) {
                    for item in items {
                        if let Some(s) = item.get(0).and_then(|v| v.as_str()) {
                            suggestions.push(s.to_string());
                        }
                    }
                }
            }
        }
        Ok(suggestions)
    }

    /// Fetches `/next` (Related Videos, Chapters).
    pub async fn get_video_next(&self, video_id: &str) -> Result<VideoNextDetails, InnertubeError> {
        let url = format!("{INNERTUBE_API_BASE}/next?prettyPrint=false");
        let context = self.build_context_json(AppClient::Web).await;

        let body = json!({
            "context": context,
            "videoId": video_id,
        });

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(AppClient::Web.user_agent()));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let response = self
            .http
            .post(&url)
            .headers(headers)
            .json(&body)
            .send()
            .await?;

        let json_val = response.json::<Value>().await?;
        let mut related_videos = Vec::new();
        extract_video_renderers(&json_val, &mut related_videos);

        Ok(VideoNextDetails {
            related_videos,
            comments: Vec::new(),
            chapters: Vec::new(),
        })
    }
}

/// Recursively walks an Innertube JSON response tree to extract `videoRenderer`,
/// `compactVideoRenderer`, and `gridVideoRenderer` items into normalized `VideoCardItem`s.
fn extract_video_renderers(node: &Value, out: &mut Vec<VideoCardItem>) {
    match node {
        Value::Object(map) => {
            for key in ["videoRenderer", "compactVideoRenderer", "gridVideoRenderer"] {
                if let Some(renderer) = map.get(key) {
                    if let Some(card) = parse_video_renderer(renderer) {
                        if !out.iter().any(|existing| existing.video_id == card.video_id) {
                            out.push(card);
                        }
                    }
                }
            }
            for val in map.values() {
                extract_video_renderers(val, out);
            }
        }
        Value::Array(arr) => {
            for val in arr {
                extract_video_renderers(val, out);
            }
        }
        _ => {}
    }
}

fn parse_video_renderer(r: &Value) -> Option<VideoCardItem> {
    let video_id = r.get("videoId")?.as_str()?.to_string();
    let title = extract_text(r.get("title")?);
    if title.is_empty() {
        return None;
    }

    let channel_node = r.get("ownerText").or_else(|| r.get("longBylineText")).or_else(|| r.get("shortBylineText"));
    let channel_name = channel_node.map(extract_text).unwrap_or_else(|| "YouTube Creator".to_string());

    let channel_id = channel_node
        .and_then(|n| n.pointer("/runs/0/navigationEndpoint/browseEndpoint/browseId"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let view_count_text = r
        .get("shortViewCountText")
        .or_else(|| r.get("viewCountText"))
        .map(extract_text)
        .unwrap_or_else(|| "".to_string());

    let published_time_text = r
        .get("publishedTimeText")
        .map(extract_text)
        .unwrap_or_else(|| "".to_string());

    let length_text = r
        .get("lengthText")
        .map(extract_text)
        .unwrap_or_else(|| "LIVE".to_string());

    let thumbnail_url = r
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());

    let is_live = length_text == "LIVE" || r.get("badges").map(|b| b.to_string().contains("LIVE")).unwrap_or(false);

    Some(VideoCardItem {
        video_id,
        title,
        channel_name,
        channel_id,
        view_count_text,
        published_time_text,
        length_text,
        thumbnail_url,
        is_live,
    })
}

fn extract_text(node: &Value) -> String {
    if let Some(simple) = node.get("simpleText").and_then(|v| v.as_str()) {
        return simple.to_string();
    }
    if let Some(runs) = node.get("runs").and_then(|v| v.as_array()) {
        return runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

fn extract_codecs(mime_type: &str) -> String {
    if let Some(idx) = mime_type.find("codecs=\"") {
        let rest = &mime_type[idx + 8..];
        if let Some(end) = rest.find('\"') {
            return rest[..end].to_string();
        }
    }
    String::new()
}
