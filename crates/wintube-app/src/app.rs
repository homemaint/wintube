use std::collections::HashMap;
use std::time::Duration;

use iced::event::{self, Event};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{
    button, column, container, horizontal_rule, horizontal_space, image, progress_bar, row, scrollable,
    slider, text, text_input, vertical_rule, Column, Row,
};
use iced::{Alignment, Color, Element, Length, Subscription, Task, Theme};
use tracing::error;

use wintube_innertube::{
    DeviceAuthCode, InnertubeClient, OAuthToken, SignInService, VideoCardItem, VideoNextDetails,
    VideoPlaybackInfo,
};
use wintube_player::{
    MaxResolution, MediaEngine, PlaybackState, TrackSelector, VideoCodecPreference,
};
use wintube_services::{
    DeArrowService, DislikeStats, LocalSubscription, ReturnYouTubeDislikeService,
    SponsorBlockService, SponsorCategory, SponsorSegment, StorageEngine, WatchHistoryItem,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiFormFactor {
    /// High-density mouse & keyboard desktop layout
    Desktop,
    /// SmartTube-inspired 10-foot Leanback TV / Couch layout with spatial focus
    TvCouch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveNavTab {
    Home,
    Search,
    Subscriptions,
    History,
    NowPlaying,
    Settings,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Navigation & Mode Switching
    SwitchTab(ActiveNavTab),
    ToggleFormFactor,
    ToggleStatsForNerds,
    ToggleMiniPlayer,

    // Search & Autocomplete
    SearchQueryChanged(String),
    SubmitSearch,
    SelectSuggestion(String),
    SuggestionsLoaded(Vec<String>),
    FeedLoaded(Result<Vec<VideoCardItem>, String>),
    ThumbnailsLoaded(Vec<(String, Vec<u8>)>),

    // Video Selection & Pre-Resolving
    SelectVideo(String),
    FocusCardIndex(usize),
    VideoResolved(String, Result<LoadedVideoBundle, String>),

    // Player Transport & SmartTube Controls
    TogglePlayPause,
    SeekTo(f64),
    SeekRelative(f64),
    ChangeSpeed(f64),
    SetMaxResolution(MaxResolution),
    SetCodecPreference(VideoCodecPreference),
    ToggleSponsorBlock,
    ToggleAudioNormalization,
    LaunchExternalHardwareViewport,
    ExternalViewportResult(Result<String, String>),

    // Subscriptions & Account
    ToggleSubscribe {
        channel_id: String,
        channel_name: String,
    },
    RequestTvDeviceCode,
    TvDeviceCodeReceived(Result<DeviceAuthCode, String>),
    TvAuthCompleted(Result<OAuthToken, String>),
    CancelTvAuth,
    SignOut,
    SessionRestored(Option<OAuthToken>),

    // Background Tick & Keyboard/Controller Input
    PlayerTick,
    KeyboardEvent(Event),
}

#[derive(Debug, Clone)]
pub struct LoadedVideoBundle {
    pub playback: VideoPlaybackInfo,
    pub next: VideoNextDetails,
    pub ryd: Option<DislikeStats>,
    pub sponsor_segments: Vec<SponsorSegment>,
    pub clean_title: Option<String>,
}

pub struct WinTubeApp {
    // Core Services
    innertube: InnertubeClient,
    signin: SignInService,
    sponsorblock: SponsorBlockService,
    ryd: ReturnYouTubeDislikeService,
    dearrow: DeArrowService,
    storage: StorageEngine,
    player: MediaEngine,

    // UI State
    form_factor: UiFormFactor,
    active_tab: ActiveNavTab,
    focused_card_index: usize,
    is_loading_feed: bool,
    status_banner: String,

    // Search & Feed Data
    search_query: String,
    search_suggestions: Vec<String>,
    feed_videos: Vec<VideoCardItem>,
    subscriptions: Vec<LocalSubscription>,
    watch_history: Vec<WatchHistoryItem>,
    thumbnail_cache: HashMap<String, image::Handle>,
    http_client: reqwest::Client,

    // Pre-Resolve Cache & Active Video Playback
    resolved_cache: HashMap<String, LoadedVideoBundle>,
    active_video: Option<LoadedVideoBundle>,
    playback_state: PlaybackState,
    position_secs: f64,
    duration_secs: f64,
    playback_speed: f64,
    last_skipped_segment_uuid: Option<String>,
    skip_toast: Option<String>,

    // SmartTube Player & Quality Settings
    max_resolution: MaxResolution,
    codec_preference: VideoCodecPreference,
    sponsorblock_enabled: bool,
    dearrow_enabled: bool,
    audio_normalization: bool,
    show_stats_for_nerds: bool,
    mini_player_active: bool,
    active_decoder_label: String,

    // TV OAuth2 Device Activation State
    device_auth_code: Option<DeviceAuthCode>,
    is_signed_in: bool,
    auth_poll_active: bool,
}

impl WinTubeApp {
    pub fn new() -> (Self, Task<Message>) {
        let innertube = InnertubeClient::new();
        let signin = SignInService::new();
        let sponsorblock = SponsorBlockService::new();
        let ryd = ReturnYouTubeDislikeService::new();
        let dearrow = DeArrowService::new();

        let db_path = std::env::current_dir()
            .unwrap_or_default()
            .join("wintube_data.redb");
        let storage = StorageEngine::new(&db_path)
            .or_else(|_| StorageEngine::in_memory())
            .expect("Failed to initialize Redb storage");

        let subscriptions = storage.get_subscriptions().unwrap_or_default();
        let watch_history = storage.get_recent_history(50).unwrap_or_default();
        let player = MediaEngine::new();

        let initial_auth = SignInService::load_token();
        let is_signed_in = initial_auth.is_some();
        if let Some(ref tok) = initial_auth {
            innertube.set_auth_token_sync(Some(tok.access_token.clone()));
        }

        let http_client = reqwest::Client::builder()
            .gzip(true)
            .build()
            .unwrap_or_default();

        let client_clone = innertube.clone();
        let feed_task = Task::perform(
            async move {
                client_clone
                    .get_home_or_trending_feed()
                    .await
                    .map_err(|e| e.to_string())
            },
            Message::FeedLoaded,
        );

        let signin_clone = signin.clone();
        let restore_task = if is_signed_in {
            Task::perform(
                async move { signin_clone.restore_session().await },
                Message::SessionRestored,
            )
        } else {
            Task::none()
        };

        let initial_task = Task::batch([feed_task, restore_task]);

        let app = Self {
            innertube,
            signin,
            sponsorblock,
            ryd,
            dearrow,
            storage,
            player,
            form_factor: UiFormFactor::Desktop,
            active_tab: ActiveNavTab::Home,
            focused_card_index: 0,
            is_loading_feed: true,
            status_banner: if is_signed_in {
                "Signed in to YouTube TV (Bot protection bypassed) • Loading Feed...".to_string()
            } else {
                "Loading Trending & Home Feed via Innertube...".to_string()
            },
            search_query: String::new(),
            search_suggestions: Vec::new(),
            feed_videos: Vec::new(),
            subscriptions,
            watch_history,
            thumbnail_cache: HashMap::new(),
            http_client,
            resolved_cache: HashMap::new(),
            active_video: None,
            playback_state: PlaybackState::Stopped,
            position_secs: 0.0,
            duration_secs: 0.0,
            playback_speed: 1.0,
            last_skipped_segment_uuid: None,
            skip_toast: None,
            max_resolution: MaxResolution::Res2160p_4K,
            codec_preference: VideoCodecPreference::Vp9Preferred,
            sponsorblock_enabled: true,
            dearrow_enabled: true,
            audio_normalization: true,
            show_stats_for_nerds: true,
            mini_player_active: false,
            active_decoder_label: "D3D11VA Hardware Decoder Ready".to_string(),
            device_auth_code: None,
            is_signed_in,
            auth_poll_active: false,
        };

        (app, initial_task)
    }

    pub fn title(&self) -> String {
        let mode_str = match self.form_factor {
            UiFormFactor::Desktop => "Desktop Mode",
            UiFormFactor::TvCouch => "10-Foot TV Couch Mode",
        };
        if let Some(ref bundle) = self.active_video {
            format!("WinTube ({}) - ▶ {}", mode_str, bundle.playback.metadata.title)
        } else {
            format!("WinTube ({}) - High-Performance Rust YouTube Client", mode_str)
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::TokyoNightStorm
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let tick_sub = iced::time::every(Duration::from_millis(250)).map(|_| Message::PlayerTick);
        let key_sub = event::listen().map(Message::KeyboardEvent);
        Subscription::batch(vec![tick_sub, key_sub])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SwitchTab(tab) => {
                self.active_tab = tab;
                self.focused_card_index = 0;
                if tab == ActiveNavTab::Subscriptions {
                    self.subscriptions = self.storage.get_subscriptions().unwrap_or_default();
                } else if tab == ActiveNavTab::History {
                    self.watch_history = self.storage.get_recent_history(50).unwrap_or_default();
                } else if tab == ActiveNavTab::Home && self.feed_videos.is_empty() {
                    self.is_loading_feed = true;
                    let client = self.innertube.clone();
                    return Task::perform(
                        async move {
                            client
                                .get_home_or_trending_feed()
                                .await
                                .map_err(|e| e.to_string())
                        },
                        Message::FeedLoaded,
                    );
                }
                Task::none()
            }

            Message::ToggleFormFactor => {
                self.form_factor = match self.form_factor {
                    UiFormFactor::Desktop => UiFormFactor::TvCouch,
                    UiFormFactor::TvCouch => UiFormFactor::Desktop,
                };
                self.status_banner = match self.form_factor {
                    UiFormFactor::Desktop => "Switched to Desktop PC Mode (Mouse + Keyboard)".to_string(),
                    UiFormFactor::TvCouch => "Switched to SmartTube 10-Foot TV Mode (Use Arrow Keys / Enter / Backspace)".to_string(),
                };
                Task::none()
            }

            Message::ToggleStatsForNerds => {
                self.show_stats_for_nerds = !self.show_stats_for_nerds;
                Task::none()
            }

            Message::ToggleMiniPlayer => {
                self.mini_player_active = !self.mini_player_active;
                Task::none()
            }

            Message::SearchQueryChanged(q) => {
                self.search_query = q.clone();
                if q.trim().len() >= 2 {
                    let client = self.innertube.clone();
                    return Task::perform(
                        async move { client.search_suggestions(&q).await.unwrap_or_default() },
                        Message::SuggestionsLoaded,
                    );
                } else {
                    self.search_suggestions.clear();
                }
                Task::none()
            }

            Message::SuggestionsLoaded(suggestions) => {
                self.search_suggestions = suggestions.into_iter().take(6).collect();
                Task::none()
            }

            Message::SelectSuggestion(suggestion) => {
                self.search_query = suggestion;
                self.search_suggestions.clear();
                self.update(Message::SubmitSearch)
            }

            Message::SubmitSearch => {
                let q = self.search_query.trim().to_string();
                if q.is_empty() {
                    return Task::none();
                }
                self.search_suggestions.clear();
                self.is_loading_feed = true;
                self.active_tab = ActiveNavTab::Search;
                self.status_banner = format!("Searching Innertube for '{}'...", q);

                let client = self.innertube.clone();
                Task::perform(
                    async move { client.search_videos(&q).await.map_err(|e| e.to_string()) },
                    Message::FeedLoaded,
                )
            }

            Message::FeedLoaded(res) => {
                self.is_loading_feed = false;
                match res {
                    Ok(videos) => {
                        self.status_banner = format!("Loaded {} videos via Innertube", videos.len());
                        self.feed_videos = videos.clone();
                        self.focused_card_index = 0;

                        let dl_items: Vec<(String, String)> = videos
                            .iter()
                            .filter_map(|v| v.thumbnail_url.as_ref().map(|u| (v.video_id.clone(), u.clone())))
                            .collect();

                        let client = self.http_client.clone();
                        return Task::perform(
                            async move {
                                let mut results = Vec::new();
                                let mut set = tokio::task::JoinSet::new();
                                for (id, url) in dl_items {
                                    let c = client.clone();
                                    set.spawn(async move {
                                        if let Ok(res) = c.get(&url).send().await {
                                            if let Ok(bytes) = res.bytes().await {
                                                return Some((id, bytes.to_vec()));
                                            }
                                        }
                                        None
                                    });
                                }
                                while let Some(res) = set.join_next().await {
                                    if let Ok(Some((id, bytes))) = res {
                                        results.push((id, bytes));
                                    }
                                }
                                results
                            },
                            Message::ThumbnailsLoaded,
                        );
                    }
                    Err(err) => {
                        error!("Feed load error: {}", err);
                        self.status_banner = format!("Feed error: {}", err);
                    }
                }
                Task::none()
            }

            Message::ThumbnailsLoaded(items) => {
                for (id, bytes) in items {
                    self.thumbnail_cache.insert(id, image::Handle::from_bytes(bytes));
                }
                Task::none()
            }

            Message::FocusCardIndex(idx) => {
                if !self.feed_videos.is_empty() {
                    self.focused_card_index = idx.min(self.feed_videos.len() - 1);
                }
                Task::none()
            }

            Message::SelectVideo(video_id) => {
                // Instant playback if already pre-resolved in cache
                if let Some(bundle) = self.resolved_cache.get(&video_id).cloned() {
                    return self.activate_video_bundle(bundle);
                }

                self.status_banner = format!("Extracting 4K/8K DASH streams & SponsorBlock for {}...", video_id);
                self.active_tab = ActiveNavTab::NowPlaying;

                let client = self.innertube.clone();
                let ryd = self.ryd.clone();
                let sb = self.sponsorblock.clone();
                let dearrow = self.dearrow.clone();
                let vid = video_id.clone();

                Task::perform(
                    async move {
                        let (player_res, next_res, ryd_res, sb_res, da_res) = tokio::join!(
                            client.get_video_playback_info(&vid),
                            client.get_video_next(&vid),
                            ryd.get_votes(&vid),
                            sb.fetch_segments(&vid),
                            dearrow.get_branding(&vid),
                        );

                        let playback = player_res.map_err(|e| e.to_string())?;
                        let next = next_res.unwrap_or_default();
                        let clean_title = da_res
                            .ok()
                            .flatten()
                            .and_then(|b| b.titles.into_iter().next().map(|t| t.title));

                        Ok(LoadedVideoBundle {
                            playback,
                            next,
                            ryd: ryd_res.ok(),
                            sponsor_segments: sb_res.unwrap_or_default(),
                            clean_title,
                        })
                    },
                    move |res| Message::VideoResolved(video_id.clone(), res),
                )
            }

            Message::VideoResolved(video_id, res) => match res {
                Ok(bundle) => {
                    self.resolved_cache.insert(video_id, bundle.clone());
                    let activate_task = self.activate_video_bundle(bundle.clone());

                    let rel_items: Vec<(String, String)> = bundle
                        .next
                        .related_videos
                        .iter()
                        .filter_map(|v| v.thumbnail_url.as_ref().map(|u| (v.video_id.clone(), u.clone())))
                        .collect();

                    if !rel_items.is_empty() {
                        let client = self.http_client.clone();
                        let dl_task = Task::perform(
                            async move {
                                let mut results = Vec::new();
                                let mut set = tokio::task::JoinSet::new();
                                for (id, url) in rel_items {
                                    let c = client.clone();
                                    set.spawn(async move {
                                        if let Ok(res) = c.get(&url).send().await {
                                            if let Ok(bytes) = res.bytes().await {
                                                return Some((id, bytes.to_vec()));
                                            }
                                        }
                                        None
                                    });
                                }
                                while let Some(res) = set.join_next().await {
                                    if let Ok(Some((id, bytes))) = res {
                                        results.push((id, bytes));
                                    }
                                }
                                results
                            },
                            Message::ThumbnailsLoaded,
                        );
                        Task::batch([activate_task, dl_task])
                    } else {
                        activate_task
                    }
                }
                Err(err) => {
                    self.status_banner = if err.contains("not a bot") || err.contains("Sign in") {
                        "YouTube flagged this video as bot-protected for your network/IP. Try another video, or sign in via Settings (TV activation) and retry.".to_string()
                    } else {
                        format!("Playback extraction failed: {}", err)
                    };
                    Task::none()
                }
            },

            Message::TogglePlayPause => {
                self.playback_state = self.player.toggle_play_pause();
                Task::none()
            }

            Message::SeekTo(pos) => {
                self.player.seek_to(pos);
                self.position_secs = pos;
                Task::none()
            }

            Message::SeekRelative(delta) => {
                self.position_secs = self.player.seek_relative(delta);
                Task::none()
            }

            Message::ChangeSpeed(speed) => {
                self.playback_speed = self.player.set_speed(speed);
                self.status_banner = format!("Playback speed set to {:.2}x (pitch-corrected)", self.playback_speed);
                Task::none()
            }

            Message::SetMaxResolution(res) => {
                self.max_resolution = res;
                self.status_banner = format!("Locked maximum resolution to {:?}", res);
                Task::none()
            }

            Message::SetCodecPreference(codec) => {
                self.codec_preference = codec;
                self.status_banner = format!("Preferred hardware codec set to {:?}", codec);
                Task::none()
            }

            Message::ToggleSponsorBlock => {
                self.sponsorblock_enabled = !self.sponsorblock_enabled;
                Task::none()
            }

            Message::ToggleAudioNormalization => {
                self.audio_normalization = self.player.toggle_audio_normalization();
                Task::none()
            }

            Message::LaunchExternalHardwareViewport => {
                let player = self.player.clone();
                Task::perform(
                    async move { player.launch_hardware_viewport().map_err(|e| e.to_string()) },
                    Message::ExternalViewportResult,
                )
            }

            Message::ExternalViewportResult(res) => {
                match res {
                    Ok(engine_name) => {
                        self.active_decoder_label = engine_name.clone();
                        self.status_banner = format!("Launched Hardware Video Viewport: {}", engine_name);
                    }
                    Err(err) => {
                        self.status_banner = format!("Viewport note: {}", err);
                    }
                }
                Task::none()
            }

            Message::ToggleSubscribe {
                channel_id,
                channel_name,
            } => {
                let is_subbed = self.subscriptions.iter().any(|s| s.channel_id == channel_id);
                if is_subbed {
                    let _ = self.storage.remove_subscription(&channel_id);
                    self.status_banner = format!("Unsubscribed from {}", channel_name);
                } else {
                    let _ = self.storage.add_subscription(&channel_id, &channel_name, None);
                    self.status_banner = format!("Subscribed locally to {}", channel_name);
                }
                self.subscriptions = self.storage.get_subscriptions().unwrap_or_default();
                Task::none()
            }

            Message::RequestTvDeviceCode => {
                let signin = self.signin.clone();
                self.auth_poll_active = true;
                self.status_banner = "Requesting YouTube TV OAuth2 activation code...".to_string();
                Task::perform(
                    async move { signin.request_device_code().await.map_err(|e| e.to_string()) },
                    Message::TvDeviceCodeReceived,
                )
            }

            Message::TvDeviceCodeReceived(res) => {
                match res {
                    Ok(code) => {
                        self.status_banner = format!(
                            "Go to {} and enter code: {}",
                            code.verification_url, code.user_code
                        );
                        self.device_auth_code = Some(code.clone());
                        self.auth_poll_active = true;

                        let signin = self.signin.clone();
                        Task::perform(
                            async move { signin.wait_for_token(&code).await.map_err(|e| e.to_string()) },
                            Message::TvAuthCompleted,
                        )
                    }
                    Err(e) => {
                        self.status_banner = format!("OAuth code error: {}", e);
                        self.auth_poll_active = false;
                        Task::none()
                    }
                }
            }

            Message::TvAuthCompleted(res) => {
                self.auth_poll_active = false;
                match res {
                    Ok(token) => {
                        SignInService::save_token(&token);
                        let client = self.innertube.clone();
                        let access_tok = token.access_token.clone();
                        self.is_signed_in = true;
                        self.device_auth_code = None;
                        self.status_banner = "✅ Signed in to YouTube TV! Bot protection bypassed.".to_string();
                        Task::perform(
                            async move { client.set_auth_token(Some(access_tok)).await },
                            |_| Message::PlayerTick,
                        )
                    }
                    Err(e) => {
                        self.status_banner = format!("TV Activation note: {}", e);
                        self.device_auth_code = None;
                        Task::none()
                    }
                }
            }

            Message::CancelTvAuth => {
                self.auth_poll_active = false;
                self.device_auth_code = None;
                self.status_banner = "TV activation cancelled.".to_string();
                Task::none()
            }

            Message::SignOut => {
                SignInService::clear_token();
                self.is_signed_in = false;
                self.device_auth_code = None;
                let client = self.innertube.clone();
                self.status_banner = "Signed out of YouTube account.".to_string();
                Task::perform(
                    async move { client.set_auth_token(None).await },
                    |_| Message::PlayerTick,
                )
            }

            Message::SessionRestored(token_opt) => {
                if let Some(token) = token_opt {
                    SignInService::save_token(&token);
                    let client = self.innertube.clone();
                    let tok = token.access_token;
                    self.is_signed_in = true;
                    self.status_banner = "YouTube TV session restored from saved credentials.".to_string();
                    Task::perform(
                        async move { client.set_auth_token(Some(tok)).await },
                        |_| Message::PlayerTick,
                    )
                } else {
                    Task::none()
                }
            }

            Message::PlayerTick => {
                let (state, progress) = self.player.tick(0.25);
                self.playback_state = state;
                self.position_secs = progress.position_secs;
                self.duration_secs = progress.duration_secs;

                if state == PlaybackState::Playing {
                    if let Some(ref bundle) = self.active_video {
                        // 1. SponsorBlock Auto-Skip Controller
                        if self.sponsorblock_enabled {
                            if let Some(seg) = self
                                .sponsorblock
                                .find_active_segment(self.position_secs, &bundle.sponsor_segments)
                            {
                                if self.last_skipped_segment_uuid.as_deref() != Some(&seg.uuid)
                                    && (seg.end_time - self.position_secs) > 0.5
                                {
                                    self.last_skipped_segment_uuid = Some(seg.uuid.clone());
                                    self.player.seek_to(seg.end_time);
                                    self.position_secs = seg.end_time;
                                    let msg = format!(
                                        "⚡ SponsorBlock Auto-Skipped [{}] ({:.1}s → {:.1}s)",
                                        seg.category.as_str().to_uppercase(),
                                        seg.start_time,
                                        seg.end_time
                                    );
                                    self.skip_toast = Some(msg.clone());
                                    self.status_banner = msg;
                                }
                            }
                        }

                        // 2. Local Watch History & Resume Position Controller (every ~5 seconds)
                        if (self.position_secs as u64) % 5 == 0 {
                            let meta = &bundle.playback.metadata;
                            let _ = self.storage.update_playback_progress(
                                &meta.video_id,
                                &meta.title,
                                &meta.channel_id,
                                &meta.author,
                                meta.duration_seconds as i64,
                                self.position_secs as i64,
                            );
                        }
                    }
                }
                Task::none()
            }

            Message::KeyboardEvent(Event::Keyboard(keyboard::Event::KeyPressed {
                key, ..
            })) => self.handle_key_press(key),

            Message::KeyboardEvent(_) => Task::none(),
        }
    }

    fn activate_video_bundle(&mut self, bundle: LoadedVideoBundle) -> Task<Message> {
        let meta = &bundle.playback.metadata;
        let resume_pos = self
            .storage
            .get_resume_position(&meta.video_id)
            .ok()
            .flatten()
            .unwrap_or(0) as f64;

        let best_video = TrackSelector::select_best_video(
            &bundle.playback.video_streams,
            |s| s.height,
            |s| &s.codecs,
            self.max_resolution,
            self.codec_preference,
        );

        let best_audio = bundle
            .playback
            .audio_streams
            .iter()
            .max_by_key(|a| a.bitrate);

        let video_url = best_video
            .map(|v| v.url.as_str())
            .unwrap_or_default()
            .to_string();
        let audio_url = best_audio.map(|a| a.url.clone());

        let title = if self.dearrow_enabled {
            bundle
                .clean_title
                .as_deref()
                .unwrap_or(&meta.title)
                .to_string()
        } else {
            meta.title.clone()
        };

        self.player.load_and_play(
            &title,
            &video_url,
            audio_url.as_deref(),
            meta.duration_seconds as f64,
            resume_pos,
        );

        self.duration_secs = meta.duration_seconds as f64;
        self.position_secs = resume_pos;
        self.playback_state = PlaybackState::Playing;
        self.last_skipped_segment_uuid = None;
        self.skip_toast = None;

        // Record in history immediately
        let _ = self.storage.update_playback_progress(
            &meta.video_id,
            &meta.title,
            &meta.channel_id,
            &meta.author,
            meta.duration_seconds as i64,
            resume_pos as i64,
        );
        self.watch_history = self.storage.get_recent_history(50).unwrap_or_default();

        self.status_banner = format!(
            "Playing '{}' ({} SponsorBlock segments loaded)",
            title,
            bundle.sponsor_segments.len()
        );
        self.active_video = Some(bundle);
        self.active_tab = ActiveNavTab::NowPlaying;
        // Automatically launch hardware-accelerated video viewport!
        self.update(Message::LaunchExternalHardwareViewport)
    }

    fn handle_key_press(&mut self, key: Key) -> Task<Message> {
        match key.as_ref() {
            // F11 toggles Desktop <-> SmartTube 10-foot TV Couch Mode anywhere
            Key::Named(Named::F11) => return self.update(Message::ToggleFormFactor),

            // Player shortcuts when in NowPlaying
            Key::Character("k") | Key::Character(" ") if self.active_tab == ActiveNavTab::NowPlaying => {
                return self.update(Message::TogglePlayPause);
            }
            Key::Character("j") if self.active_tab == ActiveNavTab::NowPlaying => {
                return self.update(Message::SeekRelative(-10.0));
            }
            Key::Character("l") if self.active_tab == ActiveNavTab::NowPlaying => {
                return self.update(Message::SeekRelative(10.0));
            }
            Key::Character("m") | Key::Character("i") if self.active_tab == ActiveNavTab::NowPlaying => {
                return self.update(Message::ToggleMiniPlayer);
            }
            Key::Character("[") if self.active_tab == ActiveNavTab::NowPlaying => {
                let next_speed = (self.playback_speed - 0.25).max(0.25);
                return self.update(Message::ChangeSpeed(next_speed));
            }
            Key::Character("]") if self.active_tab == ActiveNavTab::NowPlaying => {
                let next_speed = (self.playback_speed + 0.25).min(4.0);
                return self.update(Message::ChangeSpeed(next_speed));
            }

            // Spatial D-Pad / Arrow Key Navigation
            Key::Named(Named::ArrowRight) => {
                if self.active_tab == ActiveNavTab::NowPlaying {
                    return self.update(Message::SeekRelative(5.0));
                } else if !self.feed_videos.is_empty() {
                    let next = (self.focused_card_index + 1).min(self.feed_videos.len() - 1);
                    self.focused_card_index = next;
                }
            }
            Key::Named(Named::ArrowLeft) => {
                if self.active_tab == ActiveNavTab::NowPlaying {
                    return self.update(Message::SeekRelative(-5.0));
                } else if self.focused_card_index > 0 {
                    self.focused_card_index -= 1;
                }
            }
            Key::Named(Named::ArrowDown) if self.active_tab != ActiveNavTab::NowPlaying => {
                let step = if self.form_factor == UiFormFactor::TvCouch { 2 } else { 3 };
                if !self.feed_videos.is_empty() {
                    self.focused_card_index = (self.focused_card_index + step).min(self.feed_videos.len() - 1);
                }
            }
            Key::Named(Named::ArrowUp) if self.active_tab != ActiveNavTab::NowPlaying => {
                let step = if self.form_factor == UiFormFactor::TvCouch { 2 } else { 3 };
                self.focused_card_index = self.focused_card_index.saturating_sub(step);
            }
            // Enter key selects focused video in BOTH Desktop and TV mode!
            Key::Named(Named::Enter)
                if self.active_tab == ActiveNavTab::Home || self.active_tab == ActiveNavTab::Search =>
            {
                if let Some(card) = self.feed_videos.get(self.focused_card_index) {
                    return self.update(Message::SelectVideo(card.video_id.clone()));
                }
            }
            Key::Named(Named::Escape) | Key::Named(Named::BrowserBack) => {
                if self.active_tab == ActiveNavTab::NowPlaying {
                    self.active_tab = ActiveNavTab::Home;
                }
            }
            _ => {}
        }
        Task::none()
    }

    // =========================================================================
    // VIEW LAYER (Dual-Mode Desktop + 10-Foot TV Couch UI)
    // =========================================================================

    pub fn view(&self) -> Element<'_, Message> {
        let is_tv = self.form_factor == UiFormFactor::TvCouch;
        let title_size = if is_tv { 26 } else { 20 };

        // Top Navigation Bar
        let top_bar = container(
            row![
                text(if is_tv { "📺 WinTube TV" } else { "▶ WinTube" })
                    .size(title_size)
                    .color(Color::from_rgb(1.0, 0.25, 0.25)),
                horizontal_space().width(20),
                text_input("Search YouTube (Ad-Free, SponsorBlock, 4K60)...", &self.search_query)
                    .on_input(Message::SearchQueryChanged)
                    .on_submit(Message::SubmitSearch)
                    .padding(if is_tv { 12 } else { 8 })
                    .size(if is_tv { 18 } else { 14 })
                    .width(Length::FillPortion(3)),
                button(text(if is_tv { "🔍 Search" } else { "Search" }).size(if is_tv { 18 } else { 14 }))
                    .on_press(Message::SubmitSearch)
                    .padding(if is_tv { 12 } else { 8 }),
                horizontal_space().width(Length::FillPortion(1)),
                button(
                    text(match self.form_factor {
                        UiFormFactor::Desktop => "🎮 Switch to TV Mode (F11)",
                        UiFormFactor::TvCouch => "🖥 Switch to Desktop Mode (F11)",
                    })
                    .size(if is_tv { 16 } else { 13 })
                )
                .on_press(Message::ToggleFormFactor)
                .padding(if is_tv { 10 } else { 7 }),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding(12);

        // Autocomplete Suggestions Bar
        let suggestions_bar: Element<'_, Message> = if !self.search_suggestions.is_empty() {
            let mut sug_row = Row::new().spacing(8).align_y(Alignment::Center);
            sug_row = sug_row.push(text("Suggestions:").size(12).color(Color::from_rgb(0.6, 0.6, 0.65)));
            for sug in &self.search_suggestions {
                sug_row = sug_row.push(
                    button(text(sug).size(12))
                        .on_press(Message::SelectSuggestion(sug.clone()))
                        .padding([4, 8]),
                );
            }
            container(sug_row).padding([4, 16]).into()
        } else {
            column![].into()
        };

        // Left Sidebar Navigation (SmartTube Categories)
        let nav_sidebar = self.view_sidebar(is_tv);

        // Main Content Area
        let main_content = match self.active_tab {
            ActiveNavTab::Home | ActiveNavTab::Search => self.view_video_grid(is_tv),
            ActiveNavTab::Subscriptions => self.view_subscriptions(is_tv),
            ActiveNavTab::History => self.view_history(is_tv),
            ActiveNavTab::NowPlaying => self.view_now_playing(is_tv),
            ActiveNavTab::Settings => self.view_settings(is_tv),
        };

        // Mini-Player Bar (when browsing while a video is active)
        let bottom_mini_player: Element<'_, Message> =
            if self.active_tab != ActiveNavTab::NowPlaying && self.active_video.is_some() {
                self.view_mini_player_bar()
            } else {
                column![].into()
            };

        // Status / Telemetry Footer
        let status_footer = container(
            row![
                text(&self.status_banner)
                    .size(12)
                    .color(Color::from_rgb(0.65, 0.72, 0.85)),
                horizontal_space(),
                text(format!(
                    "Engine: {} | Max: {:?} | SponsorBlock: {}",
                    self.active_decoder_label,
                    self.max_resolution,
                    if self.sponsorblock_enabled { "ON" } else { "OFF" }
                ))
                .size(12)
                .color(Color::from_rgb(0.5, 0.85, 0.6)),
            ]
            .align_y(Alignment::Center),
        )
        .padding([6, 14]);

        column![
            top_bar,
            suggestions_bar,
            horizontal_rule(1),
            row![nav_sidebar, vertical_rule(1), main_content].height(Length::Fill),
            bottom_mini_player,
            horizontal_rule(1),
            status_footer,
        ]
        .into()
    }

    fn view_sidebar(&self, is_tv: bool) -> Element<'_, Message> {
        let btn_size = if is_tv { 18 } else { 14 };
        let pad = if is_tv { 14 } else { 10 };
        let sidebar_width = if is_tv { 230 } else { 185 };

        let nav_btn = |label: &'static str, tab: ActiveNavTab| {
            let active = self.active_tab == tab;
            let prefix = if active { "▶ " } else { "   " };
            button(text(format!("{}{}", prefix, label)).size(btn_size))
                .on_press(Message::SwitchTab(tab))
                .padding(pad)
                .width(Length::Fill)
        };

        container(
            column![
                nav_btn("🔥 Home / Trending", ActiveNavTab::Home),
                nav_btn("🔍 Search Results", ActiveNavTab::Search),
                nav_btn("⭐ Subscriptions", ActiveNavTab::Subscriptions),
                nav_btn("🕒 Watch History", ActiveNavTab::History),
                nav_btn("🎬 Now Playing", ActiveNavTab::NowPlaying),
                nav_btn("⚙ SmartTube Settings", ActiveNavTab::Settings),
            ]
            .spacing(8),
        )
        .width(Length::Fixed(sidebar_width as f32))
        .padding(12)
        .into()
    }

    fn view_video_grid(&self, is_tv: bool) -> Element<'_, Message> {
        if self.is_loading_feed {
            return container(
                column![
                    text("⚡ Fetching Innertube Streams...").size(if is_tv { 26 } else { 18 }),
                    text("Zero Ads • SponsorBlock Ready • ReturnYouTubeDislike Active")
                        .size(14)
                        .color(Color::from_rgb(0.6, 0.65, 0.75)),
                ]
                .spacing(10)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }

        if self.feed_videos.is_empty() {
            return container(
                text("No videos found. Try searching above!").size(18),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }

        let columns_per_row = if is_tv { 2 } else { 3 };
        let mut grid_col = Column::new().spacing(16).padding(16);

        for (chunk_idx, chunk) in self.feed_videos.chunks(columns_per_row).enumerate() {
            let mut row_widget = Row::new().spacing(16).width(Length::Fill);

            for (col_idx, video) in chunk.iter().enumerate() {
                let card_idx = chunk_idx * columns_per_row + col_idx;
                let is_focused = card_idx == self.focused_card_index;

                let focus_badge = if is_focused {
                    text(" [FOCUSED - Press Enter] ")
                        .size(11)
                        .color(Color::from_rgb(0.2, 0.95, 0.5))
                } else {
                    text("").size(11)
                };

                let thumb_h = if is_tv { 190.0 } else { 150.0 };
                let thumbnail_view: Element<'_, Message> = if let Some(handle) = self.thumbnail_cache.get(&video.video_id) {
                    image(handle.clone())
                        .width(Length::Fill)
                        .height(Length::Fixed(thumb_h))
                        .content_fit(iced::ContentFit::Cover)
                        .into()
                } else {
                    container(
                        column![
                            text("▶").size(32).color(Color::from_rgb(0.6, 0.65, 0.8)),
                            text(if video.is_live { "🔴 LIVE" } else { &video.length_text })
                                .size(12)
                                .color(Color::from_rgb(0.8, 0.85, 0.95)),
                        ]
                        .spacing(6)
                        .align_x(Alignment::Center),
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(thumb_h))
                    .center_x(Length::Fill)
                    .center_y(Length::Fixed(thumb_h))
                    .style(|_| container::Style {
                        background: Some(Color::from_rgb(0.12, 0.14, 0.20).into()),
                        border: iced::border::rounded(6),
                        ..Default::default()
                    })
                    .into()
                };

                let card_content = column![
                    thumbnail_view,
                    row![
                        text(if video.is_live { "🔴 LIVE" } else { "🎬 VIDEO" })
                            .size(11)
                            .color(if video.is_live {
                                Color::from_rgb(1.0, 0.3, 0.3)
                            } else {
                                Color::from_rgb(0.5, 0.75, 1.0)
                            }),
                        horizontal_space(),
                        focus_badge,
                        text(&video.length_text)
                            .size(12)
                            .color(Color::from_rgb(0.9, 0.9, 0.2)),
                    ]
                    .align_y(Alignment::Center),
                    text(&video.title)
                        .size(if is_tv { 18 } else { 15 })
                        .color(Color::WHITE),
                    text(&video.channel_name)
                        .size(if is_tv { 14 } else { 13 })
                        .color(Color::from_rgb(0.65, 0.75, 0.9)),
                    row![
                        text(&video.view_count_text)
                            .size(12)
                            .color(Color::from_rgb(0.55, 0.6, 0.68)),
                        horizontal_space(),
                        text(&video.published_time_text)
                            .size(12)
                            .color(Color::from_rgb(0.55, 0.6, 0.68)),
                    ],
                    row![
                        button(
                            text(if is_tv { "▶ Watch (4K / Ad-Free)" } else { "▶ Watch Now" })
                                .size(if is_tv { 15 } else { 13 })
                        )
                        .on_press(Message::SelectVideo(video.video_id.clone()))
                        .padding([6, 12]),
                        horizontal_space(),
                        button(text("Focus").size(11))
                            .on_press(Message::FocusCardIndex(card_idx))
                            .padding([4, 8]),
                    ]
                    .align_y(Alignment::Center),
                ]
                .spacing(8)
                .padding(14);

                row_widget = row_widget.push(
                    button(card_content)
                        .on_press(Message::SelectVideo(video.video_id.clone()))
                        .padding(0)
                        .style(if is_focused {
                            button::primary
                        } else {
                            button::secondary
                        })
                        .width(Length::FillPortion(1)),
                );
            }

            // Fill remaining columns in the last row
            for _ in 0..(columns_per_row - chunk.len()) {
                row_widget = row_widget.push(horizontal_space().width(Length::FillPortion(1)));
            }

            grid_col = grid_col.push(row_widget);
        }

        scrollable(grid_col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_now_playing(&self, is_tv: bool) -> Element<'_, Message> {
        let Some(ref bundle) = self.active_video else {
            return container(
                column![
                    text("⏳ Resolving 4K/8K DASH Streams & SponsorBlock...")
                        .size(22)
                        .color(Color::from_rgb(0.3, 0.95, 0.6)),
                    text(&self.status_banner)
                        .size(15)
                        .color(Color::from_rgb(0.7, 0.8, 0.9)),
                    text("Decoupling adaptive video + high-bitrate Opus streams via Innertube engine...")
                        .size(13)
                        .color(Color::from_rgb(0.5, 0.6, 0.7)),
                    horizontal_rule(1),
                    row![
                        button(text("← Back to Browse Feed").size(14))
                            .on_press(Message::SwitchTab(ActiveNavTab::Home))
                            .padding([8, 16]),
                        button(text("Force Launch Viewport").size(14))
                            .on_press(Message::LaunchExternalHardwareViewport)
                            .padding([8, 16]),
                    ]
                    .spacing(12),
                ]
                .spacing(16)
                .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        };

        let meta = &bundle.playback.metadata;
        let display_title = if self.dearrow_enabled {
            bundle.clean_title.as_deref().unwrap_or(&meta.title)
        } else {
            &meta.title
        };

        let best_video = TrackSelector::select_best_video(
            &bundle.playback.video_streams,
            |s| s.height,
            |s| &s.codecs,
            self.max_resolution,
            self.codec_preference,
        );
        let best_audio = bundle
            .playback
            .audio_streams
            .iter()
            .max_by_key(|a| a.bitrate);

        let is_subscribed = self
            .subscriptions
            .iter()
            .any(|s| s.channel_id == meta.channel_id);

        // Video Viewport Surface + Stats for Nerds Overlay
        let stats_overlay: Element<'_, Message> = if self.show_stats_for_nerds {
            let v_info = best_video
                .map(|v| {
                    format!(
                        "Video: itag {} | {}x{} @ {}fps | Codec: {} | {:.2} Mbps",
                        v.itag,
                        v.width.unwrap_or(0),
                        v.height.unwrap_or(0),
                        v.fps.unwrap_or(30),
                        v.codecs,
                        (v.bitrate as f64) / 1_000_000.0
                    )
                })
                .unwrap_or_else(|| "Video: Adaptive Stream".to_string());

            let a_info = best_audio
                .map(|a| {
                    format!(
                        "Audio: itag {} | Codec: {} | {} Hz | {:.0} kbps | Norm: {}",
                        a.itag,
                        a.codecs,
                        a.audio_sample_rate.as_deref().unwrap_or("48000"),
                        (a.bitrate as f64) / 1000.0,
                        if self.audio_normalization { "ON (ebur128)" } else { "OFF" }
                    )
                })
                .unwrap_or_else(|| "Audio: Opus".to_string());

            container(
                column![
                    text("📊 SmartTube Stats for Nerds")
                        .size(13)
                        .color(Color::from_rgb(0.3, 0.95, 0.6)),
                    text(format!("Video ID: {} | Pipeline: {}", meta.video_id, self.active_decoder_label)).size(12),
                    text(v_info).size(12),
                    text(a_info).size(12),
                    text(format!(
                        "SponsorBlock Segments: {} | Speed: {:.2}x | SleepGuard: ACTIVE",
                        bundle.sponsor_segments.len(),
                        self.playback_speed
                    ))
                    .size(12),
                ]
                .spacing(4),
            )
            .padding(10)
            .into()
        } else {
            column![].into()
        };

        let viewport_surface = container(
            column![
                row![
                    text(match self.playback_state {
                        PlaybackState::Playing => "🟢 PLAYING (Hardware D3D11VA Pipeline)",
                        PlaybackState::Paused => "⏸ PAUSED",
                        PlaybackState::Ended => "⏹ ENDED",
                        _ => "⏳ READY",
                    })
                    .size(if is_tv { 20 } else { 16 })
                    .color(Color::from_rgb(0.3, 0.95, 0.5)),
                    horizontal_space(),
                    button(text("🗔 Popout Hardware Video Window (mpv / ffplay D3D11)").size(13))
                        .on_press(Message::LaunchExternalHardwareViewport)
                        .padding([6, 12]),
                    button(text(if self.show_stats_for_nerds { "Hide Stats" } else { "Stats for Nerds" }).size(12))
                        .on_press(Message::ToggleStatsForNerds)
                        .padding([6, 10]),
                    button(text(if self.mini_player_active { "Expanded View" } else { "Compact View" }).size(12))
                        .on_press(Message::ToggleMiniPlayer)
                        .padding([6, 10]),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                horizontal_rule(1),
                stats_overlay,
                if let Some(ref toast) = self.skip_toast {
                    text(toast)
                        .size(14)
                        .color(Color::from_rgb(0.2, 1.0, 0.5))
                } else {
                    text("Ad-Free Stream • No Pre-Roll / Mid-Roll Ads").size(12).color(Color::from_rgb(0.55, 0.65, 0.75))
                },
            ]
            .spacing(10),
        )
        .padding(16)
        .width(Length::Fill);

        // Seekbar & Transport Controls
        let transport_bar = container(
            column![
                row![
                    text(format_seconds(self.position_secs as u64)).size(13),
                    slider(
                        0.0..=(self.duration_secs.max(1.0)),
                        self.position_secs,
                        Message::SeekTo
                    )
                    .step(1.0)
                    .width(Length::Fill),
                    text(format_seconds(self.duration_secs as u64)).size(13),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
                row![
                    button(
                        text(if self.playback_state == PlaybackState::Playing {
                            "⏸ Pause (K)"
                        } else {
                            "▶ Play (K)"
                        })
                        .size(14)
                    )
                    .on_press(Message::TogglePlayPause)
                    .padding([8, 16]),
                    button(text("⏪ -10s (J)").size(13))
                        .on_press(Message::SeekRelative(-10.0))
                        .padding([8, 12]),
                    button(text("+10s ⏩ (L)").size(13))
                        .on_press(Message::SeekRelative(10.0))
                        .padding([8, 12]),
                    horizontal_space(),
                    text(format!("Speed: {:.2}x", self.playback_speed)).size(13),
                    button(text("0.75x").size(12))
                        .on_press(Message::ChangeSpeed(0.75))
                        .padding([5, 8]),
                    button(text("1.0x").size(12))
                        .on_press(Message::ChangeSpeed(1.0))
                        .padding([5, 8]),
                    button(text("1.25x").size(12))
                        .on_press(Message::ChangeSpeed(1.25))
                        .padding([5, 8]),
                    button(text("1.5x").size(12))
                        .on_press(Message::ChangeSpeed(1.5))
                        .padding([5, 8]),
                    button(text("2.0x").size(12))
                        .on_press(Message::ChangeSpeed(2.0))
                        .padding([5, 8]),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(10),
        )
        .padding(12);

        // ReturnYouTubeDislike + Channel Subscription Header
        let ryd_text = if let Some(ref votes) = bundle.ryd {
            format!(
                "👍 {}   👎 {} (Restored via RYD • {:.2}/5.0)",
                format_number(votes.likes),
                format_number(votes.dislikes),
                votes.rating
            )
        } else {
            "👍 Likes / 👎 Dislikes loading...".to_string()
        };

        let metadata_panel = container(
            column![
                text(display_title).size(if is_tv { 22 } else { 18 }).color(Color::WHITE),
                row![
                    text(format!("👤 {}", meta.author))
                        .size(15)
                        .color(Color::from_rgb(0.6, 0.8, 1.0)),
                    button(
                        text(if is_subscribed {
                            "✓ Subscribed (Local)"
                        } else {
                            "+ Subscribe (Local)"
                        })
                        .size(13)
                    )
                    .on_press(Message::ToggleSubscribe {
                        channel_id: meta.channel_id.clone(),
                        channel_name: meta.author.clone(),
                    })
                    .padding([6, 12]),
                    horizontal_space(),
                    text(format!("👁 {} views", format_number(meta.view_count))).size(13),
                    horizontal_space().width(16),
                    text(ryd_text)
                        .size(13)
                        .color(Color::from_rgb(0.95, 0.8, 0.35)),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        )
        .padding(12);

        // SponsorBlock Segments Timeline Panel
        let mut sb_col = Column::new().spacing(6);
        sb_col = sb_col.push(
            text(format!(
                "🛡 SponsorBlock Segments ({})",
                bundle.sponsor_segments.len()
            ))
            .size(15)
            .color(Color::from_rgb(0.3, 0.95, 0.6)),
        );
        if bundle.sponsor_segments.is_empty() {
            sb_col = sb_col.push(
                text("No crowd-reported sponsor segments in this video.")
                    .size(12)
                    .color(Color::from_rgb(0.6, 0.6, 0.65)),
            );
        } else {
            for seg in &bundle.sponsor_segments {
                sb_col = sb_col.push(
                    row![
                        text(format!(
                            "[{}] {} → {}",
                            seg.category.as_str().to_uppercase(),
                            format_seconds(seg.start_time as u64),
                            format_seconds(seg.end_time as u64)
                        ))
                        .size(12)
                        .color(sponsor_category_color(&seg.category)),
                        horizontal_space(),
                        button(text("Skip past").size(11))
                            .on_press(Message::SeekTo(seg.end_time))
                            .padding([3, 8]),
                    ]
                    .align_y(Alignment::Center),
                );
            }
        }

        // Related / Next Videos Sidebar
        let mut related_col = Column::new().spacing(8);
        related_col = related_col.push(text("⏭ Up Next / Related Videos").size(15));
        for rel in bundle.next.related_videos.iter().take(10) {
            let rel_thumb: Element<'_, Message> = if let Some(handle) = self.thumbnail_cache.get(&rel.video_id) {
                image(handle.clone())
                    .width(Length::Fixed(80.0))
                    .height(Length::Fixed(48.0))
                    .content_fit(iced::ContentFit::Cover)
                    .into()
            } else {
                container(text("▶").size(14).color(Color::from_rgb(0.6, 0.65, 0.8)))
                    .width(Length::Fixed(80.0))
                    .height(Length::Fixed(48.0))
                    .center_x(Length::Fixed(80.0))
                    .center_y(Length::Fixed(48.0))
                    .style(|_| container::Style {
                        background: Some(Color::from_rgb(0.12, 0.14, 0.20).into()),
                        border: iced::border::rounded(4),
                        ..Default::default()
                    })
                    .into()
            };

            related_col = related_col.push(
                container(
                    row![
                        rel_thumb,
                        column![
                            text(&rel.title).size(13),
                            text(format!("{} • {}", rel.channel_name, rel.length_text))
                                .size(11)
                                .color(Color::from_rgb(0.6, 0.65, 0.75)),
                        ]
                        .width(Length::Fill),
                        button(text("▶").size(13))
                            .on_press(Message::SelectVideo(rel.video_id.clone()))
                            .padding([6, 10]),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding(6),
            );
        }

        scrollable(
            column![
                viewport_surface,
                horizontal_rule(1),
                transport_bar,
                horizontal_rule(1),
                metadata_panel,
                horizontal_rule(1),
                row![
                    container(sb_col).width(Length::FillPortion(1)).padding(12),
                    vertical_rule(1),
                    container(related_col).width(Length::FillPortion(1)).padding(12),
                ]
                .spacing(12),
            ]
            .spacing(6),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn view_subscriptions(&self, is_tv: bool) -> Element<'_, Message> {
        let mut col = Column::new().spacing(12).padding(20);
        col = col.push(
            text("⭐ Local Privacy-Preserving Subscriptions")
                .size(if is_tv { 24 } else { 19 }),
        );
        col = col.push(
            text("Stored locally in pure-Rust Redb storage without requiring a Google Account.")
                .size(13)
                .color(Color::from_rgb(0.6, 0.65, 0.75)),
        );

        if self.subscriptions.is_empty() {
            col = col.push(
                text("You haven't subscribed to any channels yet. Open any video and click '+ Subscribe (Local)'.")
                    .size(14),
            );
        } else {
            for sub in &self.subscriptions {
                col = col.push(
                    container(
                        row![
                            text(format!("👤 {}", sub.channel_name)).size(16),
                            horizontal_space(),
                            button(text("Search Channel Videos").size(13))
                                .on_press(Message::SelectSuggestion(sub.channel_name.clone()))
                                .padding([6, 12]),
                            button(text("Unsubscribe").size(13))
                                .on_press(Message::ToggleSubscribe {
                                    channel_id: sub.channel_id.clone(),
                                    channel_name: sub.channel_name.clone(),
                                })
                                .padding([6, 12]),
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center),
                    )
                    .padding(10),
                );
            }
        }

        scrollable(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_history(&self, is_tv: bool) -> Element<'_, Message> {
        let mut col = Column::new().spacing(12).padding(20);
        col = col.push(
            text("🕒 Local Watch History & Resume Points")
                .size(if is_tv { 24 } else { 19 }),
        );

        if self.watch_history.is_empty() {
            col = col.push(text("No videos watched yet.").size(14));
        } else {
            for item in &self.watch_history {
                let pct = if item.duration_seconds > 0 {
                    (item.watched_seconds as f32 / item.duration_seconds as f32) * 100.0
                } else {
                    0.0
                };
                col = col.push(
                    container(
                        column![
                            row![
                                column![
                                    text(&item.title).size(15),
                                    text(format!(
                                        "{} • Resumes at {} / {}",
                                        item.channel_name,
                                        format_seconds(item.watched_seconds as u64),
                                        format_seconds(item.duration_seconds as u64)
                                    ))
                                    .size(12)
                                    .color(Color::from_rgb(0.65, 0.72, 0.85)),
                                ]
                                .width(Length::Fill),
                                button(text("▶ Resume").size(13))
                                    .on_press(Message::SelectVideo(item.video_id.clone()))
                                    .padding([6, 14]),
                            ]
                            .align_y(Alignment::Center),
                            progress_bar(0.0..=100.0, pct).height(4),
                        ]
                        .spacing(6),
                    )
                    .padding(10),
                );
            }
        }

        scrollable(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_settings(&self, is_tv: bool) -> Element<'_, Message> {
        let auth_section: Element<'_, Message> = if self.is_signed_in {
            column![
                row![
                    text("✅ Connected to YouTube TV").size(16).color(Color::from_rgb(0.2, 0.95, 0.5)),
                    horizontal_space(),
                    button(text("Sign Out").size(13))
                        .on_press(Message::SignOut)
                        .padding([6, 14]),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                text("Account session is active. TVHTML5 4K/8K DASH streams are fully unlocked and bot-check free.")
                    .size(13)
                    .color(Color::from_rgb(0.65, 0.75, 0.85)),
            ]
            .spacing(8)
            .into()
        } else if let Some(ref code) = self.device_auth_code {
            column![
                text(format!("1. Visit {} on your mobile phone or PC browser", code.verification_url)).size(15),
                row![
                    text("2. Enter TV Activation Code: ").size(16),
                    text(&code.user_code).size(26).color(Color::from_rgb(0.2, 0.95, 0.5)),
                ]
                .spacing(10)
                .align_y(Alignment::Center),
                row![
                    text(if self.auth_poll_active {
                        "⏳ Waiting for approval at youtube.com/activate..."
                    } else {
                        "Code generated."
                    })
                    .size(13)
                    .color(Color::from_rgb(0.95, 0.85, 0.3)),
                    horizontal_space(),
                    button(text("Cancel").size(12))
                        .on_press(Message::CancelTvAuth)
                        .padding([4, 10]),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(10)
            .into()
        } else {
            column![
                text("Sign in using Google's official TV device code (SmartTube method). No password or cookies needed.")
                    .size(13)
                    .color(Color::from_rgb(0.65, 0.75, 0.85)),
                button(text("🔑 Generate YouTube TV OAuth2 Activation Code").size(14))
                    .on_press(Message::RequestTvDeviceCode)
                    .padding([8, 14]),
            ]
            .spacing(8)
            .into()
        };

        let col = column![
            text("⚙ SmartTube Engine & Quality Settings").size(if is_tv { 24 } else { 20 }),
            horizontal_rule(1),
            text("Video Resolution Ceiling").size(16),
            row![
                button(text("1080p Full HD").size(13))
                    .on_press(Message::SetMaxResolution(MaxResolution::Res1080p))
                    .padding([6, 12]),
                button(text("1440p QHD").size(13))
                    .on_press(Message::SetMaxResolution(MaxResolution::Res1440p))
                    .padding([6, 12]),
                button(text("2160p 4K UHD (Recommended)").size(13))
                    .on_press(Message::SetMaxResolution(MaxResolution::Res2160p_4K))
                    .padding([6, 12]),
                button(text("4320p 8K").size(13))
                    .on_press(Message::SetMaxResolution(MaxResolution::Res4320p_8K))
                    .padding([6, 12]),
            ]
            .spacing(10),
            horizontal_rule(1),
            text("Hardware Video Codec Preference (D3D11VA)").size(16),
            row![
                button(text("AV1 (RTX 30/40, RX 6000+, Arc)").size(13))
                    .on_press(Message::SetCodecPreference(VideoCodecPreference::Av1Preferred))
                    .padding([6, 12]),
                button(text("VP9 (Universal 4K60 GPU Decode)").size(13))
                    .on_press(Message::SetCodecPreference(VideoCodecPreference::Vp9Preferred))
                    .padding([6, 12]),
                button(text("H.264 / AVC1 (Legacy Battery Saver)").size(13))
                    .on_press(Message::SetCodecPreference(VideoCodecPreference::Avc1Preferred))
                    .padding([6, 12]),
            ]
            .spacing(10),
            horizontal_rule(1),
            text("SmartTube Content Filtering & Audio Pipeline").size(16),
            row![
                button(
                    text(format!(
                        "SponsorBlock Auto-Skip: {}",
                        if self.sponsorblock_enabled { "ENABLED" } else { "DISABLED" }
                    ))
                    .size(13)
                )
                .on_press(Message::ToggleSponsorBlock)
                .padding([8, 14]),
                button(
                    text(format!(
                        "Loudness Normalization (ebur128): {}",
                        if self.audio_normalization { "ENABLED" } else { "DISABLED" }
                    ))
                    .size(13)
                )
                .on_press(Message::ToggleAudioNormalization)
                .padding([8, 14]),
            ]
            .spacing(12),
            horizontal_rule(1),
            text("Google TV Account Sign-In (Device Code Flow — No Play Services)").size(16),
            auth_section,
        ]
        .spacing(14)
        .padding(20);

        scrollable(col)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_mini_player_bar(&self) -> Element<'_, Message> {
        let Some(ref bundle) = self.active_video else {
            return column![].into();
        };

        container(
            row![
                text("🎬 Mini-Player:")
                    .size(13)
                    .color(Color::from_rgb(0.3, 0.95, 0.6)),
                text(&bundle.playback.metadata.title).size(13),
                horizontal_space(),
                text(format!(
                    "{} / {}",
                    format_seconds(self.position_secs as u64),
                    format_seconds(self.duration_secs as u64)
                ))
                .size(12),
                button(
                    text(if self.playback_state == PlaybackState::Playing {
                        "⏸"
                    } else {
                        "▶"
                    })
                    .size(13)
                )
                .on_press(Message::TogglePlayPause)
                .padding([4, 10]),
                button(text("Return to Player").size(12))
                    .on_press(Message::SwitchTab(ActiveNavTab::NowPlaying))
                    .padding([4, 10]),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([8, 16])
        .into()
    }
}

fn format_seconds(total_secs: u64) -> String {
    let hrs = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hrs > 0 {
        format!("{}:{:02}:{:02}", hrs, mins, secs)
    } else {
        format!("{:02}:{:02}", mins, secs)
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", (n as f64) / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", (n as f64) / 1_000.0)
    } else {
        n.to_string()
    }
}

fn sponsor_category_color(cat: &SponsorCategory) -> Color {
    match cat {
        SponsorCategory::Sponsor => Color::from_rgb(0.0, 0.85, 0.0),
        SponsorCategory::SelfPromo => Color::from_rgb(1.0, 1.0, 0.0),
        SponsorCategory::Interaction => Color::from_rgb(0.8, 0.3, 1.0),
        SponsorCategory::Intro => Color::from_rgb(0.0, 1.0, 1.0),
        SponsorCategory::Outro => Color::from_rgb(0.2, 0.4, 1.0),
        _ => Color::from_rgb(0.9, 0.6, 0.2),
    }
}
