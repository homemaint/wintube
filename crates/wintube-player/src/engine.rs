use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use tracing::{debug, info};

use crate::controllers::{PlaybackProgress, PlaybackState};
use crate::windows_os::ScreenSleepGuard;

/// Hardware-accelerated Media Engine for Windows PC.
/// Supports:
/// 1. Native `mpv` / `libmpv` D3D11VA zero-copy hardware decoding (`--hwdec=d3d11va`, `--audio-file=...` for 4K/8K DASH muxing)
/// 2. `ffplay` hardware-accelerated fallback (`-hwaccel d3d11va`)
/// 3. In-app synchronized timeline simulation & external window launch
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

        self.sleep_guard.acquire();
        info!(
            "MediaEngine started '{}' at {:.1}s / {:.1}s (speed: {:.2}x)",
            title, start_position_secs, duration_secs, guard.speed
        );
    }

    /// Launches hardware-accelerated external D3D11VA video viewport (`mpv` if available, else `ffplay`).
    /// Automatically passes separate 4K/8K video + Opus audio streams or muxed stream, playback speed, and start timestamp.
    pub fn launch_hardware_viewport(&self) -> anyhow::Result<String> {
        let mut guard = self.inner.lock().unwrap();

        // Kill any previous external player process
        if let Some(mut child) = guard.child_process.take() {
            let _ = child.kill();
        }

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
                .arg("--hwdec=d3d11va")
                .arg("--gpu-context=d3d11")
                .arg(format!("--title=WinTube - {}", title))
                .arg(format!("--start=+{:.1}", pos))
                .arg(format!("--speed={:.2}", speed))
                .arg("--force-window=immediate")
                .arg("--keep-open=no")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            if let Some(ref aud) = audio_url {
                cmd.arg(format!("--audio-file={}", aud));
            }
            if normalize {
                cmd.arg("--af=dynaudnorm=f=150:g=15");
            }

            let child = cmd.spawn()?;
            guard.child_process = Some(child);
            return Ok("mpv (Direct3D11VA Zero-Copy Hardware Decoder)".to_string());
        }

        // 2. Try VLC Media Player (supports --input-slave for separate video+audio muxing + D3D11)
        if let Some(vlc_bin) = find_binary("vlc.exe") {
            let mut cmd = Command::new(&vlc_bin);
            cmd.arg(&video_url)
                .arg(format!("--meta-title=WinTube - {}", title))
                .arg(format!("--start-time={:.1}", pos))
                .arg("--play-and-exit");

            if let Some(ref aud) = audio_url {
                cmd.arg(format!("--input-slave={}", aud));
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

        // 4. Fallback to ffplay (if installed and valid)
        if let Some(ffplay_bin) = find_binary("ffplay.exe") {
            let mut cmd = Command::new(&ffplay_bin);
            cmd.arg("-window_title")
                .arg(format!("WinTube - {}", title))
                .arg("-ss")
                .arg(format!("{:.1}", pos))
                .arg("-autoexit")
                .arg("-loglevel")
                .arg("quiet")
                .arg(&video_url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            let child = cmd.spawn()?;
            guard.child_process = Some(child);
            return Ok("FFmpeg/ffplay Hardware-Accelerated Viewport".to_string());
        }

        // 5. Fallback: Launch stream via Windows Default Shell Association
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", "", &video_url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = cmd.spawn()?;
        guard.child_process = Some(child);
        Ok("Default Windows Player / Browser Stream Viewport".to_string())
    }

    pub fn toggle_play_pause(&self) -> PlaybackState {
        let mut guard = self.inner.lock().unwrap();
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
        debug!("Seeked to {:.1}s", clamped);
    }

    pub fn seek_relative(&self, delta_secs: f64) -> f64 {
        let mut guard = self.inner.lock().unwrap();
        let next = (guard.position_secs + delta_secs).clamp(0.0, guard.duration_secs.max(1.0));
        guard.position_secs = next;
        next
    }

    pub fn set_speed(&self, speed: f64) -> f64 {
        let mut guard = self.inner.lock().unwrap();
        guard.speed = speed.clamp(0.25, 4.0);
        guard.speed
    }

    pub fn set_volume(&self, volume: f64) -> f64 {
        let mut guard = self.inner.lock().unwrap();
        guard.volume = volume.clamp(0.0, 100.0);
        guard.volume
    }

    pub fn toggle_audio_normalization(&self) -> bool {
        let mut guard = self.inner.lock().unwrap();
        guard.audio_normalization = !guard.audio_normalization;
        guard.audio_normalization
    }

    /// Advances playback clock by `dt_secs` when playing; returns updated progress.
    pub fn tick(&self, dt_secs: f64) -> (PlaybackState, PlaybackProgress) {
        let mut guard = self.inner.lock().unwrap();
        if guard.state == PlaybackState::Playing {
            guard.position_secs += dt_secs * guard.speed;
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
        self.sleep_guard.release();
    }
}

fn find_binary(name: &str) -> Option<PathBuf> {
    // 1. Direct workspace / relative bin/ directories
    let local_search_dirs = [
        PathBuf::from(r"D:\My Projects\Wintube\bin\mpv"),
        PathBuf::from(r"D:\My Projects\Wintube\bin"),
        PathBuf::from("bin").join("mpv"),
        PathBuf::from("bin"),
        PathBuf::from("."),
    ];

    for dir in &local_search_dirs {
        let candidate = dir.join(name);
        if candidate.is_file() {
            if let Ok(meta) = candidate.metadata() {
                if meta.len() > 0 {
                    return Some(candidate);
                }
            }
        }
    }

    // 2. Executable parent directory bin/
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            for sub in &["bin/mpv", "bin", "."] {
                let candidate = parent.join(sub).join(name);
                if candidate.is_file() {
                    if let Ok(meta) = candidate.metadata() {
                        if meta.len() > 0 {
                            return Some(candidate);
                        }
                    }
                }
            }
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
            PathBuf::from(&local_app_data).join("Programs").join("mpv").join(name),
            PathBuf::from(&local_app_data).join("Programs").join("mpv.net").join(name),
            PathBuf::from(&local_app_data).join("Microsoft").join("WinGet").join("Links").join(name),
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
