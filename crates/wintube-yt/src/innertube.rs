//! Search / browse / comments / live-chat via the InnerTube JSON API.
//!
//! Search goes straight to `youtubei/v1/search`, which is roughly an order of
//! magnitude faster than shelling out to yt-dlp and does not need the sidecar
//! at all. yt-dlp stays as the fallback (and remains the source of truth for
//! formats, deciphering and downloads).

use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use wintube_core::{PlaybackStep, PlaybackStreams, VideoCard};

/// The public InnerTube key used by the YouTube web client. It is shipped in
/// every web page and grants no account access — it only identifies the
/// client to the API.
const WEB_API_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const WEB_CLIENT_VERSION: &str = "2.20240726.00.00";
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// One page of search results plus the token for the next page.
pub struct SearchPage {
    pub cards: Vec<VideoCard>,
    pub continuation: Option<String>,
}

/// Search YouTube (InnerTube /search). Returns lightweight cards for the grid.
pub async fn search(query: &str) -> Result<Vec<VideoCard>, String> {
    Ok(search_page(query, None).await?.cards)
}

/// One page of /search results. `continuation` comes from the previous
/// page's `continuationItemRenderer`; `None` fetches the first page.
pub async fn search_page(
    query: &str,
    continuation: Option<&str>,
) -> Result<SearchPage, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(USER_AGENT)
        .build()
        .map_err(|e| e.to_string())?;

    let mut body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": WEB_CLIENT_VERSION,
                "hl": "en",
                "gl": "US",
            }
        },
        "query": query,
    });
    if let Some(token) = continuation {
        body["continuation"] = serde_json::json!(token);
    }

    // DNS to youtube.com is flaky on some networks, so a single transport
    // error should not cost the user the whole search.
    let mut last_err = String::new();
    for attempt in 0..2 {
        let result = async {
            let resp = client
                .post(format!(
                    "https://www.youtube.com/youtubei/v1/search?key={WEB_API_KEY}"
                ))
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("InnerTube request failed: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("InnerTube search failed: {}", resp.status()));
            }
            resp.json::<Value>().await.map_err(|e| e.to_string())
        }
        .await;

        match result {
            Ok(json) => {
                let mut renderers = Vec::new();
                collect_video_renderers(&json, &mut renderers);
                let cards: Vec<VideoCard> =
                    renderers.iter().filter_map(parse_card).collect();
                // A /search page carries ~20-30 videos plus channel/playlist
                // items; keep everything — the frontend paginates (Load more).
                let continuation = find_continuation(&json);
                if cards.is_empty() && continuation.is_none() {
                    return Err("InnerTube returned no video results".to_string());
                }
                return Ok(SearchPage { cards, continuation });
            }
            Err(e) => {
                last_err = e;
                if attempt == 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                }
            }
        }
    }
    Err(last_err)
}

/// Find the next-page token (`continuationItemRenderer`) in a search payload.
fn find_continuation(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if key == "continuationItemRenderer" {
                    if let Some(token) = val
                        .get("continuationEndpoint")
                        .and_then(|e| e.get("continuationCommand"))
                        .and_then(|c| c.get("token"))
                        .and_then(|t| t.as_str())
                    {
                        return Some(token.to_string());
                    }
                }
                if let Some(found) = find_continuation(val) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => items.iter().filter_map(find_continuation).next(),
        _ => None,
    }
}

/// Walk the response and collect every `videoRenderer` object.
///
/// The search payload nests results several levels deep and the exact path
/// changes with YouTube's A/B tests, so collect recursively instead of
/// hard-coding a path.
fn collect_video_renderers(value: &Value, out: &mut Vec<Value>) {
    match value {
        Value::Object(map) => {
            for (key, val) in map {
                if key == "videoRenderer" {
                    if val.is_object() {
                        out.push(val.clone());
                    }
                } else {
                    collect_video_renderers(val, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_video_renderers(item, out);
            }
        }
        _ => {}
    }
}

/// Convert one `videoRenderer` into a grid card.
fn parse_card(v: &Value) -> Option<VideoCard> {
    let id = v.get("videoId")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let title = text_of(v.get("title")).unwrap_or_else(|| "Untitled".to_string());
    let channel = text_of(v.get("ownerText")).unwrap_or_default();
    let duration = text_of(v.get("lengthText")).and_then(|s| parse_duration(&s));
    let views = text_of(v.get("viewCountText"));
    let age = text_of(v.get("publishedTimeText"));
    let thumbnail = v
        .get("thumbnail")
        .and_then(|t| t.get("thumbnails"))
        .and_then(|t| t.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|t| t.get("url").and_then(|u| u.as_str()))
                .next_back()
        })
        .unwrap_or("")
        .to_string();
    Some(VideoCard {
        id,
        title,
        channel,
        views,
        age,
        duration,
        thumbnail,
    })
}

