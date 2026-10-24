use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::debug;
use windows::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};

/// Prevents Windows from turning off the display or going to sleep while a video is playing.
/// Mirrors Android's `FLAG_KEEP_SCREEN_ON` used in SmartTube's `PlaybackActivity`.
#[derive(Debug, Default)]
pub struct ScreenSleepGuard {
    active: Arc<AtomicBool>,
}

impl ScreenSleepGuard {
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Keeps the display awake and system responsive during playback.
    pub fn acquire(&self) {
        if !self.active.swap(true, Ordering::SeqCst) {
            unsafe {
                let prev = SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED);
                debug!("Acquired Windows Screen Sleep Guard (prev state: {:?})", prev);
            }
        }
    }

    /// Releases sleep inhibition when video is paused or stopped.
    pub fn release(&self) {
        if self.active.swap(false, Ordering::SeqCst) {
            unsafe {
                let prev = SetThreadExecutionState(ES_CONTINUOUS);
                debug!("Released Windows Screen Sleep Guard (prev state: {:?})", prev);
            }
        }
    }
}

impl Drop for ScreenSleepGuard {
    fn drop(&mut self) {
        self.release();
    }
}
