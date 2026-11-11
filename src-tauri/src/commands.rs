use crate::AppState;
use tauri::{Emitter, State};
use wintube_core::{CodecPref, PlayInfo, Quality};
use wintube_yt::YtClient;

#[tauri::command]
pub async fn get_play_info(video_id: String) -> Result<PlayInfo, String> {
    let (yt, _ff) = wintube_download::ensure_sidecars();
    crate::log_line(&format!("get_play_info id={video_id} sidecar={yt}"));
    let info = YtClient::new(yt)
        .get_play_info(&video_id)
        .await
        .map_err(|e| e.to_string())?;
    crate::log_line(&format!(
        "get_play_info id={video_id} formats={} progressive={} dash={} hls={}",
        info.formats.len(),
        info.best_progressive_url.is_some(),
        info.dash_url.is_some(),
        info.hls_url.is_some()
    ));
    Ok(info)
}

#[derive(serde::Serialize)]
pub struct SearchResult {
    pub cards: Vec<wintube_core::VideoCard>,
    pub continuation: Option<String>,
}

#[tauri::command]
pub async fn get_playback_streams(
    video_id: String,
) -> Result<wintube_core::PlaybackStreams, String> {
    let started = std::time::Instant::now();
    // InnerTube first: the VISIONOS client still exposes direct adaptive URLs
    // for videos where yt-dlp only sees the progressive 360p stream (the
    // SABR-only experiment). yt-dlp stays as the fallback.
    let streams = match wintube_yt::innertube::playback_streams(&video_id).await {
        Ok(s) => {
            crate::log_line(&format!(
                "get_playback_streams id={video_id} source=innertube steps={} audio={} elapsed={}ms",
                s.video.len(),
                s.audio_url.is_some(),
                started.elapsed().as_millis()
            ));
            s
        }
        Err(e) => {
            crate::log_line(&format!(
                "get_playback_streams id={video_id} innertube failed ({e}); falling back to yt-dlp"
            ));
            let (yt, _ff) = wintube_download::ensure_sidecars();
            YtClient::new(yt)
                .get_playback_streams(&video_id)
                .await
                .map_err(|e| e.to_string())?
        }
    };
    let streams = crate::relay::rewrite(streams);
    crate::log_line(&format!(
        "get_playback_streams id={video_id} steps={} audio={}",
        streams.video.len(),
        streams.audio_url.is_some()
    ));
    Ok(streams)
}

#[tauri::command]
pub async fn log_line(message: String) {
    crate::log_line(&message);
}

