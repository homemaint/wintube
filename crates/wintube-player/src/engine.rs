use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::{debug, info, warn};

use crate::controllers::{PlaybackProgress, PlaybackState};
use crate::embedded::{self, EmbeddedSurface};
use crate::mpv_ipc::MpvIpcMonitor;
use crate::windows_os::ScreenSleepGuard;

use windows::Win32::Foundation::HWND;

const MPV_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const AUDIO_NORMALIZATION_FILTER: &str = "dynaudnorm=f=150:g=15";

/// Hardware-accelerated Media Engine for Windows PC.
/// Supports:
/// 1. **In-app embedded viewport**: native child HWND + `mpv --wid` D3D11VA
///    zero-copy hardware decoding, with real-time control via mpv's JSON IPC
///    (play/pause, seek, speed, volume, normalization, position/duration/eof).
/// 2. External `mpv` / `ffplay` / `VLC` / `MPC-HC` popout window fallback.
/// 3. Simulated synchronized timeline fallback when no real player is alive.
#[derive(Clone)]
pub struct MediaEngine {
    inner: Arc<Mutex<MediaEngineInner>>,
    sleep_guard: Arc<ScreenSleepGuard>,
}

struct MediaEngineInner {
    state: PlaybackState,
    position_secs: f64,
    duration_secs: f64,
    speed: f64,
    volume: f64,
    audio_normalization: bool,
    active_video_url: Option<String>,
    active_audio_url: Option<String>,
    active_title: String,
    child_process: Option<Child>,
    // In-app embedded viewport state
    embedded_surface: Option<EmbeddedSurface>,
    parent_hwnd: Option<isize>,
    ipc: Option<Arc<MpvIpcMonitor>>,
    /// Wall-clock time of the previous [`MediaEngine::tick`]. Used so the
    /// locally interpolated clock advances by real elapsed time instead of a
    /// fixed step (iced can deliver several queued ticks back-to-back).
    last_tick_at: Option<Instant>,
    /// True once mpv has reported at least one live sample for the current
    /// playback session. Until then the position stays at the requested start
    /// instead of inventing progress while mpv is still buffering.
    ipc_started: bool,
}

impl Drop for MediaEngineInner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child_process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(ipc) = self.ipc.take() {
            ipc.shutdown();
        }
    }
}

/// Shuts down and releases the current IPC monitor (background threads wind down).
fn release_ipc(ipc_slot: &mut Option<Arc<MpvIpcMonitor>>) {
    if let Some(ipc) = ipc_slot.take() {
        ipc.shutdown();
    }
}

