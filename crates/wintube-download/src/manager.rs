//! yt-dlp sidecar manager.
//! Downloads 8K down to 144p via format strings like SmartTube presets:
//!   bestvideo[height<=2160][vcodec^=avc]  -> 4K (H.264, remuxed in Rust)
//!   bestvideo[height<=4320][vcodec^=avc]  -> 8K (H.264, remuxed in Rust)
//!
//! H.264 + AAC downloads need no ffmpeg: yt-dlp fetches the video-only and
//! audio-only MP4s separately and `wintube-mux` merges them. AV1/VP9 requests
//! still fall back to yt-dlp's ffmpeg merge.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;
use wintube_core::{CodecPref, DownloadJob, JobStatus, Quality};

pub fn sidecar_dir() -> PathBuf {
    if let Some(exe) = std::env::current_exe().ok() {
        if let Some(p) = exe.parent() {
            return p.join("sidecars");
        }
    }
    PathBuf::from("sidecars")
}

/// Candidate folders that may hold the sidecar binaries, in priority order:
/// next to the running exe (installed/built layout), the current directory,
/// and the source tree (dev builds, whose exe lives under `target/`).
fn sidecar_candidates() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    dirs.push(sidecar_dir());
    if let Some(exe) = std::env::current_exe().ok() {
        if let Some(p) = exe.parent() {
            // Bundled Tauri sidecars land directly next to the executable.
            dirs.push(p.to_path_buf());
            // `target/<triple>/<profile>/../../..` is the workspace root.
            dirs.push(p.join("../../src-tauri/sidecars"));
            dirs.push(p.join("../../../src-tauri/sidecars"));
        }
    }
    dirs.push(PathBuf::from("sidecars"));
    dirs.push(PathBuf::from("src-tauri/sidecars"));
    // Common per-user install locations, in case the app was moved or the
    // sidecar was dropped somewhere else on the machine.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Wintube"));
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(roaming).join("Wintube"));
    }
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        dirs.push(PathBuf::from(program_files).join("Wintube"));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        dirs.push(PathBuf::from(program_files_x86).join("Wintube"));
    }
    // Baked in at compile time; only valid for dev builds on the build machine.
    dirs.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src-tauri/sidecars"
    )));
    dirs
}

fn find_sidecar(name: &str) -> String {
    for dir in sidecar_candidates() {
        let candidate = dir.join(name);
        if candidate.is_file() {
            // Return an absolute path: a relative one breaks as soon as the
            // working directory changes (the app is launched from the Start
            // menu with an arbitrary CWD). Avoid `canonicalize` here — on
            // Windows it returns a `\\?\` UNC-prefixed path that some tools
            // choke on.
            let abs = if candidate.is_absolute() {
                candidate
            } else {
                match std::env::current_dir() {
                    Ok(cwd) => cwd.join(candidate),
                    Err(_) => candidate,
                }
            };
            return abs.to_string_lossy().to_string();
        }
    }
    // Fall back to PATH for development.
    if cfg!(windows) {
        name.trim_end_matches(".exe").to_string()
    } else {
        name.to_string()
    }
}

pub fn ensure_sidecars() -> (String, String) {
    let yt = find_sidecar(if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" });
    let ff = find_sidecar(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" });
    (yt, ff)
}

/// Whether this job can be merged by the pure-Rust remuxer.
fn uses_rust_remux(codec: CodecPref) -> bool {
    matches!(codec, CodecPref::Avc | CodecPref::Auto)
}

/// Append to the same `%APPDATA%\Wintube\wintube.log` the app shell uses.
/// Downloads run in a detached task, so this file is the only way to see what
/// yt-dlp actually did after the fact.
fn log_line(msg: &str) {
    use std::io::Write;
    let Some(base) = std::env::var_os("APPDATA") else {
        return;
    };
    let dir = PathBuf::from(base).join("Wintube");
    let _ = std::fs::create_dir_all(&dir);
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("wintube.log"))
    else {
        return;
    };
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(f, "[{ts}] [download] {msg}");
}

/// Video output template. `%(height)s` followed by a literal `p` — the old
/// `%(height)p` is not a valid conversion and made yt-dlp reject the whole
/// template with `unsupported format character 'p'`.
fn video_template(dir: &str) -> String {
    format!("{dir}/%(title).120s [%(id)s] %(height)sp.%(ext)s")
}

/// Audio output template: no `%(height)s`, audio-only formats have no height.
fn audio_template(dir: &str) -> String {
    format!("{dir}/%(title).120s [%(id)s] audio.%(ext)s")
}

/// Effective download directory: an empty setting means "the user's Downloads
/// folder", never the drive root.
pub fn effective_dir(dir: &str) -> String {
    let trimmed = dir.trim();
    if trimmed.is_empty() {
        return dirs_downloads();
    }
    trimmed.replace('\\', "/")
}

/// `%USERPROFILE%\Downloads`, without pulling in a platform crate.
pub fn dirs_downloads() -> String {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let p = PathBuf::from(profile).join("Downloads");
        return p.to_string_lossy().replace('\\', "/");
    }
    ".".to_string()
}

