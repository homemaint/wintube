//! Live verification of the loopback media relay against real googlevideo
//! streams.
//!
//! Run manually (requires network):
//!
//! ```text
//! cargo test -p wintube-app --test relay_live -- --ignored --nocapture
//! ```

#[path = "../src/relay.rs"]
mod relay;

use std::process::Command;
use std::time::{Duration, Instant};

use relay::{StreamRelay, UpstreamInfo};
use wintube_innertube::InnertubeClient;

struct FetchOutcome {
    code: String,
    bytes: u64,
    elapsed: Duration,
    exit: Option<i32>,
}

impl std::fmt::Display for FetchOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} bytes in {:.1}s (curl exit {:?})",
            self.code,
            self.bytes,
            self.elapsed.as_secs_f64(),
            self.exit
        )
    }
}

/// Issues one request and reports status, downloaded size and wall time.
fn fetch(url: &str, range: Option<&str>, max_time: u64) -> FetchOutcome {
    let mut cmd = Command::new("curl.exe");
    cmd.args([
        "-s",
        "-o",
        "NUL",
        "-w",
        "%{http_code} %{size_download}",
        "--max-time",
    ])
    .arg(max_time.to_string());
    if let Some(range) = range {
        cmd.args(["-H", &format!("Range: {range}")]);
    }
    let started = Instant::now();
    let out = cmd.arg(url).output().expect("run curl");
    let elapsed = started.elapsed();
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut parts = text.split_whitespace();
    let code = parts.next().unwrap_or_default().to_string();
    let bytes = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    FetchOutcome {
        code,
        bytes,
        elapsed,
        exit: out.status.code(),
    }
}

#[test]
#[ignore = "hits the live YouTube CDN"]
fn relay_serves_full_length_visionos_streams() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("debug")
        .with_target(false)
        .try_init();

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let info = runtime
        .block_on(async {
            InnertubeClient::new()
                .get_video_playback_info("U50sfNqHtOw")
                .await
        })
        .expect("playback info");

    println!("client: {:?}", info.client);
    let best = info
        .video_streams
        .iter()
        .max_by_key(|s| s.height.unwrap_or(0))
        .expect("video stream")
        .clone();
    let audio = info
        .audio_streams
        .iter()
        .max_by_key(|a| a.bitrate)
        .expect("audio stream")
        .clone();
    println!(
        "video itag {} ({}x{:?}) / audio itag {}",
        best.itag,
        best.width.unwrap_or(0),
        best.height,
        audio.itag
    );

    // Direct CDN reference first, so relay numbers can be compared against the
    // raw googlevideo URL for the exact same range.
    let direct = fetch(&best.url, Some("bytes=134217728-155189247"), 25);
    println!("direct @128M (20 MiB): {direct}");

    let relay = StreamRelay::start().expect("start relay");
    println!("relay base URL: {}", relay.base_url());
    let mut upstream = UpstreamInfo::new(
        best.url.clone(),
        best.mime_type.clone(),
        info.client.user_agent(),
    );
    if let Some(total) = upstream.total_len {
        println!("declared video length: {total} bytes");
        assert!(total > 64 * 1024 * 1024, "test video should exceed 64 MiB");
    }
    upstream.referer = Some("https://www.youtube.com/".to_string());
    let video_url = relay.register(upstream, None);

    let audio_url = relay.register(
        UpstreamInfo::new(
            audio.url.clone(),
            audio.mime_type.clone(),
            info.client.user_agent(),
        ),
        None,
    );

    // Bounded high-offset request through the relay (what mpv does on seek).
    let bounded = fetch(&video_url, Some("bytes=134217728-155189247"), 30);
    println!("relay  @128M (20 MiB): {bounded}");
    assert_eq!(bounded.code, "206", "relay must answer with 206");
    assert_eq!(
        bounded.bytes,
        20 * 1024 * 1024,
        "relay must deliver the whole bounded range ({bounded})"
    );

    // Open-ended request as issued by mpv. The relay must stream well past
    // the 32 MiB mark that anonymous IOS/ANDROID_VR URLs die at.
    let open = fetch(&video_url, Some("bytes=0-"), 20);
    println!("relay  open-ended:     {open}");
    assert_eq!(open.code, "206");
    assert!(
        open.bytes > 40 * 1024 * 1024,
        "relay delivered only {} bytes; expected to pass the 32 MiB wall",
        open.bytes
    );

    let audio_out = fetch(&audio_url, Some("bytes=4194304-5242879"), 20);
    println!("relay  audio @4M:      {audio_out}");
    assert_eq!(audio_out.code, "206");
    assert_eq!(audio_out.bytes, 1024 * 1024);

    // HEAD must report success without a body.
    let out = Command::new("curl.exe")
        .args(["-s", "-I", "--max-time", "15"])
        .arg(&video_url)
        .output()
        .expect("curl HEAD");
    let head = String::from_utf8_lossy(&out.stdout);
    println!("HEAD response:\n{head}");
    assert!(head.contains("206"));

    drop(relay);
    std::thread::sleep(Duration::from_millis(100));
}
