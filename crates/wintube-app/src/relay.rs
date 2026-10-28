//! Loopback HTTP relay for YouTube media streams (googlevideo / GVS).
//!
//! # Why this exists
//!
//! YouTube's media CDN is picky about *who* asks for what:
//!
//! * The `VISIONOS` client (what this app uses) serves open-ended
//!   `Range: bytes=0-` requests in full (`206`), sometimes behind a `302`
//!   that must be followed. That is the fast path.
//! * Older clients (`IOS`, `ANDROID_VR`) answer open-ended requests with a
//!   `302` to a "CMS redirect" URL whose follow-up is rejected with
//!   `403 Forbidden`. Bounded ranges are still served, so those URLs work
//!   through the fallback path.
//! * MPV/FFmpeg's HTTP layer does not follow such redirects, so a direct URL
//!   can abort immediately with "HTTP error 403" and the UI flickers back to
//!   "paused".
//!
//! This module therefore runs a tiny HTTP/1.1 server on `127.0.0.1`:
//!
//! * mpv talks ordinary HTTP to `http://127.0.0.1:<port>/s/<id>`,
//! * the relay forwards the range upstream (following redirects), falling
//!   back to a sequence of bounded ranges when the CDN refuses it,
//! * it transparently re-resolves the stream through innertube when the CDN
//!   answers `403` (expired URL, session block, ...).
//!
//! The client sees a normal byte stream with correct `Content-Length` /
//! `Content-Range` headers, so mpv can open, seek and play the media.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{RANGE, REFERER, USER_AGENT};
use reqwest::StatusCode;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

/// Largest bounded range requested upstream on the fallback path.
///
/// VISIONOS URLs serve bounded ranges of any size with `206`; 8 MiB keeps
/// the number of round trips low for the older clients that need chunking
/// without holding a large slice in memory.
const UPSTREAM_CHUNK: u64 = 8 * 1024 * 1024;

/// How often a single upstream range may be re-resolved after a `403`.
const MAX_RESOLVE_ATTEMPTS: u32 = 3;

/// How often a transient upstream/network failure is retried.
const MAX_TRANSPORT_RETRIES: u32 = 3;

/// Future returned by a [`Resolver`].
pub type ResolveFuture = Pin<Box<dyn Future<Output = Option<UpstreamInfo>> + Send>>;

/// Callback used to obtain a fresh upstream URL for a stream that the CDN
/// refused (HTTP 403) or that expired.
pub type Resolver = Arc<dyn Fn() -> ResolveFuture + Send + Sync>;

/// Everything the relay needs to know about one upstream media stream.
#[derive(Clone, Debug)]
pub struct UpstreamInfo {
    /// Direct googlevideo URL.
    pub url: String,
    /// MIME type of the stream (e.g. `video/mp4`).
    pub mime: String,
    /// User agent used for upstream requests (must match the innertube client
    /// that produced the URL).
    pub user_agent: String,
    /// Optional `Referer` header for upstream requests.
    pub referer: Option<String>,
    /// Total size of the stream in bytes, when known.
    pub total_len: Option<u64>,
}

impl UpstreamInfo {
    /// Builds an upstream descriptor, deriving the content length and MIME
    /// type from the googlevideo URL query string when possible
    /// (`clen=` / `mime=` parameters).
    pub fn new(
        url: impl Into<String>,
        mime: impl Into<String>,
        user_agent: impl Into<String>,
    ) -> Self {
        let url = url.into();
        let mime_from_url = url_param(&url, "mime")
            .map(|m| percent_decode(&m))
            .filter(|m| !m.is_empty());
        let total_len = url_param(&url, "clen").and_then(|v| v.parse::<u64>().ok());

        Self {
            url,
            mime: mime_from_url.unwrap_or_else(|| mime.into()),
            user_agent: user_agent.into(),
            referer: Some("https://www.youtube.com/".to_string()),
            total_len,
        }
    }
}

struct SlotState {
    upstream: tokio::sync::RwLock<UpstreamInfo>,
    resolver: Option<Resolver>,
}