#[derive(Clone, Default)]
pub struct DownloadManager {
    pub jobs: Arc<Mutex<HashMap<Uuid, DownloadJob>>>,
}

impl DownloadManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> Vec<DownloadJob> {
        let m = self.jobs.lock().unwrap();
        let mut v: Vec<_> = m.values().cloned().collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }


    /// Start a background yt-dlp job. `on_event` is called with progress updates
    /// (the Tauri layer forwards these to the frontend). Returns the job id.
    pub async fn start<F>(
        &self,
        video_id: &str,
        title: &str,
        quality: Quality,
        codec: CodecPref,
        out_dir: &str,
        on_event: F,
    ) -> Result<Uuid, wintube_core::CoreError>
    where
        F: Fn(DownloadJob) + Send + Sync + 'static,
    {
        let rust_remux = uses_rust_remux(codec);
        let format_string = if rust_remux {
            quality.ytdlp_video_avc()
        } else {
            quality.ytdlp_best(codec)
        };
        let id = Uuid::new_v4();
        let job = DownloadJob {
            id,
            video_id: video_id.into(),
            title: title.into(),
            quality_label: quality.label().into(),
            format_string: format_string.clone(),
            status: JobStatus::Pending,
            progress: 0.0,
            speed_bps: None,
            eta_secs: None,
            output_path: None,
            error: None,
            created_at: chrono::Utc::now(),
        };
        self.jobs.lock().unwrap().insert(id, job.clone());
        on_event(job);

        let (ytdlp, _ffmpeg) = ensure_sidecars();
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        // Never let an empty setting turn the template into an absolute path
        // at the drive root (`/%(title)...`).
        let out_dir = effective_dir(out_dir);
        let _ = std::fs::create_dir_all(&out_dir);
        let tmpl = video_template(&out_dir);
        let audio_tmpl = audio_template(&out_dir);
        let jobs = self.jobs.clone();
        let on_event = Arc::new(on_event);
        log_line(&format!(
            "start id={id} video={video_id} quality={:?} codec={codec:?} rust_remux={rust_remux} dir={out_dir} sidecar={ytdlp}",
            quality
        ));
        // Own the id so the spawned task can use it for destination recovery.
        let video_id = video_id.to_string();

        tokio::spawn(async move {
            let result = if rust_remux {
                download_and_remux(
                    &ytdlp,
                    &url,
                    &quality.ytdlp_video_avc(),
                    quality.ytdlp_audio_m4a(),
                    &tmpl,
                    &audio_tmpl,
                    &out_dir,
                    &video_id,
                    &jobs,
                    id,
                    &on_event,
                )
                .await
            } else {
                download_merged(
                    &ytdlp,
                    &url,
                    &format_string,
                    &tmpl,
                    &out_dir,
                    &video_id,
                    &jobs,
                    id,
                    &on_event,
                )
                .await
            };

            let mut jobs = jobs.lock().unwrap();
            if let Some(job) = jobs.get_mut(&id) {
                match result {
                    Ok(output_path) => {
                        job.status = JobStatus::Done;
                        job.progress = 1.0;
                        job.output_path = Some(output_path);
                    }
                    Err(error) => {
                        job.status = JobStatus::Failed;
                        job.error = Some(error);
                    }
                }
                let job = job.clone();
                drop(jobs);
                on_event(job);
            }
        });

        Ok(id)
    }

    pub fn cancel(&self, id: Uuid) {
        if let Some(j) = self.jobs.lock().unwrap().get_mut(&id) {
            j.status = JobStatus::Cancelled;
        }
    }

    /// Pure-Rust remux path: yt-dlp downloads video-only + audio-only MP4s,
    /// `wintube-mux` merges them — no ffmpeg needed.
    /// `video_url`/`audio_url` must be direct MP4 track URLs (H.264/AAC).
    /// Pass `0` for `width`/`height`/`fps` to read them from the container.
    pub async fn remux(
        &self,
        video_mp4: &str,
        audio_mp4: &str,
        output_mp4: &str,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Result<wintube_mux::RemuxInfo, wintube_core::CoreError> {
        let v = video_mp4.to_string();
        let a = audio_mp4.to_string();
        let o = output_mp4.to_string();
        let r = tokio::task::spawn_blocking(move || -> Result<wintube_mux::RemuxInfo, String> {
            wintube_mux::remux_mp4_tracks(&v, &a, &o, width, height, fps).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| wintube_core::CoreError::Internal(e.to_string()))?;
        r.map_err(wintube_core::CoreError::Internal)
    }
}

/// Update one job, then emit it *after* releasing the jobs mutex. The event
/// callback forwards to Tauri, and holding the lock across it invites deadlocks.
fn set_job<F>(
    jobs: &Arc<Mutex<HashMap<Uuid, DownloadJob>>>,
    id: Uuid,
    on_event: &F,
    update: impl FnOnce(&mut DownloadJob),
) where
    F: Fn(DownloadJob),
{
    let mut guard = jobs.lock().unwrap();
    let Some(job) = guard.get_mut(&id) else {
        return;
    };
    update(job);
    let job = job.clone();
    drop(guard);
    on_event(job);
}

/// yt-dlp prints `[download] Destination:` on stdout, but on Windows its
/// console encoding can silently drop non-ANSI title characters. yt-dlp maps
/// `|` to the fullwidth `｜` (U+FF5C) in filenames, so a title containing a
/// pipe produced destinations that do not exist even though the files did.
/// `--encoding utf-8` fixes the output; this scan is the safety net (and also
/// recovers yt-dlp's merged output, which is only announced as `[Merger]`).
fn resolve_downloads(reported: Vec<String>, out_dir: &str, video_id: &str) -> Vec<String> {
    let tag = format!("[{video_id}]");
    let mut found: Vec<String> = reported
        .into_iter()
        .filter(|path| std::path::Path::new(path).is_file())
        .collect();

    let mut scanned: Vec<(std::time::SystemTime, String)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(out_dir) {
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.contains(&tag) || name.ends_with(".part") {
                continue;
            }
            scanned.push((
                meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                path.to_string_lossy().to_string(),
            ));
        }
    }
    // Newest first: a retry leaves older files behind, and the newest one is
    // the download that just finished.
    scanned.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in scanned {
        if !found.iter().any(|p| same_path(p, &path)) {
            found.push(path);
        }
    }
    found
}

