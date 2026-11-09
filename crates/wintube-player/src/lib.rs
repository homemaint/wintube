//! Player config for mpv (embedded) with HTML5 <video> fallback.
//! SmartTube -> ExoPlayer. Here -> mpv (libmpv) for AV1/VP9/HDR/8K HW decode.
//! The frontend owns <video> in M1 (progressive URL); mpv HWND embed is M2.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerConfig {
    pub hw_accel: bool,
    pub max_quality: String,
    pub codec_pref: String,
    pub volume: u8,
    pub speed: f64,
    pub sponsorblock: bool,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self { hw_accel: true, max_quality: "auto".into(), codec_pref: "auto".into(), volume: 80, speed: 1.0, sponsorblock: true }
    }
}

/// mpv args equivalent of SmartTube's ExoPlayer selector presets.
pub fn mpv_args(cfg: &PlayerConfig) -> Vec<String> {
    let mut a = vec![
        "--no-video".to_string(), // placeholder replaced by caller
    ];
    a.clear();
    if cfg.hw_accel {
        a.push("--hwdec=auto-safe".into());
        a.push("--gpu-context=d3d11".into());
    }
    a.push("--ytdl-format=bestvideo+bestaudio/best".into());
    a.push(format!("--volume={}", cfg.volume));
    a.push(format!("--speed={}", cfg.speed));
    a.push("--subsdecoding=cp65001".into());
    a
}

/// Pick the DASH/HLS/progressive URL mpv should open for a PlayInfo.
pub fn pick_url(info: &wintube_core::PlayInfo, max_height: Option<u32>) -> Option<String> {
    info.dash_url.clone()
        .or(info.hls_url.clone())
        .or_else(|| {
            info.formats.iter()
                .filter(|f| max_height.map(|h| f.height.unwrap_or(0) <= h).unwrap_or(true))
                .find(|f| f.is_progressive)
                .map(|f| f.url.clone())
        })
}
