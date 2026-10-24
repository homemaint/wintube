use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodecPreference {
    Av1Preferred,
    Vp9Preferred,
    Avc1Preferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaxResolution {
    Res720p,
    Res1080p,
    Res1440p,
    Res2160p_4K,
    Res4320p_8K,
}

impl MaxResolution {
    pub fn max_height(&self) -> u32 {
        match self {
            MaxResolution::Res720p => 720,
            MaxResolution::Res1080p => 1080,
            MaxResolution::Res1440p => 1440,
            MaxResolution::Res2160p_4K => 2160,
            MaxResolution::Res4320p_8K => 4320,
        }
    }
}

/// Smart track selection logic.
/// Automatically selects optimal video stream resolution and avoids unwanted auto-dubbed audio tracks.
pub struct TrackSelector;

impl TrackSelector {
    /// Selects the best video stream matching resolution ceiling and codec preference.
    pub fn select_best_video<'a, T>(
        streams: &'a [T],
        get_height: impl Fn(&T) -> Option<u32>,
        get_codecs: impl Fn(&T) -> &str,
        max_res: MaxResolution,
        codec_pref: VideoCodecPreference,
    ) -> Option<&'a T> {
        let ceiling = max_res.max_height();

        let eligible: Vec<&'a T> = streams
            .iter()
            .filter(|s| get_height(s).unwrap_or(0) <= ceiling)
            .collect();

        if eligible.is_empty() {
            return streams.first();
        }

        // Score by resolution + codec match
        let mut scored: Vec<(&'a T, u32)> = eligible
            .into_iter()
            .map(|s| {
                let h = get_height(s).unwrap_or(0);
                let codec = get_codecs(s);
                let mut score = h * 10;

                match codec_pref {
                    VideoCodecPreference::Av1Preferred if codec.starts_with("av01") => score += 50,
                    VideoCodecPreference::Vp9Preferred if codec.starts_with("vp9") => score += 50,
                    VideoCodecPreference::Avc1Preferred if codec.starts_with("avc1") => score += 50,
                    _ => {}
                }

                (s, score)
            })
            .collect();

        scored.sort_by_key(|&(_, score)| score);
        scored.last().map(|&(s, _)| s)
    }
}
