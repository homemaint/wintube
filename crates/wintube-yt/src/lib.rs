//! InnerTube client wrapper.
//! Primary: yt-dlp for reliable metadata+formats (handles decipher/SABR/PO-token
//! churn for us, exactly like SmartTube's AppService+Player decipher step).
//! Secondary: innertube-rs for search/browse/comments/chat where yt-dlp has no
//! native endpoint. This keeps the Rust binary thin while matching SmartTube's
//! MediaServiceCore feature surface (browse/search/next/comments/chat).
//!
//! DESIGN NOTE: SmartTube builds its own MPD from adaptiveFormats
//! (formatbuilders/mpdbuilder). Here mpv consumes the DASH URL / progressive
//! URL directly in M1; MPD synthesis is a later optimization. The Format model
//! is kept identical so the quality picker works either way.

pub mod client;
pub mod innertube;

pub use client::YtClient;
