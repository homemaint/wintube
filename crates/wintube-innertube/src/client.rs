use serde::{Deserialize, Serialize};

/// Represents the YouTube client context spoofed by Innertube.
/// Mirrors `com.liskovsoft.youtubeapi.common.helpers.AppClient` in SmartTube.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppClient {
    /// YouTube TV HTML5 client (used by SmartTube for high-bitrate DASH manifests)
    TvHtml5,
    /// Android VR / Quest client (often bypasses age restrictions and poToken requirements)
    AndroidVr,
    /// iOS Mobile client (clean HLS and MP4 streams)
    Ios,
    /// Standard Desktop Web client (rich renderer data for comments, browse, and search)
    Web,
    /// Mobile Web client
    MWeb,
    /// Android Embedded client
    AndroidEmbed,
    /// Apple Vision Pro client (`VISIONOS`).
    ///
    /// Delivers un-attested progressive/DASH streams that are **not** subject
    /// to the ~32 MiB truncation applied to anonymous IOS/ANDROID_VR media
    /// requests, which makes it the preferred client for playback.
    VisionOs,
}

impl AppClient {
    pub fn client_name(&self) -> &'static str {
        match self {
            AppClient::TvHtml5 => "TVHTML5",
            AppClient::AndroidVr => "ANDROID_VR",
            AppClient::Ios => "IOS",
            AppClient::Web => "WEB",
            AppClient::MWeb => "MWEB",
            AppClient::AndroidEmbed => "ANDROID_EMBEDDED_PLAYER",
            AppClient::VisionOs => "VISIONOS",
        }
    }

    pub fn client_version(&self) -> &'static str {
        match self {
            AppClient::TvHtml5 => "7.20260707.07.00",
            AppClient::AndroidVr => "1.65.10",
            AppClient::Ios => "20.03.02",
            AppClient::Web => "2.20240910.05.00",
            AppClient::MWeb => "2.20240910.05.00",
            AppClient::AndroidEmbed => "19.38.3",
            AppClient::VisionOs => "1.02",
        }
    }

    pub fn client_id(&self) -> u32 {
        match self {
            AppClient::TvHtml5 => 31,
            AppClient::AndroidVr => 28,
            AppClient::Ios => 5,
            AppClient::Web => 1,
            AppClient::MWeb => 2,
            AppClient::AndroidEmbed => 56,
            AppClient::VisionOs => 101,
        }
    }

    pub fn user_agent(&self) -> &'static str {
        match self {
            AppClient::TvHtml5 => "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)",
            AppClient::AndroidVr => "com.google.android.apps.youtube.vr.oculus/1.65.10 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
            AppClient::Ios => "com.google.ios.youtube/20.03.02 (iPhone14,3; U; CPU iOS 17_5_1 like Mac OS X; en_US)",
            AppClient::Web => "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36",
            AppClient::MWeb => "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1",
            AppClient::AndroidEmbed => "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Mobile Safari/537.36",
            AppClient::VisionOs => "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
        }
    }

    pub fn referer(&self) -> Option<&'static str> {
        match self {
            AppClient::TvHtml5 => Some("https://www.youtube.com/tv"),
            AppClient::Web => Some("https://www.youtube.com/"),
            AppClient::MWeb => Some("https://m.youtube.com/"),
            _ => None,
        }
    }
}