/// Windows paths are case-insensitive and the two sources use different
/// separators (`%TEMP%` templates use `/`, yt-dlp prints `\`), so compare
/// normalised forms when deduplicating.
fn same_path(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.replace('\\', "/").eq_ignore_ascii_case(&b.replace('\\', "/"))
    } else {
        a == b
    }
}

/// Runs yt-dlp once, streaming progress into the job and collecting the
/// destination file paths it reports.
///
/// stderr is drained concurrently: yt-dlp can be chatty on failures, and a
/// full pipe buffer would otherwise deadlock the child while we wait on it.
/// The tail is kept for the error message and the app log.
#[allow(clippy::too_many_arguments)]
async fn run_ytdlp<F>(
    ytdlp: &str,
    args: &[&str],
    out_dir: &str,
    video_id: &str,
    jobs: &Arc<Mutex<HashMap<Uuid, DownloadJob>>>,
    id: Uuid,
    on_event: &Arc<F>,
    progress_range: (f64, f64),
) -> Result<Vec<String>, String>
where
    F: Fn(DownloadJob) + Send + Sync + 'static,
{
    log_line(&format!("run {ytdlp} {}", args.join(" ")));
    let mut child = Command::new(ytdlp)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("yt-dlp could not be started ({ytdlp}): {e}"))?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");

    // Bound the tail to the last N lines; enough for any real error message.
    let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let tail = stderr_tail.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut buf = tail.lock().unwrap();
            buf.push_back(line);
            while buf.len() > 40 {
                buf.pop_front();
            }
        }
    });

    let mut lines = BufReader::new(stdout).lines();
    let mut destinations = Vec::new();
    let mut last_pct = -1f64;

    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(path) = line.strip_prefix("[download] Destination: ") {
            let path = path.trim().to_string();
            log_line(&format!("destination {path}"));
            destinations.push(path);
            set_job(jobs, id, on_event.as_ref(), |job| {
                job.status = JobStatus::Downloading;
            });
            continue;
        }
        if let Some(p) = parse_percent(&line) {
            if (p - last_pct).abs() > 1.0 {
                last_pct = p;
                let mapped = progress_range.0 + (progress_range.1 - progress_range.0) * p / 100.0;
                let speed = parse_speed(&line);
                let eta = parse_eta(&line);
                set_job(jobs, id, on_event.as_ref(), |job| {
                    job.status = JobStatus::Downloading;
                    job.progress = mapped;
                    job.speed_bps = speed;
                    job.eta_secs = eta;
                });
            }
        }
        if line.contains("[Merger]") || line.contains("Merging") {
            set_job(jobs, id, on_event.as_ref(), |job| {
                job.status = JobStatus::Merging;
            });
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("failed waiting for yt-dlp: {e}"))?;
    let _ = stderr_task.await;
    let tail = {
        let buf = stderr_tail.lock().unwrap();
        buf.iter().cloned().collect::<Vec<_>>().join("\n")
    };
    let reported = destinations.len();
    let destinations = resolve_downloads(destinations, out_dir, video_id);
    if destinations.len() != reported {
        log_line(&format!(
            "resolved {reported} reported destination(s) to {} file(s) on disk",
            destinations.len()
        ));
        for path in &destinations {
            log_line(&format!("  file {path}"));
        }
    }
    log_line(&format!(
        "exit {status} destinations={} stderr_tail={}",
        destinations.len(),
        tail.replace('\n', " | ")
    ));
    if !status.success() {
        // Last few lines usually contain yt-dlp's one-line `ERROR:` reason.
        let detail = tail
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(if detail.is_empty() {
            format!("yt-dlp exited with {status}")
        } else {
            format!("yt-dlp exited with {status}: {detail}")
        });
    }
    Ok(destinations)
}

