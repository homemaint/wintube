use serde::Deserialize;
use wintube_core::{Format, PlayInfo, Subtitle, Video};

#[derive(Debug, Deserialize)]
struct YtDlpOut {
    id: String,
    title: String,
    channel: Option<String>,
    channel_id: Option<String>,
    uploader: Option<String>,
    description: Option<String>,
    // Fractional seconds occur for some videos; truncate on the way out.
    duration: Option<f64>,
    view_count: Option<u64>,
    like_count: Option<u64>,
    thumbnail: Option<String>,
    upload_date: Option<String>,
    is_live: Option<bool>,
    tags: Option<Vec<String>>,
    categories: Option<Vec<String>>,
    subtitles: Option<serde_json::Value>,
    automatic_captions: Option<serde_json::Value>,
    formats: Vec<YtFormat>,
    dash_manifest_url: Option<String>,
    manifest_url: Option<String>,
    hls_manifest_url: Option<String>,
    storyboard_spec: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone)]
struct YtFormat {
    format_id: String,
    url: String,
    ext: String,
    vcodec: Option<String>,
    acodec: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    // yt-dlp reports fractional fps for storyboard formats (e.g. 0.507),
    // so this must stay a float.
    fps: Option<f64>,
    tbr: Option<f64>,
    filesize: Option<u64>,
    filesize_approx: Option<u64>,
    container: Option<String>,
    video_ext: Option<String>,
    audio_ext: Option<String>,
    is_dash_period: Option<bool>,
    http_headers: Option<serde_json::Value>,
    protocol: Option<String>,
    quality_label: Option<String>,
    format_note: Option<String>,
}

fn parse_caps(v: Option<&serde_json::Value>) -> Vec<Subtitle> {
    let mut out = Vec::new();
    if let Some(serde_json::Value::Object(map)) = v {
        for (lang, tracks) in map {
            if let serde_json::Value::Array(arr) = tracks {
                for t in arr {
                    if let Some(u) = t.get("url").and_then(|x| x.as_str()) {
                        let ext = t.get("ext").and_then(|x| x.as_str()).unwrap_or("vtt").to_string();
                        out.push(Subtitle { lang: lang.clone(), label: lang.clone(), ext, url: u.to_string() });
                    }
                }
            }
        }
    }
    out
}

impl From<YtDlpOut> for PlayInfo {
    fn from(o: YtDlpOut) -> Self {
        let video = Video {
            id: o.id.clone(),
            title: o.title.clone(),
            channel: o.channel.clone().or(o.uploader).unwrap_or_default(),
            channel_id: o.channel_id,
            channel_avatar: None,
            description: o.description,
            duration: o.duration.map(|d| d as u64),
            view_count: o.view_count,
            like_count: o.like_count,
            thumbnail: o.thumbnail.clone().unwrap_or_default(),
            thumbnail_hq: o.thumbnail.clone(),
            upload_date: o.upload_date,
            is_live: o.is_live.unwrap_or(false),
            is_upcoming: false,
            tags: o.tags.unwrap_or_default(),
            subtitles: { let mut s = parse_caps(o.subtitles.as_ref()); s.extend(parse_caps(o.automatic_captions.as_ref())); s },
            categories: o.categories.unwrap_or_default(),
        };
        let mut formats: Vec<Format> = o.formats.into_iter().map(|f| {
            let v = f.vcodec.as_deref().unwrap_or("none");
            let a = f.acodec.as_deref().unwrap_or("none");
            let is_vo = v != "none" && a == "none";
            let is_ao = a != "none" && v == "none";
            Format {
                format_id: f.format_id, url: f.url, ext: f.ext,
                codec_video: if v!="none"{Some(v.to_string())}else{None},
                codec_audio: if a!="none"{Some(a.to_string())}else{None},
                width: f.width, height: f.height, fps: f.fps,
                vcodec: f.vcodec, acodec: f.acodec, tbr: f.tbr,
                filesize: f.filesize, filesize_approx: f.filesize_approx,
                container: f.container,
                is_video_only: is_vo, is_audio_only: is_ao,
                is_progressive: !is_vo && !is_ao,
                is_hdr: f.format_note.as_deref().map(|n| n.contains("HDR")).unwrap_or(false),
                http_headers: f.http_headers, protocol: f.protocol,
                quality_label: f.quality_label,
            }
        }).collect();
        // best-first for picker
        formats.sort_by(|a,b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));
        let best_prog = formats.iter().find(|f| f.is_progressive).map(|f| f.url.clone());
        PlayInfo { video, formats, best_progressive_url: best_prog, dash_url: o.dash_manifest_url, hls_url: o.hls_manifest_url.or(o.manifest_url), storyboard_url: None }
    }
}

pub struct YtClient {
    pub ytdlp_path: String,
}

impl Default for YtClient {
    fn default() -> Self { Self { ytdlp_path: "yt-dlp".into() } }
}

