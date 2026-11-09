//! Quality + codec ladders mirroring SmartTube's video presets.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Auto,
    P4320, // 8K
    P2160, // 4K
    P1440, // QHD
    P1080, // FHD
    P720,  // HD
    P480,
    P360,
    P240,
    P144,
}

impl Quality {
    pub fn prefs_height(&self) -> Option<u32> {
        match self {
            Quality::Auto => None,
            Quality::P4320 => Some(4320),
            Quality::P2160 => Some(2160),
            Quality::P1440 => Some(1440),
            Quality::P1080 => Some(1080),
            Quality::P720 => Some(720),
            Quality::P480 => Some(480),
            Quality::P360 => Some(360),
            Quality::P240 => Some(240),
            Quality::P144 => Some(144),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Quality::Auto => "Auto (best)",
            Quality::P4320 => "4320p · 8K",
            Quality::P2160 => "2160p · 4K",
            Quality::P1440 => "1440p · QHD",
            Quality::P1080 => "1080p · FHD",
            Quality::P720 => "720p · HD",
            Quality::P480 => "480p · SD",
            Quality::P360 => "360p",
            Quality::P240 => "240p",
            Quality::P144 => "144p",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "4320p" | "8K" => Quality::P4320,
            "2160p" | "4K" => Quality::P2160,
            "1440p" | "QHD" => Quality::P1440,
            "1080p" | "FHD" => Quality::P1080,
            "720p" | "HD" => Quality::P720,
            "480p" => Quality::P480,
            "360p" => Quality::P360,
            "240p" => Quality::P240,
            "144p" => Quality::P144,
            _ => Quality::Auto,
        }
    }

    /// yt-dlp format string for this ladder level.
    pub fn ytdlp_best(&self, codec: CodecPref) -> String {
        let h = self.prefs_height().unwrap_or(9999);
        let vcodec = match codec {
            CodecPref::Av1 => "[vcodec^=av01]/",
            CodecPref::Vp9 => "[vcodec^=vp9]/",
            CodecPref::Avc => "[vcodec^=avc]/",
            CodecPref::Auto => "",
        };
        format!("bestvideo[height<={h}]{vcodec}+bestaudio/best[height<={h}]/best")
    }

    /// Separate H.264 video-only format for the pure-Rust remux path.
    /// `wintube-mux` replaces ffmpeg for this combination only.
    pub fn ytdlp_video_avc(&self) -> String {
        let h = self.prefs_height().unwrap_or(9999);
        format!("bestvideo[height<={h}][vcodec^=avc]/bestvideo[vcodec^=avc]/best")
    }

    /// Separate AAC audio-only format for the pure-Rust remux path.
    pub fn ytdlp_audio_m4a(&self) -> &'static str {
        "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]/bestaudio"
    }

    pub fn ytdlp_audio() -> &'static str {
        "bestaudio/bestaudio[ext=m4a]/bestaudio[ext=opus]"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodecPref {
    Auto,
    Av1,
    Vp9,
    Avc,
}

impl CodecPref {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "av1" | "av01" => CodecPref::Av1,
            "vp9" => CodecPref::Vp9,
            "avc" | "h264" => CodecPref::Avc,
            _ => CodecPref::Auto,
        }
    }
}

/// Sort youtube-dlp loose formats into the ladder for the quality picker.
pub fn ladder_for(formats: &[crate::Format]) -> Vec<(String, Option<u32>, usize)> {
    // returns (label, height, count available)
    let heights = [4320u32, 2160, 1440, 1080, 720, 480, 360, 240, 144];
    let mut out = Vec::new();
    for &h in &heights {
        let label = match h {
            4320 => "4320p · 8K",
            2160 => "2160p · 4K",
            1440 => "1440p · QHD",
            1080 => "1080p · FHD",
            720 => "720p · HD",
            _ => continue,
        }.to_string();
        let n = formats.iter().filter(|f| f.height.map(|hh| hh <= h).unwrap_or(false) && !f.is_audio_only).count();
        if n > 0 { out.push((label, Some(h), n)); }
    }
    out
}
