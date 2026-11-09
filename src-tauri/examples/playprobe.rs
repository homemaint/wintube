//! Probe the playback path the way the app does it: resolve formats with the
//! sidecar and check that the progressive stream is actually fetchable.
//!
//! Usage: cargo run --release --example playprobe -p wintube -- <videoId>

#[tokio::main]
async fn main() {
    let id = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "dQw4w9WgXcQ".to_string());
    let (yt, _ff) = wintube_download::ensure_sidecars();
    println!("sidecar: {yt}");

    let info = match wintube_yt::YtClient::new(yt).get_play_info(&id).await {
        Ok(i) => i,
        Err(e) => {
            println!("get_play_info FAILED: {e}");
            return;
        }
    };
    println!("title: {}", info.video.title);
    println!("formats: {}", info.formats.len());
    println!(
        "dash={} hls={}",
        info.dash_url.is_some(),
        info.hls_url.is_some()
    );
    match &info.best_progressive_url {
        Some(u) => {
            println!("progressive: {}", &u[..u.len().min(110)]);
            let client = reqwest::Client::builder()
                .user_agent("Mozilla/5.0")
                .build()
                .expect("client");
            match client
                .get(u)
                .header("Range", "bytes=0-2047")
                .send()
                .await
            {
                Ok(r) => {
                    let status = r.status();
                    let ct = r
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("?")
                        .to_string();
                    let len = r.content_length().unwrap_or(0);
                    let body = r.bytes().await.map(|b| b.len()).unwrap_or(0);
                    println!("GET -> status={status} type={ct} content-length={len} got={body} bytes");
                    if body > 0 {
                        println!("RESULT: progressive stream is playable");
                    }
                }
                Err(e) => println!("GET failed: {e}"),
            }
        }
        None => println!("progressive: NONE — <video> would have no source"),
    }
}