/// Local media relay.
///
/// The relay owns a dedicated Tokio runtime so it keeps serving even while the
/// UI thread is busy.
pub struct StreamRelay {
    base_url: String,
    slots: Arc<Mutex<HashMap<String, Arc<SlotState>>>>,
    next_id: AtomicU64,
    _runtime: tokio::runtime::Runtime,
}

impl StreamRelay {
    /// Binds the relay to a random free loopback port and starts serving.
    pub fn start() -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("wintube-relay")
            .enable_all()
            .build()?;

        let listener = runtime.block_on(TcpListener::bind(("127.0.0.1", 0)))?;
        let addr = listener.local_addr()?;

        let slots: Arc<Mutex<HashMap<String, Arc<SlotState>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let http = reqwest::Client::builder()
            // Redirects are followed by reqwest itself; the final URL is
            // remembered afterwards so the next range request skips the
            // redirect round-trip.
            .redirect(reqwest::redirect::Policy::limited(5))
            .tcp_nodelay(true)
            .pool_max_idle_per_host(8)
            .connect_timeout(Duration::from_secs(15))
            .build()?;

        let ctx = Arc::new(ServerCtx {
            slots: slots.clone(),
            http: http.clone(),
        });

        runtime.spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _peer)) => {
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            handle_connection(stream, ctx).await;
                        });
                    }
                    Err(err) => {
                        warn!("[relay] accept failed: {err}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }
        });

        info!("[relay] media relay listening on http://{addr}");

        Ok(Self {
            base_url: format!("http://{addr}"),
            slots,
            next_id: AtomicU64::new(1),
            _runtime: runtime,
        })
    }

    /// Registers an upstream stream and returns the loopback URL mpv should use.
    pub fn register(&self, upstream: UpstreamInfo, resolver: Option<Resolver>) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let state = Arc::new(SlotState {
            upstream: tokio::sync::RwLock::new(upstream),
            resolver,
        });
        self.slots.lock().unwrap().insert(id.clone(), state);
        format!("{}/s/{}", self.base_url, id)
    }

    /// Base URL of the relay (useful for diagnostics).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

struct ServerCtx {
    slots: Arc<Mutex<HashMap<String, Arc<SlotState>>>>,
    http: reqwest::Client,
}

struct HttpRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
}

async fn handle_connection(stream: TcpStream, ctx: Arc<ServerCtx>) {
    let _ = stream.set_nodelay(true);
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    loop {
        let request = match read_request(&mut reader).await {
            Ok(Some(req)) => req,
            Ok(None) => break,
            Err(err) => {
                debug!("[relay] request read failed: {err}");
                break;
            }
        };

        let keep_alive = request
            .headers
            .get("connection")
            .map(|v| !v.eq_ignore_ascii_case("close"))
            .unwrap_or(true);

        match serve_request(&ctx, &request, &mut write_half).await {
            Ok(true) => {
                if !keep_alive {
                    break;
                }
            }
            Ok(false) => break,
            Err(err) => {
                debug!("[relay] request failed: {err}");
                break;
            }
        }
    }

    let _ = write_half.shutdown().await;
}

async fn read_request(reader: &mut BufReader<OwnedReadHalf>) -> io::Result<Option<HttpRequest>> {
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        return Ok(None);
    }

    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    if method.is_empty() || target.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "malformed request line",
        ));
    }

    let mut headers = HashMap::new();
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line).await? == 0 {
            break;
        }
        let trimmed = header_line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            headers.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    Ok(Some(HttpRequest {
        method,
        target,
        headers,
    }))
}

