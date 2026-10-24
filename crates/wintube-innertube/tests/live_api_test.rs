use wintube_innertube::InnertubeClient;

#[tokio::test]
async fn test_innertube_search_and_metadata() {
    let client = InnertubeClient::new();

    // 1. Live Search
    println!("Testing live Innertube Search...");
    let search_res = client.search("Rust programming language").await;
    assert!(search_res.is_ok(), "Search request should succeed: {:?}", search_res.err());
    let search_json = search_res.unwrap();
    assert!(search_json.get("contents").is_some() || search_json.get("trackingParams").is_some());

    // 2. Stream & Playback Extraction
    println!("Testing live Video Player Extraction for dQw4w9WgXcQ...");
    let playback_res = client.get_video_playback_info("dQw4w9WgXcQ").await;
    match playback_res {
        Ok(info) => {
            println!("Successfully resolved video: {}", info.metadata.title);
            assert_eq!(info.metadata.video_id, "dQw4w9WgXcQ");
            assert!(!info.video_streams.is_empty(), "Should resolve at least 1 video stream");
            assert!(!info.audio_streams.is_empty(), "Should resolve at least 1 audio stream");
            println!("Found {} video streams and {} audio streams", info.video_streams.len(), info.audio_streams.len());
        }
        Err(err) => {
            println!("Playback note (expected if botguard poToken active on IP): {:?}", err);
        }
    }
}