impl MediaEngine {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MediaEngineInner {
                state: PlaybackState::Stopped,
                position_secs: 0.0,
                duration_secs: 0.0,
                speed: 1.0,
                volume: 100.0,
                audio_normalization: true,
                active_video_url: None,
                active_audio_url: None,
                active_title: String::new(),
                child_process: None,
                embedded_surface: None,
                parent_hwnd: None,
                ipc: None,
                last_tick_at: None,
                ipc_started: false,
            })),
            sleep_guard: Arc::new(ScreenSleepGuard::new()),
        }
    }

    /// Starts playback of a resolved video stream (and optional separate high-bitrate Opus audio stream).
    pub fn load_and_play(
        &self,
        title: &str,
        video_url: &str,
        audio_url: Option<&str>,
        duration_secs: f64,
        start_position_secs: f64,
    ) {
        self.stop_external_player();

        let mut guard = self.inner.lock().unwrap();
        guard.active_title = title.to_string();
        guard.active_video_url = Some(video_url.to_string());
        guard.active_audio_url = audio_url.map(|s| s.to_string());
        guard.duration_secs = duration_secs;
        guard.position_secs = start_position_secs;
        guard.state = PlaybackState::Playing;
        guard.last_tick_at = None;
        guard.ipc_started = false;

        self.sleep_guard.acquire();
        info!(
            "MediaEngine started '{}' at {:.1}s / {:.1}s (speed: {:.2}x)",
            title, start_position_secs, duration_secs, guard.speed
        );
    }

    /// Attaches mpv to a native child window INSIDE the WinTube app window
    /// (`--wid` embedding) so video & audio play directly in the app UI.
    /// Must be called from the UI thread (creates the child HWND).
    pub fn launch_embedded_viewport(&self, is_tv: bool) -> anyhow::Result<String> {
        let mut guard = self.inner.lock().unwrap();

        if let Some(mut child) = guard.child_process.take() {
            let _ = child.kill();
        }
        release_ipc(&mut guard.ipc);

        let video_url = guard
            .active_video_url
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No active video stream loaded"))?;
        let audio_url = guard.active_audio_url.clone();
        let title = guard.active_title.clone();
        let pos = guard.position_secs;
        let speed = guard.speed;
        let normalize = guard.audio_normalization;

        let parent = embedded::find_main_window_hwnd()
            .ok_or_else(|| anyhow::anyhow!("WinTube main window not found"))?;
        let rect = embedded::compute_video_band_rect(parent, is_tv)
            .ok_or_else(|| anyhow::anyhow!("Unable to compute embedded video region"))?;
        let child_hwnd = EmbeddedSurface::ensure(&mut guard.embedded_surface, parent, rect)?;
        guard.parent_hwnd = Some(parent.0 as isize);

        let pipe_name = crate::mpv_ipc::new_pipe_name();
        let mpv_bin = find_binary("mpv.exe").ok_or_else(|| anyhow::anyhow!("mpv.exe not found"))?;

        let mpv_log = std::env::temp_dir().join(format!("wintube-mpv-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&mpv_log);

        let mut cmd = Command::new(&mpv_bin);
        cmd.arg(&video_url)
            .arg(format!("--wid={}", child_hwnd))
            .arg("--hwdec=auto-safe")
            .arg("--gpu-context=d3d11")
            .arg("--force-window=no")
            .arg("--keep-open=yes")
            .arg("--osc=no")
            .arg(format!("--title=WinTube - {}", title))
            .arg(format!("--speed={:.2}", speed))
            .arg(format!("--user-agent={}", MPV_USER_AGENT))
            .arg("--http-header-fields=Referer: https://www.youtube.com/")
            .arg(format!("--input-ipc-server={}", pipe_name))
            .arg(format!("--log-file={}", mpv_log.display()))
            .arg("--msg-level=all=info")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if pos > 1.0 {
            cmd.arg(format!("--start=+{:.1}", pos));
        }
        if let Some(ref aud) = audio_url {
            cmd.arg(format!("--audio-file={}", aud));
        }
        if normalize {
            cmd.arg(format!("--af={}", AUDIO_NORMALIZATION_FILTER));
        }

        let child = cmd.spawn()?;
        guard.child_process = Some(child);
        guard.ipc = Some(MpvIpcMonitor::start(pipe_name));
        info!(
            "Embedded in-app mpv viewport attached (parent {} child {} rect {:?} log {})",
            parent.0 as isize,
            child_hwnd,
            rect,
            mpv_log.display()
        );
        Ok("Embedded mpv (D3D11VA Zero-Copy) — In-App Viewport".to_string())
    }

    /// Repositions the embedded video surface after window resizes.
    pub fn sync_embedded_layout(&self, is_tv: bool) {
        let mut guard = self.inner.lock().unwrap();
        let parent_hwnd = match guard.parent_hwnd {
            Some(hwnd) => hwnd,
            None => return,
        };
        let Some(surface) = guard.embedded_surface.as_mut() else {
            return;
        };
        let parent = HWND(parent_hwnd as *mut _);
        if let Some(rect) = embedded::compute_video_band_rect(parent, is_tv) {
            if rect != surface.last_rect {
                surface.move_to(rect);
                debug!("Repositioned embedded video surface to {:?}", rect);
            }
        }
    }

    /// Shows or hides the embedded video surface (e.g. when leaving Now Playing).
    pub fn set_embedded_visible(&self, visible: bool) {
        let guard = self.inner.lock().unwrap();
        if let Some(surface) = guard.embedded_surface.as_ref() {
            surface.set_visible(visible);
        }
    }

    /// True when a live mpv IPC connection is reporting fresh playback state.
    pub fn is_embedded_ipc_live(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard
            .ipc
            .as_ref()
            .map(|ipc| ipc.is_fresh())
            .unwrap_or(false)
    }

    /// IPC pipe name of the live embedded player, if any (for diagnostics).
    pub fn ipc_pipe_name(&self) -> Option<String> {
        let guard = self.inner.lock().unwrap();
        guard.ipc.as_ref().map(|ipc| ipc.pipe_name().to_string())
    }

    /// Launches hardware-accelerated external D3D11VA video viewport (`mpv` if available, else `ffplay`).
    /// Automatically passes separate 4K/8K video + Opus audio streams or muxed stream, playback speed, and start timestamp.
    pub fn launch_hardware_viewport(&self) -> anyhow::Result<String> {
        let mut guard = self.inner.lock().unwrap();

        // Kill any previous external player process
        if let Some(mut child) = guard.child_process.take() {
            let _ = child.kill();
        }
        release_ipc(&mut guard.ipc);

        let video_url = guard
            .active_video_url
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No active video stream loaded"))?;
        let audio_url = guard.active_audio_url.clone();
        let title = guard.active_title.clone();
        let pos = guard.position_secs;
        let speed = guard.speed;
        let normalize = guard.audio_normalization;

        // 1. Try mpv / mpvnet first (supports --audio-file for 4K60/8K adaptive streams + D3D11VA + scaletempo2)
        if let Some(mpv_bin) = find_binary("mpv.exe") {
            let mut cmd = Command::new(&mpv_bin);
            cmd.arg(&video_url)
                .arg("--hwdec=auto-safe")
                .arg("--gpu-context=d3d11")
                .arg(format!("--title=WinTube - {}", title))
                .arg(format!("--speed={:.2}", speed))
                .arg("--force-window=immediate")
                .arg("--keep-open=no")
                .arg(format!("--user-agent={}", MPV_USER_AGENT))
                .arg("--http-header-fields=Referer: https://www.youtube.com/")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            if pos > 1.0 {
                cmd.arg(format!("--start=+{:.1}", pos));
            }

            if let Some(ref aud) = audio_url {
                cmd.arg(format!("--audio-file={}", aud));
            }
            if normalize {
                cmd.arg(format!("--af={}", AUDIO_NORMALIZATION_FILTER));
            }

            let child = cmd.spawn()?;
            guard.child_process = Some(child);
            return Ok("mpv (Direct3D11VA Zero-Copy Hardware Decoder)".to_string());
        }

        // 2. Fallback to ffplay (installed via FFmpeg)
        if let Some(ffplay_bin) = find_binary("ffplay.exe") {
            let mut cmd = Command::new(&ffplay_bin);
            cmd.arg("-window_title")
                .arg(format!("WinTube - {}", title))
                .arg("-loglevel")
                .arg("warning")
                .arg("-user_agent")
                .arg(MPV_USER_AGENT)
                .arg("-headers")
                .arg("Referer: https://www.youtube.com/\r\n")
                .arg(&video_url);

            if pos > 1.0 {
                cmd.arg("-ss").arg(format!("{:.1}", pos));
            }

            let child = cmd.spawn()?;
            guard.child_process = Some(child);
            return Ok("FFmpeg/ffplay Hardware-Accelerated Viewport".to_string());
        }

        // 3. Fallback to VLC Media Player
        if let Some(vlc_bin) = find_binary("vlc.exe") {
            let mut cmd = Command::new(&vlc_bin);
            cmd.arg(&video_url)
                .arg(format!("--meta-title=WinTube - {}", title))
                .arg(format!("--http-user-agent={}", MPV_USER_AGENT));

            if pos > 1.0 {
                cmd.arg(format!("--start-time={:.1}", pos));
            }

            if let Some(ref aud) = audio_url {
                cmd.arg(format!(":input-slave={}", aud));
            }

            let child = cmd.spawn()?;
            guard.child_process = Some(child);
            return Ok("VLC Media Player (Direct3D11 Hardware Viewport)".to_string());
        }

        // 3. Try MPC-HC (Media Player Classic 64-bit with K-Lite LAV D3D11 Decoder)
        if let Some(mpc_bin) = find_binary("mpc-hc64.exe").or_else(|| find_binary("mpc-hc.exe")) {
            let mut cmd = Command::new(&mpc_bin);
            let start_ms = (pos * 1000.0) as u64;
            cmd.arg(&video_url)
                .arg("/play")
                .arg("/start")
                .arg(start_ms.to_string());

            let child = cmd.spawn()?;
            guard.child_process = Some(child);
            return Ok("Media Player Classic (K-Lite D3D11 Hardware Viewport)".to_string());
        }

        // 4. Fallback: Launch stream via Windows Default Shell Association
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", &video_url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn()?;
        guard.child_process = Some(child);
        Ok("Default Windows Player / Browser Stream Viewport".to_string())
    }

    pub fn is_child_alive(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if let Some(ref mut child) = guard.child_process {
            if let Ok(Some(_status)) = child.try_wait() {
                guard.child_process = None;
                false
            } else {
                true
            }
        } else {
            false
        }
    }

    pub fn toggle_play_pause(&self) -> PlaybackState {
        let mut guard = self.inner.lock().unwrap();

        // Real player control via mpv IPC when available
        if let Some(ref ipc) = guard.ipc {
            if ipc.is_fresh() {
                let runtime = ipc.snapshot();
                if runtime.eof_reached {
                    // Restart from the beginning after end-of-file
                    let _ = ipc.send(&serde_json::json!({ "command": ["seek", 0.0, "absolute"] }));
                    let _ = ipc
                        .send(&serde_json::json!({ "command": ["set_property", "pause", false] }));
                } else {
                    let _ = ipc.send(&serde_json::json!({ "command": ["cycle", "pause"] }));
                }
                guard.state = if runtime.paused || runtime.eof_reached {
                    self.sleep_guard.acquire();
                    PlaybackState::Playing
                } else {
                    self.sleep_guard.release();
                    PlaybackState::Paused
                };
                return guard.state;
            }
        }

        guard.state = match guard.state {
            PlaybackState::Playing => {
                self.sleep_guard.release();
                PlaybackState::Paused
            }
            PlaybackState::Paused | PlaybackState::Stopped | PlaybackState::Ended => {
                self.sleep_guard.acquire();
                PlaybackState::Playing
            }
            other => other,
        };
        guard.state
    }

    pub fn seek_to(&self, position_secs: f64) {
        let mut guard = self.inner.lock().unwrap();
        let clamped = position_secs.clamp(0.0, guard.duration_secs.max(1.0));
        guard.position_secs = clamped;
        if let Some(ref ipc) = guard.ipc {
            let _ = ipc.send(&serde_json::json!({ "command": ["seek", clamped, "absolute"] }));
        }
        debug!("Seeked to {:.1}s", clamped);
    }

    pub fn seek_relative(&self, delta_secs: f64) -> f64 {
        let mut guard = self.inner.lock().unwrap();
        let next = (guard.position_secs + delta_secs).clamp(0.0, guard.duration_secs.max(1.0));
        guard.position_secs = next;
        if let Some(ref ipc) = guard.ipc {
            let _ = ipc.send(&serde_json::json!({ "command": ["seek", next, "absolute"] }));
        }
        next
    }

    pub fn set_speed(&self, speed: f64) -> f64 {
        let mut guard = self.inner.lock().unwrap();
        guard.speed = speed.clamp(0.25, 4.0);
        if let Some(ref ipc) = guard.ipc {
            let _ =
                ipc.send(&serde_json::json!({ "command": ["set_property", "speed", guard.speed] }));
        }
        guard.speed
    }

    pub fn set_volume(&self, volume: f64) -> f64 {
        let mut guard = self.inner.lock().unwrap();
        guard.volume = volume.clamp(0.0, 100.0);
        if let Some(ref ipc) = guard.ipc {
            let _ = ipc
                .send(&serde_json::json!({ "command": ["set_property", "volume", guard.volume] }));
        }
        guard.volume
    }

    pub fn toggle_audio_normalization(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        guard.audio_normalization = !guard.audio_normalization;
        if let Some(ref ipc) = guard.ipc {
            let filter = if guard.audio_normalization {
                AUDIO_NORMALIZATION_FILTER
            } else {
                ""
            };
            let _ = ipc.send(&serde_json::json!({ "command": ["set_property", "af", filter] }));
        }
        guard.audio_normalization
    }

    /// Advances playback clock by `dt_secs` when playing; returns updated progress.
    /// Prefers REAL position/duration/state reported by the embedded mpv IPC.
    pub fn tick(&self, dt_secs: f64) -> (PlaybackState, PlaybackProgress) {
        let mut guard = self.inner.lock().unwrap();

        // Wall-clock delta: iced may deliver a burst of queued `PlayerTick`
        // messages after a slow frame, and adding a fixed 0.25 s for each would
        // run the UI clock ahead of mpv (which then snaps it back on the next
        // IPC sample).
        let now = Instant::now();
        let wall_dt = guard
            .last_tick_at
            .map(|last| now.duration_since(last).as_secs_f64())
            .filter(|dt| *dt > 0.0)
            .unwrap_or(dt_secs)
            .min(1.0);
        guard.last_tick_at = Some(now);

        // Live mpv state via IPC wins over the simulated clock
        if let Some(ref ipc) = guard.ipc {
            if ipc.is_fresh() {
                let runtime = ipc.snapshot();
                guard.ipc_started = true;
                let new_state = if runtime.eof_reached {
                    PlaybackState::Ended
                } else if runtime.paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                };
                if new_state != guard.state {
                    match new_state {
                        PlaybackState::Playing => self.sleep_guard.acquire(),
                        PlaybackState::Ended | PlaybackState::Paused => self.sleep_guard.release(),
                        _ => {}
                    }
                    guard.state = new_state;
                }
                guard.position_secs = runtime.time_pos;
                if runtime.duration > 0.0 {
                    guard.duration_secs = runtime.duration;
                }
                let progress = PlaybackProgress {
                    position_secs: guard.position_secs,
                    duration_secs: guard.duration_secs,
                    buffer_secs: (guard.position_secs + 25.0).min(guard.duration_secs),
                };
                return (guard.state, progress);
            }
        }

        // IPC is attached but has not reported yet (mpv still starting or
        // seeking): hold the requested start position instead of inventing
        // progress that will snap back as soon as real samples arrive.
        if guard.ipc.is_some() && !guard.ipc_started {
            let progress = PlaybackProgress {
                position_secs: guard.position_secs,
                duration_secs: guard.duration_secs,
                buffer_secs: guard.position_secs.min(guard.duration_secs),
            };
            return (guard.state, progress);
        }

        // Check if external hardware viewport child process terminated
        if let Some(ref mut child) = guard.child_process {
            if let Ok(Some(status)) = child.try_wait() {
                warn!("mpv child process exited: {status:?}");
                guard.child_process = None;
                guard.state = PlaybackState::Paused;
                self.sleep_guard.release();
            }
        }

        if guard.state == PlaybackState::Playing {
            guard.position_secs += wall_dt * guard.speed;
            if guard.duration_secs > 0.0 && guard.position_secs >= guard.duration_secs {
                guard.position_secs = guard.duration_secs;
                guard.state = PlaybackState::Ended;
                self.sleep_guard.release();
            }
        }
        let progress = PlaybackProgress {
            position_secs: guard.position_secs,
            duration_secs: guard.duration_secs,
            buffer_secs: (guard.position_secs + 25.0).min(guard.duration_secs),
        };
        (guard.state, progress)
    }

    pub fn stop_external_player(&self) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(mut child) = guard.child_process.take() {
            let _ = child.kill();
        }
        release_ipc(&mut guard.ipc);
        if let Some(surface) = guard.embedded_surface.as_ref() {
            surface.set_visible(false);
        }
        self.sleep_guard.release();
    }
}

