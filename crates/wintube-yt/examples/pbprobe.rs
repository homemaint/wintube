//! Probe: print the selectable playback ladder for a video.
//!
//! `cargo run --release --example pbprobe -p wintube-yt -- <video_id>`

#[tokio::main]
async fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "p0709kbMH3Q".to_string());

    let t = std::time::Instant::now();
    match wintube_yt::innertube::playback_streams(&id).await {
        Ok(s) => {
            println!(
                "{}ms: {} video steps, audio={}, duration={:?}",
                t.elapsed().as_millis(),
                s.video.len(),
                s.audio_url.is_some(),
                s.duration
            );
            for v in &s.video {
                println!("  {}p ({}x{})", v.height, v.width, v.height);
            }
            if let Some(url) = s.audio_url {
                println!("  audio host={}", url.split('/').nth(2).unwrap_or("?"));
            }
        }
        Err(e) => println!("FAILED after {}ms: {e}", t.elapsed().as_millis()),
    }
}
