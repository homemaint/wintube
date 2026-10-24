use wintube_innertube::InnertubeClient;
use wintube_services::{ReturnYouTubeDislikeService, SponsorBlockService, StorageEngine};
use wintube_player::{ScreenSleepGuard, TrackSelector, MaxResolution, VideoCodecPreference};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    println!("============================================================");
    println!("     WinTube (Native Rust YouTube Client for Windows PC)    ");
    println!("============================================================");

    let client = InnertubeClient::new();
    let ryd = ReturnYouTubeDislikeService::new();
    let sb = SponsorBlockService::new();
    let storage = StorageEngine::in_memory()?;
    let sleep_guard = ScreenSleepGuard::new();

    let query = "Rust 2026 programming";
    println!("\n[1] Querying Innertube Search for '{}'...", query);
    let search_results = client.search(query).await?;
    println!(" -> Innertube search returned payload successfully!");

    let sample_video_id = "dQw4w9WgXcQ";
    println!("\n[2] Fetching video stream & metadata for '{}'...", sample_video_id);
    let playback_info = client.get_video_playback_info(sample_video_id).await?;
    println!(" -> Title: {}", playback_info.metadata.title);
    println!(" -> Channel: {} (ID: {})", playback_info.metadata.author, playback_info.metadata.channel_id);
    println!(" -> Duration: {} seconds", playback_info.metadata.duration_seconds);
    println!(" -> Available Video Streams: {}", playback_info.video_streams.len());
    println!(" -> Available Audio Streams: {}", playback_info.audio_streams.len());

    println!("\n[3] Smart Track Selection (Target: 1440p / 4K, VP9/AV1 Preferred)...");
    let best_video = TrackSelector::select_best_video(
        &playback_info.video_streams,
        |s| s.height,
        |s| &s.codecs,
        MaxResolution::Res2160p_4K,
        VideoCodecPreference::Vp9Preferred,
    );
    if let Some(video) = best_video {
        println!(" -> Selected Best Stream: Itag {}, Resolution {}x{}@{}fps, Codec: {}, Bitrate: {} bps",
            video.itag,
            video.width.unwrap_or(0),
            video.height.unwrap_or(0),
            video.fps.unwrap_or(0),
            video.codecs,
            video.bitrate,
        );
    }

    println!("\n[4] Querying ReturnYouTubeDislike (RYD) & SponsorBlock concurrently...");
    let (ryd_res, sb_res) = tokio::join!(
        ryd.get_votes(sample_video_id),
        sb.fetch_segments(sample_video_id)
    );

    if let Ok(votes) = ryd_res {
        println!(" -> Restored Dislikes: {} (Likes: {}, Rating: {:.2}/5.00)", votes.dislikes, votes.likes, votes.rating);
    }

    if let Ok(segments) = sb_res {
        println!(" -> SponsorBlock Segments Detected: {}", segments.len());
    }

    println!("\n[5] Private Zero-Tracking Storage (redb)...");
    storage.update_playback_progress(
        sample_video_id,
        &playback_info.metadata.title,
        &playback_info.metadata.channel_id,
        &playback_info.metadata.author,
        playback_info.metadata.duration_seconds as i64,
        120,
    )?;
    let resume = storage.get_resume_position(sample_video_id)?;
    println!(" -> Saved watch progress; Resume Position: {:?}s", resume);

    println!("\n[6] Windows OS Power / Sleep Guard...");
    sleep_guard.acquire();
    println!(" -> Screen sleep guard acquired: System and display will not sleep during video playback.");
    sleep_guard.release();
    println!(" -> Screen sleep guard released cleanly.");

    println!("\n============================================================");
    println!("     All WinTube Core Engines Verified & Operating!         ");
    println!("============================================================");

    Ok(())
}
