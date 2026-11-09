//! Search / browse / comments / live-chat via the InnerTube JSON API.
//!
//! Search goes straight to `youtubei/v1/search`, which is roughly an order of
//! magnitude faster than shelling out to yt-dlp and does not need the sidecar
//! at all. yt-dlp stays as the fallback (and remains the source of truth for
//! formats, deciphering and downloads).

use serde_json::Value;
use wintube_core::VideoCard;

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
