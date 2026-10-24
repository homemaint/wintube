use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackState {
    Stopped,
    Loading,
    Playing,
    Paused,
    Ended,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackProgress {
    pub position_secs: f64,
    pub duration_secs: f64,
    pub buffer_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlayerEvent {
    StateChanged(PlaybackState),
    Progress(PlaybackProgress),
    TrackChanged {
        video_track: Option<String>,
        audio_track: Option<String>,
    },
    Seeked(f64),
    VolumeChanged(f64),
    SpeedChanged(f64),
    Error(String),
}

/// Trait implemented by player controllers (SponsorBlock controller, History logger, Auto-Next).
/// Mirrors SmartTube's `PlayerListener` architecture in `com.liskovsoft.smartyoutubetv2.common.exoplayer.controller`.
pub trait PlayerEventListener: Send + Sync {
    fn on_event(&self, event: &PlayerEvent);
}