/// Read a text node: InnerTube uses either `simpleText` or a list of `runs`.
fn text_of(value: Option<&Value>) -> Option<String> {
    let v = value?;
    if let Some(s) = v.get("simpleText").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    if let Some(runs) = v.get("runs").and_then(|x| x.as_array()) {
        let joined: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    v.as_str().map(|s| s.to_string())
}

/// Parse `m:ss` / `h:mm:ss` into seconds.
fn parse_duration(text: &str) -> Option<u64> {
    let parts: Vec<u64> = text
        .split(':')
        .filter_map(|p| p.trim().parse::<u64>().ok())
        .collect();
    match parts.len() {
        2 => Some(parts[0] * 60 + parts[1]),
        3 => Some(parts[0] * 3600 + parts[1] * 60 + parts[2]),
        _ => None,
    }
}

/// Trending feed via InnerTube /browse. Kept for the Home page once the
/// region-specific trending params are wired up.
pub async fn trending() -> Result<Vec<VideoCard>, String> {
    search("trending music").await
}

// ---------------------------------------------------------------------------
// Playback ladder via the InnerTube player API
// ---------------------------------------------------------------------------
//
// yt-dlp only surfaces a direct URL for the progressive 360p stream on videos
// where YouTube has enabled SABR-only streaming. The VISIONOS player client
// still returns ordinary adaptive URLs for the whole AVC ladder, but only when
// the request carries a visitor session (`visitorData`). We fetch that session
// from the YouTube homepage once, cache it, and ask VISIONOS for the ladder.
// This replaces the old yt-dlp-based `get_playback_streams` fast path; yt-dlp
// remains the fallback for videos the VISIONOS client cannot play.

const VISIONOS_CLIENT_VERSION: &str = "1.02";
const VISIONOS_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

struct VisitorCache {
    value: String,
    fetched: Instant,
}

static VISITOR: OnceLock<std::sync::Mutex<Option<VisitorCache>>> = OnceLock::new();
const VISITOR_TTL: Duration = Duration::from_secs(6 * 60 * 60);

fn visitor_slot() -> &'static std::sync::Mutex<Option<VisitorCache>> {
    VISITOR.get_or_init(|| std::sync::Mutex::new(None))
}

