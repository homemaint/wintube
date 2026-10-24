use crate::models::FormatStream;

/// Generates an in-memory MPEG-DASH MPD manifest from Innertube's `adaptive_formats`.
/// This replicates SmartTube's `DashBuilder` (`com.liskovsoft.smartyoutubetv2.common.exoplayer.selector`),
/// allowing libmpv / media engines to seamlessly synchronize separate 4K/8K video & Opus audio tracks.
pub struct DashManifestBuilder;

impl DashManifestBuilder {
    pub fn build(
        duration_seconds: u64,
        video_formats: &[FormatStream],
        audio_formats: &[FormatStream],
    ) -> String {
        let mut mpd = String::new();
        mpd.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        mpd.push_str(&format!(
            r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011" type="static" mediaPresentationDuration="PT{}S" minBufferTime="PT1.5S">"#,
            duration_seconds
        ));
        mpd.push_str("\n  <Period>\n");

        // Video Adaptation Set
        if !video_formats.is_empty() {
            mpd.push_str(r#"    <AdaptationSet mimeType="video/mp4" contentType="video" subsegmentAlignment="true">"#);
            mpd.push('\n');
            for format in video_formats {
                if let Some(ref url) = format.url {
                    let width = format.width.unwrap_or(1920);
                    let height = format.height.unwrap_or(1080);
                    let fps = format.fps.unwrap_or(30);
                    let bitrate = format.bitrate;
                    let itag = format.itag;

                    mpd.push_str(&format!(
                        r#"      <Representation id="{itag}" bandwidth="{bitrate}" width="{width}" height="{height}" frameRate="{fps}">"#,
                    ));
                    mpd.push('\n');

                    if let (Some(ref init), Some(ref index)) = (&format.init_range, &format.index_range) {
                        mpd.push_str(&format!(
                            r#"        <BaseURL>{url}</BaseURL>
        <SegmentBase indexRange="{index_start}-{index_end}">
          <Initialization range="{init_start}-{init_end}"/>
        </SegmentBase>"#,
                            url = quick_xml_escape(url),
                            index_start = index.start,
                            index_end = index.end,
                            init_start = init.start,
                            init_end = init.end,
                        ));
                    } else {
                        mpd.push_str(&format!(
                            "        <BaseURL>{}</BaseURL>",
                            quick_xml_escape(url)
                        ));
                    }
                    mpd.push_str("\n      </Representation>\n");
                }
            }
            mpd.push_str("    </AdaptationSet>\n");
        }

        // Audio Adaptation Set
        if !audio_formats.is_empty() {
            mpd.push_str(r#"    <AdaptationSet mimeType="audio/mp4" contentType="audio" subsegmentAlignment="true">"#);
            mpd.push('\n');
            for format in audio_formats {
                if let Some(ref url) = format.url {
                    let bitrate = format.bitrate;
                    let itag = format.itag;
                    let sample_rate = format.audio_sample_rate.as_deref().unwrap_or("48000");

                    mpd.push_str(&format!(
                        r#"      <Representation id="{itag}" bandwidth="{bitrate}" audioSamplingRate="{sample_rate}">"#,
                    ));
                    mpd.push('\n');

                    if let (Some(ref init), Some(ref index)) = (&format.init_range, &format.index_range) {
                        mpd.push_str(&format!(
                            r#"        <BaseURL>{url}</BaseURL>
        <SegmentBase indexRange="{index_start}-{index_end}">
          <Initialization range="{init_start}-{init_end}"/>
        </SegmentBase>"#,
                            url = quick_xml_escape(url),
                            index_start = index.start,
                            index_end = index.end,
                            init_start = init.start,
                            init_end = init.end,
                        ));
                    } else {
                        mpd.push_str(&format!(
                            "        <BaseURL>{}</BaseURL>",
                            quick_xml_escape(url)
                        ));
                    }
                    mpd.push_str("\n      </Representation>\n");
                }
            }
            mpd.push_str("    </AdaptationSet>\n");
        }

        mpd.push_str("  </Period>\n</MPD>\n");
        mpd
    }
}

fn quick_xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
