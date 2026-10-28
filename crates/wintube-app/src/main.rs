#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod relay;

use app::WinTubeApp;

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("WinTube starting (Iced + wgpu GPU UI)");

    iced::application(WinTubeApp::title, WinTubeApp::update, WinTubeApp::view)
        .subscription(WinTubeApp::subscription)
        .theme(WinTubeApp::theme)
        .window_size(iced::Size::new(1440.0, 900.0))
        .antialiasing(true)
        .run_with(WinTubeApp::new)
}