/// Serves one request. Returns `true` when the connection may be reused.
async fn serve_request(
    ctx: &Arc<ServerCtx>,
    request: &HttpRequest,
    writer: &mut OwnedWriteHalf,
) -> io::Result<bool> {
    let path = request.target.split('?').next().unwrap_or("");
    let Some(slot_id) = path.strip_prefix("/s/") else {
        write_simple(writer, 404, "text/plain", b"not found").await?;
        return Ok(false);
    };

    let slot = {
        let slots = ctx.slots.lock().unwrap();
        slots.get(slot_id).cloned()
    };
    let Some(slot) = slot else {
        write_simple(writer, 404, "text/plain", b"unknown stream").await?;
        return Ok(false);
    };

    let head_only = request.method.eq_ignore_ascii_case("HEAD");
    let upstream = slot.upstream.read().await.clone();

    // Resolve the total size (needed for Content-Length / Content-Range).
    let mut total = upstream.total_len;
    if total.is_none() {
        total = probe_total_len(ctx, &slot).await;
        if let Some(size) = total {
            slot.upstream.write().await.total_len = Some(size);
        }
    }
    let Some(total_len) = total else {
        write_simple(writer, 502, "text/plain", b"stream size unknown").await?;
        return Ok(false);
    };

    let range_header = request.headers.get("range").map(|s| s.as_str());
    let (start, requested_end) = match parse_range(range_header) {
        Some((s, e)) => (s, e),
        None => (0, None),
    };

    if start >= total_len {
        let body = format!(
            "bytes {}-{}/{}",
            start,
            total_len.saturating_sub(1),
            total_len
        );
        let response = format!(
            "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            body
        );
        writer.write_all(response.as_bytes()).await?;
        return Ok(false);
    }

    let end = requested_end
        .unwrap_or(total_len - 1)
        .min(total_len - 1)
        .max(start);
    let length = end - start + 1;

    let head = format!(
        "HTTP/1.1 206 Partial Content\r\n\
         Content-Type: {}\r\n\
         Accept-Ranges: bytes\r\n\
         Content-Length: {}\r\n\
         Content-Range: bytes {}-{}/{}\r\n\
         Cache-Control: no-store\r\n\
         Connection: keep-alive\r\n\r\n",
        upstream.mime, length, start, end, total_len
    );
    writer.write_all(head.as_bytes()).await?;

    if head_only {
        return Ok(true);
    }

    debug!(
        "[relay] serving bytes {}-{} of {} ({})",
        start, end, total_len, upstream.mime
    );

    let upstream_end = requested_end.map(|requested| requested.min(total_len - 1));
    match stream_range(ctx, &slot, start, upstream_end, Some(total_len), writer).await {
        Ok(()) => Ok(true),
        Err(err) => {
            warn!("[relay] stream aborted at byte {start}: {err}");
            Ok(false)
        }
    }
}

