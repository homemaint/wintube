use wintube_services::{ReturnYouTubeDislikeService, SponsorBlockService};

#[tokio::test]
async fn test_live_services() {
    let video_id = "dQw4w9WgXcQ";

    // 1. ReturnYouTubeDislike
    let ryd = ReturnYouTubeDislikeService::new();
    let stats = ryd.get_votes(video_id).await;
    assert!(stats.is_ok(), "RYD should succeed: {:?}", stats.err());
    let stats = stats.unwrap();
    println!("RYD Stats for {}: Likes: {}, Dislikes: {}, Rating: {:.2}", video_id, stats.likes, stats.dislikes, stats.rating);
    assert!(stats.likes > 0);

    // 2. SponsorBlock (SHA-256 privacy prefix)
    let sb = SponsorBlockService::new();
    let segments = sb.fetch_segments(video_id).await;
    assert!(segments.is_ok(), "SponsorBlock query should succeed: {:?}", segments.err());
    let segments = segments.unwrap();
    println!("SponsorBlock segments found for {}: {}", video_id, segments.len());
    for seg in &segments {
        println!("Segment: {:?} [{:.1}s - {:.1}s]", seg.category, seg.start_time, seg.end_time);
    }
}