impl YtClient {
    pub fn new(ytdlp_path: impl Into<String>) -> Self { Self { ytdlp_path: ytdlp_path.into() } }

    /// Resolve full PlayInfo (formats 8K→144p, DASH/HLS, subtitles) via yt-dlp -j.
    /// This replaces SmartTube's InnertubeService+Player.decipher pipeline.
    ///
    /// The `android` player client is used on purpose: the default web client
    /// now returns separate video/audio-only formats with no progressive MP4,
    /// which a plain `<video>` tag cannot play. Android still serves itag 18
    /// (360p progressive). Downloads re-resolve formats independently, so
    /// they are unaffected.
    pub async fn get_play_info(&self, video_id: &str) -> Result<PlayInfo, wintube_core::CoreError> {
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        let out = tokio::process::Command::new(&self.ytdlp_path)
            .args(["-j","--no-playlist","--no-warnings","--socket-timeout","30",
                   "--extractor-args","youtube:player_client=android",&url])
            .output().await.map_err(|e| wintube_core::CoreError::SidecarMissing(format!("yt-dlp: {e}")))?;
        if !out.status.success() {
            return Err(wintube_core::CoreError::YtDlp(String::from_utf8_lossy(&out.stderr).to_string()));
        }
        let parsed: YtDlpOut = serde_json::from_slice(&out.stdout)?;
        Ok(parsed.into())
    }

    /// The selectable playback ladder: video-only AVC at every height plus
    /// one AAC audio stream, for synced dual-element playback. Resolved with
    /// the default client (the same one downloads use), which exposes the
    /// full ladder without a JS runtime. No merging — the frontend plays the
    /// video muted and the audio on their own clocks.
    pub async fn get_playback_streams(
        &self,
        video_id: &str,
    ) -> Result<wintube_core::PlaybackStreams, wintube_core::CoreError> {
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        let out = tokio::process::Command::new(&self.ytdlp_path)
            .args([
                "-j",
                "--no-playlist",
                "--no-warnings",
                "--socket-timeout",
                "30",
                &url,
            ])
            .output()
            .await
            .map_err(|e| wintube_core::CoreError::SidecarMissing(format!("yt-dlp: {e}")))?;
        if !out.status.success() {
            return Err(wintube_core::CoreError::YtDlp(
                String::from_utf8_lossy(&out.stderr).to_string(),
            ));
        }
        let parsed: YtDlpOut = serde_json::from_slice(&out.stdout)?;

        let mut video: Vec<wintube_core::PlaybackStep> = Vec::new();
        let mut best_tbr: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
        let mut audio_url: Option<String> = None;
        let mut best_audio_score = f64::NEG_INFINITY;

        for f in &parsed.formats {
            let v = f.vcodec.as_deref().unwrap_or("none");
            let a = f.acodec.as_deref().unwrap_or("none");
            if v != "none" && a == "none" {
                // Video-only: keep the best (highest tbr) AVC per height.
                if v.starts_with("avc") {
                    if let (Some(h), Some(w)) = (f.height, f.width) {
                        let score = f.tbr.unwrap_or(0.0);
                        let is_better = best_tbr.get(&h).map(|s| score > *s).unwrap_or(true);
                        if is_better {
                            best_tbr.insert(h, score);
                            let step = wintube_core::PlaybackStep {
                                height: h,
                                width: w,
                                url: f.url.clone(),
                            };
                            match video.iter_mut().find(|s| s.height == h) {
                                Some(slot) => *slot = step,
                                None => video.push(step),
                            }
                        }
                    }
                }
            } else if a != "none" && v == "none" {
                // Audio-only: prefer AAC/m4a, then the highest bitrate.
                let score = f.tbr.unwrap_or(0.0)
                    + if f.ext == "m4a" || a.starts_with("mp4a") { 1000.0 } else { 0.0 };
                if score > best_audio_score {
                    best_audio_score = score;
                    audio_url = Some(f.url.clone());
                }
            }
        }

        video.sort_by_key(|s| s.height);
        let duration = parsed.duration.map(|d| d as u64);
        Ok(wintube_core::PlaybackStreams {
            video,
            audio_url,
            duration,
        })
    }

    /// Flat playlist info for channel/playlist grids.
    pub async fn get_flat(&self, url: &str) -> Result<serde_json::Value, wintube_core::CoreError> {
        let out = tokio::process::Command::new(&self.ytdlp_path)
            .args(["-J","--flat-playlist","--no-warnings",url])
            .output().await.map_err(|e| wintube_core::CoreError::SidecarMissing(e.to_string()))?;
        if !out.status.success() { return Err(wintube_core::CoreError::YtDlp(String::from_utf8_lossy(&out.stderr).to_string())); }
        Ok(serde_json::from_slice(&out.stdout)?)
    }
}
