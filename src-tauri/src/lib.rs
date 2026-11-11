#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod relay;

use std::sync::Mutex;

/// Append a line to `%APPDATA%\Wintube\wintube.log`.
///
/// The installed app has no console to look at, so the log is the only way to
/// see what a sidecar invocation actually did (resolved path, exit code,
/// stderr). Cheap enough to call on every yt-dlp run.
pub fn log_line(msg: &str) {
    use std::io::Write;
    let Some(base) = dirs::data_dir() else { return };
    let dir = base.join("Wintube");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("wintube.log"))
    else {
        return;
    };
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let _ = writeln!(f, "[{ts}] {msg}");
}

pub struct AppState {
    pub db: Mutex<Option<wintube_storage::Db>>,
    pub downloads: wintube_download::DownloadManager,
    pub settings: Mutex<wintube_core::AppSettings>,
}

pub fn main() {
    let db_path = dirs::data_dir()
        .map(|p| p.join("Wintube").join("wintube.db").to_string_lossy().to_string())
        .unwrap_or_else(|| "wintube.db".into());
    if let Some(parent) = std::path::Path::new(&db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let db = wintube_storage::Db::open(&db_path).ok();
    let settings = db.as_ref().map(|d| d.load_settings()).unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            db: Mutex::new(db),
            downloads: wintube_download::DownloadManager::new(),
            settings: Mutex::new(settings),
        })
        .register_asynchronous_uri_scheme_protocol("wintube-media", |_ctx, request, responder| {
            crate::relay::handle(request, responder);
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_play_info,
            commands::get_playback_streams,
            commands::log_line,
            commands::search_videos,
            commands::get_history,
            commands::save_history,
            commands::get_settings,
            commands::save_settings,
            commands::start_download,
            commands::list_downloads,
            commands::cancel_download,
            commands::ensure_sidecars,
            commands::sidecar_diagnostics,
            commands::remux_download,
            commands::get_sponsor_segments,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Wintube");
}