/// Streams `[start, end]` (or `[start, EOF)` when `end` is `None`) to the
/// client.
///
/// Strategy:
/// 1. **Fast path** – one upstream request for the whole remaining span
///    (open-ended when the client did not ask for a bounded range). The
///    VISIONOS client serves these directly, so playback keeps full CDN
///    throughput.
/// 2. **Bounded fallback** – if the CDN refuses the open range (IOS /
///    ANDROID_VR URLs answer `302`/`403`), the remaining bytes are fetched in
///    2 MiB slices, which the CDN always serves.
async fn stream_range(
    ctx: &Arc<ServerCtx>,
    slot: &Arc<SlotState>,
    start: u64,
    end: Option<u64>,
    total: Option<u64>,
    writer: &mut OwnedWriteHalf,
) -> io::Result<()> {
    let hard_end = match (end, total) {
        (Some(end), _) => end,
        (None, Some(total)) => total.saturating_sub(1),
        (None, None) => u64::MAX,
    };
    if start > hard_end {
        return Ok(());
    }

    let mut position = start;

    // 1) Fast path: one request for the whole remaining span.
    if hard_end.saturating_sub(position).saturating_add(1) > UPSTREAM_CHUNK {
        match fetch_range(ctx, slot, position, end, false).await {
            Ok(mut response) => {
                let skip = if response.status() == StatusCode::OK {
                    position
                } else {
                    0
                };
                let expected = hard_end - position + 1;
                let outcome = pump_body(&mut response, writer, skip, expected).await;
                position += outcome.written;

                match outcome.error {
                    Some(PumpError::Client(err)) => return Err(err),
                    Some(PumpError::Upstream(err)) => {
                        debug!(
                            "[relay] open stream interrupted at byte {position}: {err}; continuing with bounded ranges"
                        );
                    }
                    None => {
                        if position > hard_end {
                            return Ok(());
                        }
                    }
                }
            }
            Err(FetchError::Forbidden) => {
                debug!(
                    "[relay] upstream refused the open range; switching to bounded 2 MiB ranges"
                );
            }
            Err(FetchError::EndOfStream) => return Ok(()),
            Err(FetchError::Other(err)) => return Err(err),
        }
    }

    // 2) Bounded fallback: 2 MiB slices.
    let mut slice_failures = 0u32;
    while position <= hard_end {
        let chunk_end = position.saturating_add(UPSTREAM_CHUNK - 1).min(hard_end);

        let mut response = match fetch_range(ctx, slot, position, Some(chunk_end), true).await {
            Ok(response) => response,
            Err(FetchError::Forbidden) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "upstream refused the range",
                ));
            }
            Err(FetchError::EndOfStream) => break,
            Err(FetchError::Other(err)) => return Err(err),
        };

        let skip = if response.status() == StatusCode::OK {
            position
        } else {
            0
        };
        let expected = chunk_end - position + 1;
        let outcome = pump_body(&mut response, writer, skip, expected).await;

        match outcome.error {
            Some(PumpError::Client(err)) => return Err(err),
            Some(PumpError::Upstream(err)) => {
                position += outcome.written;
                if outcome.written > 0 {
                    slice_failures = 0;
                    continue;
                }
                slice_failures += 1;
                if slice_failures > MAX_TRANSPORT_RETRIES {
                    return Err(io::Error::other(format!(
                        "upstream body failed at byte {position}: {err}"
                    )));
                }
                tokio::time::sleep(Duration::from_millis(250 * slice_failures as u64)).await;
                continue;
            }
            None => {
                position += outcome.written;
                if outcome.written == 0 {
                    debug!("[relay] upstream returned no data at byte {position}; end of stream");
                    break;
                }
                if outcome.written < expected {
                    debug!(
                        "[relay] short upstream read at byte {position} ({} / {expected}) -> end of stream",
                        outcome.written
                    );
                    break;
                }
            }
        }
    }

    Ok(())
}

enum FetchError {
    Forbidden,
    EndOfStream,
    Other(io::Error),
}

/// Issues one upstream range request, following redirects, retrying transient
/// failures and refreshing the stream URL when the CDN answers `403`.
async fn fetch_range(
    ctx: &Arc<ServerCtx>,
    slot: &Arc<SlotState>,
    start: u64,
    end: Option<u64>,
    allow_resolve: bool,
) -> Result<reqwest::Response, FetchError> {
    let range = match end {
        Some(end) => format!("bytes={start}-{end}"),
        None => format!("bytes={start}-"),
    };
    let mut resolve_attempts = 0u32;
    let mut transport_attempts = 0u32;

    loop {
        let upstream = slot.upstream.read().await.clone();
        let mut req = ctx
            .http
            .get(&upstream.url)
            .header(RANGE, range.clone())
            .header(USER_AGENT, upstream.user_agent.clone());
        if let Some(referer) = upstream.referer.clone() {
            req = req.header(REFERER, referer);
        }

        match req.send().await {
            Ok(response) => {
                let status = response.status();
                if status == StatusCode::PARTIAL_CONTENT {
                    // Remember the URL the redirect chain ended on, so the
                    // next range request does not pay for it again.
                    let final_url = response.url().to_string();
                    if final_url != upstream.url {
                        slot.upstream.write().await.url = final_url;
                    }
                    return Ok(response);
                }
                if status == StatusCode::RANGE_NOT_SATISFIABLE {
                    debug!("[relay] upstream answered 416 for {range} -> end of stream");
                    return Err(FetchError::EndOfStream);
                }
                if status == StatusCode::FORBIDDEN {
                    if allow_resolve {
                        resolve_attempts += 1;
                        if let Some(resolver) = slot.resolver.clone() {
                            if resolve_attempts <= MAX_RESOLVE_ATTEMPTS {
                                warn!(
                                    "[relay] upstream 403 for {range}; re-resolving stream URL (attempt {resolve_attempts})"
                                );
                                if let Some(fresh) = resolver().await {
                                    *slot.upstream.write().await = fresh;
                                    continue;
                                }
                            }
                        }
                    }
                    return Err(FetchError::Forbidden);
                }
                if status == StatusCode::OK {
                    // The CDN ignored our Range header and started the whole
                    // file: skip forward instead of failing.
                    warn!("[relay] upstream ignored Range ({range}); skipping to {start}");
                    return Ok(response);
                }
                if status.is_server_error() {
                    transport_attempts += 1;
                    if transport_attempts <= MAX_TRANSPORT_RETRIES {
                        tokio::time::sleep(Duration::from_millis(250 * transport_attempts as u64))
                            .await;
                        continue;
                    }
                }
                return Err(FetchError::Other(io::Error::other(format!(
                    "upstream status {status}"
                ))));
            }
            Err(err) => {
                transport_attempts += 1;
                if transport_attempts <= MAX_TRANSPORT_RETRIES {
                    tokio::time::sleep(Duration::from_millis(250 * transport_attempts as u64))
                        .await;
                    continue;
                }
                return Err(FetchError::Other(io::Error::other(format!(
                    "upstream request failed: {err}"
                ))));
            }
        }
    }
}

