//! Pure-Rust remuxer: YouTube video-only + audio-only MP4 tracks → single MP4.
//! Demux with `mp4` (alfg), mux with `muxide`. Replaces `ffmpeg -c copy`
//! for Wintube's download path. H.264/AAC first; AV1/Opus/VP9 follow same pattern.
//! NOTE: muxide is write-only (encoder frames in, MP4 out) — this crate supplies
//! the demux + Annex-B conversion layer ffmpeg normally does.

pub mod fmp4;
pub mod remux;

pub use remux::{remux_mp4_tracks, RemuxInfo};
