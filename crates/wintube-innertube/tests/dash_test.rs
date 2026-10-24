use wintube_innertube::DashManifestBuilder;
use wintube_innertube::models::{FormatStream, ByteRange};

#[test]
fn test_dash_manifest_generation() {
    let video_stream = FormatStream {
        itag: 313,
        url: Some("https://example.com/video_4k.mp4".to_string()),
        mime_type: "video/webm; codecs=\"vp9\"".to_string(),
        bitrate: 18000000,
        width: Some(3840),
        height: Some(2160),
        quality: Some("hd2160".to_string()),
        quality_label: Some("2160p".to_string()),
        fps: Some(60),
        audio_quality: None,
        approx_duration_ms: Some("120000".to_string()),
        audio_sample_rate: None,
        audio_channels: None,
        average_bitrate: Some(17500000),
        content_length: Some("250000000".to_string()),
        init_range: Some(ByteRange { start: "0".to_string(), end: "740".to_string() }),
        index_range: Some(ByteRange { start: "741".to_string(), end: "1400".to_string() }),
        signature_cipher: None,
        cipher: None,
    };

    let audio_stream = FormatStream {
        itag: 251,
        url: Some("https://example.com/audio_opus.webm".to_string()),
        mime_type: "audio/webm; codecs=\"opus\"".to_string(),
        bitrate: 160000,
        width: None,
        height: None,
        quality: Some("tiny".to_string()),
        quality_label: None,
        fps: None,
        audio_quality: Some("AUDIO_QUALITY_MEDIUM".to_string()),
        approx_duration_ms: Some("120000".to_string()),
        audio_sample_rate: Some("48000".to_string()),
        audio_channels: Some(2),
        average_bitrate: Some(155000),
        content_length: Some("2400000".to_string()),
        init_range: Some(ByteRange { start: "0".to_string(), end: "650".to_string() }),
        index_range: Some(ByteRange { start: "651".to_string(), end: "1200".to_string() }),
        signature_cipher: None,
        cipher: None,
    };

    let mpd = DashManifestBuilder::build(120, &[video_stream], &[audio_stream]);
    assert!(mpd.contains("<MPD"));
    assert!(mpd.contains("mediaPresentationDuration=\"PT120S\""));
    assert!(mpd.contains("width=\"3840\" height=\"2160\""));
    assert!(mpd.contains("audioSamplingRate=\"48000\""));
    assert!(mpd.contains("https://example.com/video_4k.mp4"));
    assert!(mpd.contains("https://example.com/audio_opus.webm"));
}