enum PumpError {
    Client(io::Error),
    Upstream(String),
}

struct PumpOutcome {
    written: u64,
    error: Option<PumpError>,
}

/// Copies at most `limit` bytes of the response body to the client, skipping
/// `skip` bytes first (used when the CDN ignores the Range header).
async fn pump_body(
    response: &mut reqwest::Response,
    writer: &mut OwnedWriteHalf,
    mut skip: u64,
    limit: u64,
) -> PumpOutcome {
    let mut written = 0u64;
    while written < limit {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(err) => {
                return PumpOutcome {
                    written,
                    error: Some(PumpError::Upstream(format!("upstream body failed: {err}"))),
                };
            }
        };

        let mut data = chunk;
        if skip > 0 {
            let drop = skip.min(data.len() as u64) as usize;
            skip -= drop as u64;
            data = data.slice(drop..);
            if data.is_empty() {
                continue;
            }
        }

        let take = ((limit - written) as usize).min(data.len());
        if let Err(err) = writer.write_all(&data[..take]).await {
            return PumpOutcome {
                written,
                error: Some(PumpError::Client(err)),
            };
        }
        written += take as u64;
    }

    PumpOutcome {
        written,
        error: None,
    }
}

/// Determines the total size of a stream with a one-byte range request.
async fn probe_total_len(ctx: &Arc<ServerCtx>, slot: &Arc<SlotState>) -> Option<u64> {
    let upstream = slot.upstream.read().await.clone();
    let mut req = ctx
        .http
        .get(&upstream.url)
        .header(RANGE, "bytes=0-0")
        .header(USER_AGENT, upstream.user_agent.clone());
    if let Some(referer) = upstream.referer.clone() {
        req = req.header(REFERER, referer);
    }

    let response = req.send().await.ok()?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return None;
    }
    let content_range = response.headers().get("content-range")?.to_str().ok()?;
    let total = content_range
        .rsplit('/')
        .next()?
        .trim()
        .parse::<u64>()
        .ok()?;

    let final_url = response.url().to_string();
    if final_url != upstream.url {
        slot.upstream.write().await.url = final_url;
    }
    Some(total)
}

async fn write_simple(
    writer: &mut OwnedWriteHalf,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} Error\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    writer.write_all(response.as_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await
}

/// Parses a `Range: bytes=start-end` header.
fn parse_range(header: Option<&str>) -> Option<(u64, Option<u64>)> {
    let header = header?.trim();
    let spec = header.strip_prefix("bytes=")?;
    let first = spec.split(',').next()?.trim();
    let (start, end) = first.split_once('-')?;
    let start = start.trim().parse::<u64>().ok()?;
    let end = end.trim();
    if end.is_empty() {
        Some((start, None))
    } else {
        Some((start, end.parse::<u64>().ok()))
    }
}

/// Extracts a query parameter from a URL.
fn url_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some((name, value)) = pair.split_once('=') {
            if name == key {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Minimal percent-decoder for URL query values.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(hex) = hex {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    index += 3;
                    continue;
                }
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