/// Downloads separate H.264 video-only and AAC audio-only files with yt-dlp,
/// then merges them with `wintube-mux`. No ffmpeg involved.
#[allow(clippy::too_many_arguments)]
async fn download_and_remux<F>(
    ytdlp: &str,
    url: &str,
    video_format: &str,
    audio_format: &str,
    tmpl: &str,
    audio_tmpl: &str,
    out_dir: &str,
    video_id: &str,
    jobs: &Arc<Mutex<HashMap<Uuid, DownloadJob>>>,
    id: Uuid,
    on_event: &Arc<F>,
) -> Result<String, String>
where
    F: Fn(DownloadJob) + Send + Sync + 'static,
{
    let video_files = run_ytdlp(
        ytdlp,
        &[
            "-f",
            video_format,
            "-o",
            tmpl,
            "--newline",
            "--no-warnings",
            "--encoding",
            "utf-8",
            "--socket-timeout",
            "30",
            url,
        ],
        out_dir,
        video_id,
        jobs,
        id,
        on_event,
        (0.0, 0.85),
    )
    .await?;
    let video_path = video_files
        .into_iter()
        .find(|path| {
            let lower = path.to_ascii_lowercase();
            lower.ends_with(".mp4") && !lower.ends_with(".merged.mp4")
        })
        .ok_or("yt-dlp did not report a video file")?;

    let audio_files = run_ytdlp(
        ytdlp,
        &[
            "-f",
            audio_format,
            "-o",
            audio_tmpl,
            "--newline",
            "--no-warnings",
            "--encoding",
            "utf-8",
            "--socket-timeout",
            "30",
            url,
        ],
        out_dir,
        video_id,
        jobs,
        id,
        on_event,
        (0.85, 0.97),
    )
    .await?;
    let audio_path = audio_files
        .into_iter()
        .find(|path| path.to_ascii_lowercase().ends_with(".m4a"))
        .ok_or("yt-dlp did not report an audio file")?;

    {
        let mut jobs = jobs.lock().unwrap();
        if let Some(job) = jobs.get_mut(&id) {
            job.status = JobStatus::Merging;
            let job = job.clone();
            drop(jobs);
            on_event(job);
        }
    }

    let video = std::path::Path::new(&video_path);
    let output_path = video
        .with_extension("")
        .to_string_lossy()
        .to_string()
        + ".merged.mp4";

    let v = video_path.clone();
    let a = audio_path.clone();
    let o = output_path.clone();
    let info = tokio::task::spawn_blocking(move || {
        wintube_mux::remux_mp4_tracks(&v, &a, &o, 0, 0, 0.0).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("remux task failed: {e}"))?
    .map_err(|e| format!("remux failed: {e}"))?;
    log_line(&format!(
        "remux ok audio_fragmented={} video_samples={} audio_samples={} bytes={}",
        info.audio_fragmented, info.video_samples, info.audio_samples, info.output_bytes
    ));

    // The merged file is the deliverable; the separate tracks are intermediate.
    let _ = std::fs::remove_file(&video_path);
    let _ = std::fs::remove_file(&audio_path);

    Ok(output_path)
}

/// Downloads with yt-dlp's own (ffmpeg-based) merge. Used for AV1/VP9, which
/// `wintube-mux` does not handle yet.
#[allow(clippy::too_many_arguments)]
async fn download_merged<F>(
    ytdlp: &str,
    url: &str,
    format_string: &str,
    tmpl: &str,
    out_dir: &str,
    video_id: &str,
    jobs: &Arc<Mutex<HashMap<Uuid, DownloadJob>>>,
    id: Uuid,
    on_event: &Arc<F>,
) -> Result<String, String>
where
    F: Fn(DownloadJob) + Send + Sync + 'static,
{
    let files = run_ytdlp(
        ytdlp,
        &[
            "-f",
            format_string,
            "--merge-output-format",
            "mp4",
            "-o",
            tmpl,
            "--newline",
            "--no-warnings",
            "--encoding",
            "utf-8",
            "--socket-timeout",
            "30",
            url,
        ],
        out_dir,
        video_id,
        jobs,
        id,
        on_event,
        (0.0, 1.0),
    )
    .await?;
    // The two tracks yt-dlp announces with `Destination:` are deleted after
    // its own merge; the deliverable is the `[Merger]` output recovered by
    // `resolve_downloads`. Newest first, so prefer an `.mp4` from the front.
    files
        .iter()
        .find(|path| path.to_ascii_lowercase().ends_with(".mp4"))
        .cloned()
        .or_else(|| files.into_iter().next())
        .ok_or_else(|| "yt-dlp did not report an output file".to_string())
}

fn parse_percent(line: &str) -> Option<f64> {
    if !line.contains('%') {
        return None;
    }
    let pct = line.split('%').next()?.split_whitespace().last()?;
    pct.parse().ok()
}
fn parse_speed(line: &str) -> Option<u64> {
    // "... at  3.45MiB/s"
    let i = line.find(" at ")?;
    let rest = &line[i + 4..];
    let n: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let v: f64 = n.parse().ok()?;
    if rest.contains("GiB/s") {
        Some((v * 1e9) as u64)
    } else if rest.contains("MiB/s") {
        Some((v * 1e6) as u64)
    } else if rest.contains("KiB/s") {
        Some((v * 1e3) as u64)
    } else {
        None
    }
}
fn parse_eta(line: &str) -> Option<u64> {
    let i = line.find("ETA ")?;
    let rest = &line[i + 4..];
    let mut secs = 0u64;
    for part in rest.split(':') {
        if let Ok(n) = part.trim().parse::<u64>() {
            secs = secs * 60 + n;
        }
    }
    Some(secs)
}