/// Return a cached visitor session, refreshing it when stale or forced.
async fn visitor_data(force: bool) -> Option<String> {
    {
        let guard = visitor_slot().lock().ok()?;
        if !force {
            if let Some(c) = guard.as_ref() {
                if c.fetched.elapsed() < VISITOR_TTL {
                    return Some(c.value.clone());
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(VISIONOS_UA)
        .build()
        .ok()?;
    let html = client
        .get("https://www.youtube.com/")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Encoding", "identity")
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let value = extract_visitor(&html)?;
    if let Ok(mut guard) = visitor_slot().lock() {
        *guard = Some(VisitorCache {
            value: value.clone(),
            fetched: Instant::now(),
        });
    }
    Some(value)
}

fn extract_visitor(html: &str) -> Option<String> {
    let needle = "\"VISITOR_DATA\":\"";
    let start = html.find(needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    Some(percent_decode(&rest[..end]))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Resolve the full AVC + AAC playback ladder through the VISIONOS client.
/// Returns an error when YouTube refuses the session or serves SABR-only
/// formats; callers should then fall back to yt-dlp's progressive stream.
pub async fn playback_streams(video_id: &str) -> Result<PlaybackStreams, String> {
    let mut last_err = String::new();
    for attempt in 0..2 {
        let visitor = match visitor_data(attempt > 0).await {
            Some(v) => v,
            None => {
                last_err = "could not obtain a YouTube visitor session".to_string();
                break;
            }
        };
        match fetch_player(video_id, &visitor).await {
            Ok(streams) => return Ok(streams),
            Err(e) => {
                last_err = e;
                if attempt == 0 {
                    tokio::time::sleep(Duration::from_millis(350)).await;
                }
            }
        }
    }
    Err(last_err)
}

async fn fetch_player(video_id: &str, visitor: &str) -> Result<PlaybackStreams, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .user_agent(VISIONOS_UA)
        .build()
        .map_err(|e| e.to_string())?;

    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "VISIONOS",
                "clientVersion": VISIONOS_CLIENT_VERSION,
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "hl": "en",
                "gl": "US",
                "visitorData": visitor,
            }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });

    let resp = client
        .post(format!(
            "https://www.youtube.com/youtubei/v1/player?key={WEB_API_KEY}"
        ))
        .header("X-Goog-Visitor-Id", visitor)
        .header("X-YouTube-Client-Name", "101")
        .header("X-YouTube-Client-Version", VISIONOS_CLIENT_VERSION)
        .header("Origin", "https://www.youtube.com")
        .header("Referer", "https://www.youtube.com/")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("InnerTube player request failed: {e}"))?;

    let status = resp.status();
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("InnerTube player response was not JSON: {e}"))?;
    if !status.is_success() {
        return Err(format!("InnerTube player HTTP {status}"));
    }

    let playability = json
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        .unwrap_or("");
    if playability != "OK" {
        let reason = json
            .pointer("/playabilityStatus/reason")
            .and_then(Value::as_str)
            .unwrap_or("");
        return Err(format!("InnerTube player {playability}: {reason}"));
    }

    let formats = json
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .ok_or_else(|| "InnerTube returned no adaptive formats".to_string())?;

    let mut video_by_height: std::collections::HashMap<u32, (f64, PlaybackStep)> =
        std::collections::HashMap::new();
    let mut best_aac: Option<(f64, String)> = None;
    let mut best_audio: Option<(f64, String)> = None;

    for f in formats {
        let Some(url) = f.get("url").and_then(Value::as_str) else {
            continue;
        };
        let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
        let bitrate = f
            .get("bitrate")
            .and_then(Value::as_f64)
            .or_else(|| f.get("averageBitrate").and_then(Value::as_f64))
            .unwrap_or(0.0);
        let height = f.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
        let width = f.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;

        if mime.starts_with("video/mp4") && mime.contains("avc1") && height > 0 {
            let better = video_by_height
                .get(&height)
                .map(|(b, _)| bitrate > *b)
                .unwrap_or(true);
            if better {
                video_by_height.insert(
                    height,
                    (
                        bitrate,
                        PlaybackStep {
                            height,
                            width,
                            url: url.to_string(),
                        },
                    ),
                );
            }
        } else if mime.starts_with("audio/") {
            if mime.starts_with("audio/mp4") && mime.contains("mp4a") {
                if best_aac.as_ref().map(|(b, _)| bitrate > *b).unwrap_or(true) {
                    best_aac = Some((bitrate, url.to_string()));
                }
            }
            if best_audio.as_ref().map(|(b, _)| bitrate > *b).unwrap_or(true) {
                best_audio = Some((bitrate, url.to_string()));
            }
        }
    }

    let mut video: Vec<PlaybackStep> = video_by_height
        .into_values()
        .map(|(_, step)| step)
        .collect();
    video.sort_by_key(|s| s.height);
    if video.is_empty() {
        return Err("InnerTube returned no AVC video formats".to_string());
    }
    let audio_url = best_aac.or(best_audio).map(|(_, url)| url);
    if audio_url.is_none() {
        return Err("InnerTube returned no audio formats".to_string());
    }

    let duration = json
        .pointer("/videoDetails/lengthSeconds")
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| v.as_u64())
        });

    Ok(PlaybackStreams {
        video,
        audio_url,
        duration,
    })
}