#[tauri::command]
pub async fn search_videos(query: String, continuation: Option<String>) -> Result<SearchResult, String> {
    let started = std::time::Instant::now();
    // Fast path: the InnerTube JSON API. No subprocess, no sidecar, ~10x
    // quicker than yt-dlp's flat search.
    match wintube_yt::innertube::search_page(&query, continuation.as_deref()).await {
        Ok(mut page) => {
            crate::log_line(&format!(
                "search query={query:?} source=innertube page cards={} more={} elapsed={}ms",
                page.cards.len(),
                page.continuation.is_some(),
                started.elapsed().as_millis()
            ));
            inline_thumbnails(&mut page.cards).await;
            return Ok(SearchResult {
                cards: page.cards,
                continuation: page.continuation,
            });
        }
        Err(e) => crate::log_line(&format!(
            "search query={query:?} source=innertube failed ({e}); falling back to yt-dlp"
        )),
    }

    if continuation.is_some() {
        // Continuation tokens are InnerTube-only; the yt-dlp fallback below
        // always returns the first page, which would corrupt paging.
        return Err("Search failed (no more pages available)".to_string());
    }

    // Fallback: yt-dlp flat search.
    // Cheap, reliable path: yt-dlp flat search. Swap for innertube-rs
    // /search once its API locks (see crates/wintube-yt/src/innertube.rs).
    let (yt, _ff) = wintube_download::ensure_sidecars();
    let exists = std::path::Path::new(&yt).is_file();
    crate::log_line(&format!(
        "search_videos query={query:?} sidecar={yt} exists={exists}"
    ));
    if !exists {
        return Err(format!(
            "yt-dlp.exe was not found. Wintube looked for it next to the app \
             (and in {yt}). Reinstall Wintube so the bundled sidecar is restored."
        ));
    }
    let url = format!("ytsearch20:{query}");
    // `-J` dumps a single JSON object with an `entries` array; `-j` would
    // print one object per line (NDJSON) and fail to parse as one document.
    let mut last_err = String::new();
    for attempt in 0..2 {
        let out = match tokio::process::Command::new(&yt)
            .args([
                "-J",
                "--flat-playlist",
                "--no-warnings",
                "--socket-timeout",
                "30",
                &url,
            ])
            .output()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                last_err = format!("could not start yt-dlp ({yt}): {e}");
                crate::log_line(&format!("search attempt={attempt} spawn_error={e}"));
                continue;
            }
        };
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        crate::log_line(&format!(
            "search attempt={attempt} status={:?} stdout_len={} stderr={stderr}",
            out.status,
            out.stdout.len()
        ));
        if !out.status.success() {
            last_err = if stderr.is_empty() {
                format!("yt-dlp exited with {}", out.status)
            } else {
                stderr.clone()
            };
            continue;
        }
        let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
            Ok(v) => v,
            Err(e) => {
                last_err = format!("yt-dlp returned invalid JSON: {e}");
                continue;
            }
        };
        let entries = v
            .get("entries")
            .and_then(|e| e.as_array())
            .cloned()
            .unwrap_or_default();
        // The yt-dlp fallback has no paging: return one page of 24.
        const PAGE_SIZE: usize = 24;
        crate::log_line(&format!("search entries={}", entries.len()));
        let mut cards = Vec::new();
        for e in entries.iter().take(PAGE_SIZE) {
            let id = e.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if id.is_empty() {
                continue;
            }
            let title = e.get("title").and_then(|x| x.as_str()).unwrap_or("Untitled").to_string();
            let channel = e
                .get("uploader")
                .or_else(|| e.get("channel"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let thumb = pick_thumbnail(e, &id);
            let dur = e.get("duration").and_then(|x| x.as_f64()).map(|d| d as u64);
            cards.push(wintube_core::VideoCard {
                id,
                title,
                channel,
                views: None,
                age: None,
                duration: dur,
                thumbnail: thumb,
            });
        }
        crate::log_line(&format!(
            "search cards={} source=ytdlp elapsed={}ms",
            cards.len(),
            started.elapsed().as_millis()
        ));
        inline_thumbnails(&mut cards).await;
        return Ok(SearchResult {
            cards,
            continuation: None,
        });
    }
    Err(if last_err.is_empty() {
        "yt-dlp returned no output".to_string()
    } else {
        last_err
    })
}

/// Pick the best thumbnail URL for a search entry.
///
/// Flat search entries expose `thumbnails`: an array of `{url,width,height}`
/// objects (the singular `thumbnail` key is only present on full video
/// dumps, and may be `null`). Pick the widest entry, then fall back to the
/// canonical 16:9 still, which exists for every video.
fn pick_thumbnail(entry: &serde_json::Value, id: &str) -> String {
    if let Some(arr) = entry.get("thumbnails").and_then(|t| t.as_array()) {
        let mut best: Option<(u64, String)> = None;
        for t in arr {
            let Some(url) = t.get("url").and_then(|u| u.as_str()) else {
                continue;
            };
            if url.is_empty() {
                continue;
            }
            let width = t.get("width").and_then(|w| w.as_u64()).unwrap_or(0);
            if best.as_ref().map(|(bw, _)| width > *bw).unwrap_or(true) {
                best = Some((width, url.to_string()));
            }
        }
        if let Some((_, url)) = best {
            return url;
        }
    }
    if let Some(url) = entry.get("thumbnail").and_then(|t| t.as_str()) {
        if !url.is_empty() {
            return url.to_string();
        }
    }
    format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")
}

