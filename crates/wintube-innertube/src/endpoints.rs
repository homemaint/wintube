use std::collections::HashMap;
use std::sync::Arc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT, REFERER};
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};

use crate::client::AppClient;
use crate::cipher::CipherEngine;
use crate::dash::DashManifestBuilder;
use crate::models::{
    ByteRange, CaptionTrack, FormatStream, InnertubeContext, PlayerResponse,
    ResolvedStream, VideoDetails, VideoMetadata, VideoPlaybackInfo,
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

        let player_res: PlayerResponse = response.json().await?;

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
