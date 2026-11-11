//! Streaming relay: a `wintube-media://` custom protocol registered with
//! Tauri, so WebView2 loads the video as a page resource.
//!
//! WebView2 refuses YouTube's cross-origin fMP4 stream for a plain
//! `<video src>` (code 4 "Format error"). Bytes served *through* a
//! registered protocol handler are handed to the page by the engine itself,
//! exactly like `tauri:` and `asset:` — that's the path Windows uses for
//! custom schemes.
//!
//! Range requests are forwarded to Google, so seeking and progressive
//! buffering both work. The signed googlevideo URLs never leave the app.

use std::collections::HashMap;
use std::io::Read;
use std::sync::{Mutex, OnceLock};

fn streams() -> &'static Mutex<HashMap<u64, String>> {
    static STREAMS: OnceLock<Mutex<HashMap<u64, String>>> = OnceLock::new();
    STREAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Register a remote URL, return the local stream id.
pub fn register(url: String) -> u64 {
    let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    streams().lock().unwrap().insert(id, url);
    id
}

fn get_stream(id: u64) -> Option<String> {
    streams().lock().unwrap().get(&id).cloned()
}

/// Url prefix used in the page. On Windows, wry exposes custom protocols as
/// `http://<scheme>.localhost` and intercepts those requests; using that form
/// directly makes WebView2 treat the stream as an ordinary HTTP resource
/// instead of an unknown custom scheme.
pub fn protocol_prefix() -> &'static str {
    "http://wintube-media.localhost"
}

/// Rewrite the resolved PlaybackStreams to point at the custom protocol.
pub fn rewrite(streams: wintube_core::PlaybackStreams) -> wintube_core::PlaybackStreams {
    let base = protocol_prefix();
    let mut out = streams;
    for step in &mut out.video {
        let id = register(std::mem::take(&mut step.url));
        step.url = format!("{base}/stream/{id}");
    }
    out.audio_url = out.audio_url.map(|url| {
        let id = register(url);
        format!("{base}/stream/{id}")
    });
    out
}

/// How much media to hand back per custom-protocol response. WebView2 asks
/// for the next slice with a Range request as the player consumes data; the
/// cap keeps a single response from buffering a multi-GB video in RAM.
const CHUNK: u64 = 8 * 1024 * 1024;

/// Async responder: fetches one (possibly ranged) slice from Google and
/// hands the bytes back to WebView2. Runs on its own thread.
pub fn handle(
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let uri = request.uri().to_string();
    let is_head = request.method() == tauri::http::Method::HEAD;
    let range = request
        .headers()
        .get("Range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    crate::log_line(&format!(
        "[relay] request {uri} range={}",
        range.as_deref().unwrap_or("none")
    ));

    std::thread::spawn(move || {
        let Some(id) = stream_id(&uri) else {
            respond(responder, 404, "text/plain", b"not found".to_vec());
            return;
        };
        let Some(url) = get_stream(id) else {
            respond(responder, 404, "text/plain", b"unknown stream".to_vec());
            return;
        };

        // When the media stack asks for the whole resource, ask Google for
        // just the first chunk. The 206 + Content-Range response tells
        // WebView2 the real length, and it will range-request the rest.
        let requested_start = range.as_deref().and_then(range_start);
        let upstream_range = match &range {
            Some(r) => Some(r.clone()),
            None => Some(format!("bytes=0-{}", CHUNK - 1)),
        };

        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36")
            .build()
            .expect("relay http client");

        let mut req = client.get(&url);
        if let Some(r) = &upstream_range {
            req = req.header("Range", r.as_str());
        }
        let resp = match req.send() {
            Ok(r) => r,
            Err(e) => {
                crate::log_line(&format!("[relay] {id} fetch failed: {e}"));
                respond(responder, 502, "text/plain", b"fetch failed".to_vec());
                return;
            }
        };

        let status = resp.status().as_u16();
        let content_length = resp.content_length();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("video/mp4")
            .to_string();
        let content_range = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let total = content_range
            .as_deref()
            .and_then(content_range_total)
            .or(content_length);

        if status == 416 {
            let mut builder = tauri::http::Response::builder()
                .status(416)
                .header("Content-Type", content_type)
                .header("Accept-Ranges", "bytes");
            if let Some(t) = total {
                builder = builder.header("Content-Range", format!("bytes */{t}"));
            }
            responder.respond(builder.body(Vec::new()).expect("relay 416"));
            return;
        }

        if is_head {
            let mut builder = tauri::http::Response::builder()
                .status(200)
                .header("Content-Type", content_type)
                .header("Accept-Ranges", "bytes");
            if let Some(t) = total {
                builder = builder.header("Content-Length", t.to_string());
            }
            responder.respond(builder.body(Vec::new()).expect("relay head"));
            return;
        }

        let mut bytes = Vec::new();
        let mut limited = resp.take(CHUNK);
        if let Err(e) = limited.read_to_end(&mut bytes) {
            crate::log_line(&format!("[relay] {id} body read failed: {e}"));
            respond(responder, 502, "text/plain", b"body read failed".to_vec());
            return;
        }
        if bytes.is_empty() {
            crate::log_line(&format!("[relay] {id} upstream {status} returned no bytes"));
            respond(responder, 502, "text/plain", b"empty body".to_vec());
            return;
        }

        let start = requested_start.unwrap_or(0);
        let len = bytes.len() as u64;
        crate::log_line(&format!(
            "[relay] {id} upstream={status} start={start} bytes={len} total={}",
            total.map(|t| t.to_string()).unwrap_or_else(|| "?".to_string())
        ));

        let mut builder = tauri::http::Response::builder()
            .header("Content-Type", content_type)
            .header("Accept-Ranges", "bytes")
            .header("Cache-Control", "no-store")
            .header("Access-Control-Allow-Origin", "*");

        if let Some(t) = total {
            builder = builder
                .status(206)
                .header("Content-Length", len.to_string())
                .header(
                    "Content-Range",
                    format!("bytes {}-{}/{}", start, start + len - 1, t),
                );
        } else {
            builder = builder.status(200).header("Content-Length", len.to_string());
        }

        let response = builder.body(bytes).expect("relay response");
        responder.respond(response);
    });
}

/// `bytes=0-1023` / `bytes=1024-` -> start offset.
fn range_start(range: &str) -> Option<u64> {
    let rest = range.trim().strip_prefix("bytes=")?;
    let first = rest.split(',').next()?.trim();
    let start = first.split('-').next()?.trim();
    start.parse().ok()
}

/// `bytes 0-1023/123456` -> 123456.
fn content_range_total(range: &str) -> Option<u64> {
    range.rsplit('/').next()?.trim().parse().ok()
}

fn stream_id(uri: &str) -> Option<u64> {
    let path = uri.split("/stream/").nth(1)?;
    let id = path.split(['?', '#']).next()?;
    id.parse().ok()
}

fn respond(
    responder: tauri::UriSchemeResponder,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) {
    let response = tauri::http::Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .expect("error response");
    responder.respond(response);
}