fn find_binary(name: &str) -> Option<PathBuf> {
    fn is_usable(path: &Path) -> bool {
        path.is_file()
            && path
                .metadata()
                .map(|meta| meta.len() > 0)
                .unwrap_or(false)
    }

    // 0. Explicit override (portable installs, debugging, custom mpv builds).
    if let Ok(explicit) = std::env::var("WINTUBE_MPV_PATH") {
        let candidate = PathBuf::from(explicit);
        if is_usable(&candidate) {
            return Some(candidate);
        }
    }

    // 1. Next to the executable. A portable layout keeps `WinTube.exe` and
    //    `bin/mpv.exe` together; walking up a couple of levels also lets
    //    `target/release/wintube.exe` find the workspace's `bin/mpv.exe`.
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let mut roots = vec![parent.to_path_buf()];
            let mut cursor = parent;
            for _ in 0..2 {
                if let Some(up) = cursor.parent() {
                    roots.push(up.to_path_buf());
                    cursor = up;
                }
            }
            for root in roots {
                for sub in [".", "bin", "bin/mpv"] {
                    let candidate = root.join(sub).join(name);
                    if is_usable(&candidate) {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    // 2. Current working directory (running from the repository root).
    for sub in [".", "bin", "bin/mpv"] {
        let candidate = PathBuf::from(sub).join(name);
        if is_usable(&candidate) {
            return Some(candidate);
        }
    }

    // 3. Known installation directories (VLC, MPC-HC, mpv, WinGet)
    let program_files = [
        r"C:\Program Files\VideoLAN\VLC",
        r"C:\Program Files (x86)\VideoLAN\VLC",
        r"C:\Program Files (x86)\K-Lite Codec Pack\MPC-HC64",
        r"C:\Program Files\K-Lite Codec Pack\MPC-HC64",
        r"C:\Program Files\MPC-HC",
        r"C:\Program Files (x86)\MPC-HC",
    ];
    for base in &program_files {
        let candidate = PathBuf::from(base).join(name);
        if candidate.is_file() {
            if let Ok(meta) = candidate.metadata() {
                if meta.len() > 0 {
                    return Some(candidate);
                }
            }
        }
    }

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        let candidates = [
            PathBuf::from(&local_app_data)
                .join("Programs")
                .join("mpv")
                .join(name),
            PathBuf::from(&local_app_data)
                .join("Programs")
                .join("mpv.net")
                .join(name),
            PathBuf::from(&local_app_data)
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join(name),
        ];
        for candidate in &candidates {
            if candidate.is_file() {
                if let Ok(meta) = candidate.metadata() {
                    if meta.len() > 0 {
                        return Some(candidate.clone());
                    }
                }
            }
        }
    }

    // 4. PATH entries (verifying file exists and is not a 0-byte broken link)
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                if let Ok(meta) = candidate.metadata() {
                    if meta.len() > 0 {
                        return Some(candidate);
                    }
                }
            }
        }
    }
    None
}