/// WebView2 cannot reliably load `i.ytimg.com` directly (flaky DNS here, and
/// it serialises image loads), so fetch the thumbnails in Rust and hand the
/// frontend `data:` URIs instead. Failures keep the original URL.
async fn inline_thumbnails(cards: &mut [wintube_core::VideoCard]) {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            crate::log_line(&format!("thumbnail client build failed: {e}"));
            return;
        }
    };
    let mut handles = Vec::new();
    for (index, card) in cards.iter().enumerate() {
        if !card.thumbnail.starts_with("http") {
            continue;
        }
        let client = client.clone();
        let url = card.thumbnail.clone();
        handles.push((
            index,
            tokio::spawn(async move {
                // DNS for i.ytimg.com is flaky on this machine, so retry once.
                for attempt in 0..2 {
                    match client.get(&url).send().await {
                        Ok(resp) if resp.status().is_success() => {
                            let bytes = resp.bytes().await.ok()?;
                            if bytes.is_empty() || bytes.len() > 1024 * 1024 {
                                return None;
                            }
                            return Some(format!("data:image/jpeg;base64,{}", base64_encode(&bytes)));
                        }
                        Ok(resp) => {
                            crate::log_line(&format!(
                                "thumbnail {url} -> HTTP {}",
                                resp.status()
                            ));
                            return None;
                        }
                        Err(e) => {
                            crate::log_line(&format!("thumbnail {url} attempt {attempt}: {e}"));
                            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        }
                    }
                }
                None
            }),
        ));
    }
    let mut ok = 0usize;
    for (index, handle) in handles {
        if let Ok(Some(data_uri)) = handle.await {
            cards[index].thumbnail = data_uri;
            ok += 1;
        }
    }
    crate::log_line(&format!("thumbnails inlined {ok}/{}", cards.len()));
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        out.push(CHARS[(b[0] >> 2) as usize] as char);
        out.push(CHARS[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(b[2] & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[tauri::command]
pub fn get_history(state: State<AppState>, limit: i64) -> Result<Vec<wintube_core::HistoryEntry>, String> {
    let g = state.db.lock().unwrap();
    g.as_ref().map(|d| d.load_history(limit).map_err(|e| e.to_string())).transpose().map(|v| v.unwrap_or_default()).map_err(|e| e)
}

#[tauri::command]
pub fn save_history(state: State<AppState>, entry: wintube_core::HistoryEntry) -> Result<(), String> {
    let g = state.db.lock().unwrap();
    if let Some(d) = g.as_ref() { d.save_history(&entry).map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> wintube_core::AppSettings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: wintube_core::AppSettings) -> Result<(), String> {
    *state.settings.lock().unwrap() = settings.clone();
    let g = state.db.lock().unwrap();
    if let Some(d) = g.as_ref() { d.save_settings(&settings).map_err(|e| e.to_string())?; }
    Ok(())
}

#[tauri::command]
pub async fn start_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    video_id: String,
    title: String,
    quality: String,
    codec: String,
) -> Result<wintube_core::DownloadJob, String> {
    // An empty setting must not become the drive root; fall back to the
    // user's Downloads folder and fail loudly if it cannot be created.
    let out_dir = {
        let configured = state.settings.lock().unwrap().download_dir.clone();
        let dir = wintube_download::effective_dir(&configured);
        crate::log_line(&format!(
            "start_download id={video_id} dir={dir} quality={quality} codec={codec}"
        ));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            crate::log_line(&format!("start_download create_dir_all failed dir={dir}: {e}"));
            return Err(format!("cannot create download folder {dir}: {e}"));
        }
        dir
    };
    let q = Quality::from_str(&quality);
    let c = CodecPref::from_str(&codec);
    let mgr = state.downloads.clone();
    let app2 = app.clone();
    let id = mgr.start(&video_id, &title, q, c, &out_dir, move |job| {
        let _ = app2.emit("download://progress", job);
    }).await.map_err(|e| {
        crate::log_line(&format!("start_download failed id={video_id}: {e}"));
        e.to_string()
    })?;
    mgr.list().into_iter().find(|j| j.id == id).ok_or_else(|| "job not found".into())
}

#[tauri::command]
pub fn list_downloads(state: State<AppState>) -> Vec<wintube_core::DownloadJob> {
    state.downloads.list()
}

#[tauri::command]
pub fn cancel_download(state: State<AppState>, id: String) -> Result<(), String> {
    if let Ok(u) = id.parse::<uuid::Uuid>() { state.downloads.cancel(u); }
    Ok(())
}

#[tauri::command]
pub fn ensure_sidecars() -> serde_json::Value {
    let (yt, ff) = wintube_download::ensure_sidecars();
    serde_json::json!({"yt_dlp": yt, "ffmpeg": ff})
}

/// Everything needed to explain a broken sidecar setup from the Settings page:
/// which binary was resolved, whether it exists, its version, and whether a
/// throwaway search actually succeeds.
#[derive(serde::Serialize)]
pub struct SidecarDiagnostics {
    yt_dlp: String,
    yt_dlp_found: bool,
    yt_dlp_version: Option<String>,
    ffmpeg: String,
    ffmpeg_found: bool,
    exe_dir: String,
    log_file: String,
    probe: Option<String>,
    probe_entries: Option<usize>,
}

#[tauri::command]
pub async fn sidecar_diagnostics() -> SidecarDiagnostics {
    let (yt, ff) = wintube_download::ensure_sidecars();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_string_lossy().to_string()))
        .unwrap_or_default();
    let log_file = dirs::data_dir()
        .map(|p| p.join("Wintube").join("wintube.log").to_string_lossy().to_string())
        .unwrap_or_default();
    let mut yt_dlp_version = None;
    let mut probe = None;
    let mut probe_entries = None;
    if std::path::Path::new(&yt).is_file() {
        if let Ok(out) = tokio::process::Command::new(&yt).arg("--version").output().await {
            yt_dlp_version = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
        }
        if let Ok(out) = tokio::process::Command::new(&yt)
            .args([
                "-J",
                "--flat-playlist",
                "--no-warnings",
                "--socket-timeout",
                "20",
                "ytsearch1:test",
            ])
            .output()
            .await
        {
            if out.status.success() {
                match serde_json::from_slice::<serde_json::Value>(&out.stdout) {
                    Ok(v) => {
                        probe_entries = v
                            .get("entries")
                            .and_then(|e| e.as_array())
                            .map(|a| a.len());
                        probe = Some("ok".to_string());
                    }
                    Err(e) => probe = Some(format!("invalid JSON: {e}")),
                }
            } else {
                probe = Some(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
        }
    }
    crate::log_line(&format!(
        "diagnostics yt={yt} found={} version={yt_dlp_version:?} probe={probe:?}",
        std::path::Path::new(&yt).is_file()
    ));
    SidecarDiagnostics {
        yt_dlp_found: std::path::Path::new(&yt).is_file(),
        yt_dlp: yt,
        yt_dlp_version,
        ffmpeg_found: std::path::Path::new(&ff).is_file(),
        ffmpeg: ff,
        exe_dir,
        log_file,
        probe,
        probe_entries,
    }
}

#[tauri::command]
pub async fn remux_download(
    video_mp4: String,
    audio_mp4: String,
    output_mp4: String,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<wintube_mux::RemuxInfo, String> {
    wintube_mux::remux_mp4_tracks(&video_mp4, &audio_mp4, &output_mp4, width, height, fps)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_sponsor_segments(video_id: String) -> Result<Vec<serde_json::Value>, String> {
    // SponsorBlock API — same integration as SmartTube's DeArrow/SponsorBlock.
    let url = format!("https://sponsor.ajay.app/api/skipSegments?videoID={video_id}&category=sponsor&category=intro&category=outro");
    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
        Ok(v.as_array().cloned().unwrap_or_default())
    } else { Ok(Vec::new()) }
}
