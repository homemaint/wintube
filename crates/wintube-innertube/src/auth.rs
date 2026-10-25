use serde::{Deserialize, Serialize};
use tracing::{debug, info};

/// Implements Google TV OAuth2 Device Code authentication flow (`youtube.com/activate`).
/// Mirrors SmartTube's `SignInService` in `MediaServiceCore` so users can sign in
/// using an 8-character code without needing Google Play Services or browser cookies.
const DEVICE_CODE_URL: &str = "https://www.youtube.com/o/oauth2/device/code";
const TOKEN_URL: &str = "https://www.youtube.com/o/oauth2/token";

// Standard YouTube TV HTML5 OAuth client credentials used by TV clients
const TV_CLIENT_ID: &str = "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com";
const TV_CLIENT_SECRET: &str = "SboVhoG9s0rNafixCSGGKXAT";
const OAUTH_SCOPE: &str = "http://gdata.youtube.com https://www.googleapis.com/auth/youtube-paid-content";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAuthCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthToken {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: u64,
    pub token_type: String,
}

#[derive(Clone)]
pub struct SignInService {
    http: reqwest::Client,
}

impl SignInService {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Requests a new user activation code (`user_code`) to display on screen for `youtube.com/activate`.
    pub async fn request_device_code(&self) -> anyhow::Result<DeviceAuthCode> {
        let params = [
            ("client_id", TV_CLIENT_ID),
            ("scope", OAUTH_SCOPE),
        ];

        debug!("Requesting YouTube TV OAuth2 device code...");
        let res = self
            .http
            .post(DEVICE_CODE_URL)
            .form(&params)
            .send()
            .await?;

        let code: DeviceAuthCode = res.json().await?;
        info!("Received TV Activation Code: {} (Verify at {})", code.user_code, code.verification_url);
        Ok(code)
    }

    /// Polls for the OAuth2 token once the user enters the code at `youtube.com/activate`.
    pub async fn poll_for_token(&self, device_code: &str) -> anyhow::Result<Option<OAuthToken>> {
        let params = [
            ("client_id", TV_CLIENT_ID),
            ("client_secret", TV_CLIENT_SECRET),
            ("code", device_code),
            ("grant_type", "http://oauth.net/grant_type/device/1.0"),
        ];

        let res = self.http.post(TOKEN_URL).form(&params).send().await?;
        let val: serde_json::Value = res.json().await?;

        if val.get("error").is_some() {
            // Usually "authorization_pending" or "slow_down"
            return Ok(None);
        }

        let token: OAuthToken = serde_json::from_value(val)?;
        Ok(Some(token))
    }

    /// Refreshes an expired `access_token` using the stored `refresh_token`.
    pub async fn refresh_access_token(&self, refresh_token: &str) -> anyhow::Result<OAuthToken> {
        let params = [
            ("client_id", TV_CLIENT_ID),
            ("client_secret", TV_CLIENT_SECRET),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let res = self.http.post(TOKEN_URL).form(&params).send().await?;
        let mut token: OAuthToken = res.json().await?;
        // Refresh responses omit the refresh token; keep the one we already have.
        if token.refresh_token.is_none() {
            token.refresh_token = Some(refresh_token.to_string());
        }
        Ok(token)
    }

    /// Polls until the user approves the code at `youtube.com/activate` (or it expires).
    pub async fn wait_for_token(&self, code: &DeviceAuthCode) -> anyhow::Result<OAuthToken> {
        let interval = code.interval.max(3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(code.expires_in.max(60));
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
            if let Some(token) = self.poll_for_token(&code.device_code).await? {
                return Ok(token);
            }
        }
        anyhow::bail!("Activation code expired before it was entered")
    }

    fn token_path() -> std::path::PathBuf {
        let base = std::env::var("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("."));
        base.join("WinTube").join("auth.json")
    }

    /// Persists the OAuth token locally so sign-in survives restarts.
    pub fn save_token(token: &OAuthToken) {
        let path = Self::token_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string(token) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn load_token() -> Option<OAuthToken> {
        let data = std::fs::read_to_string(Self::token_path()).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn clear_token() {
        let _ = std::fs::remove_file(Self::token_path());
    }

    /// Loads a saved token and refreshes it into a fresh access token.
    pub async fn restore_session(&self) -> Option<OAuthToken> {
        let saved = Self::load_token()?;
        let refresh = saved.refresh_token.clone()?;
        match self.refresh_access_token(&refresh).await {
            Ok(t) if !t.access_token.is_empty() => {
                Self::save_token(&t);
                Some(t)
            }
            _ => None,
        }
    }
}
