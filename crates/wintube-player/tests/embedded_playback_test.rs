use wintube_innertube::InnertubeClient;
use wintube_player::{MediaEngine, PlaybackState};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, HMENU, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

unsafe extern "system" fn test_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// End-to-end proof of in-app embedded playback: real YouTube streams, real
/// mpv `--wid` child surface inside a WinTube-titled window, real IPC
/// telemetry (position advancing) and real seek control.
///
/// Run manually (opens a window and needs network):
/// `cargo test -p wintube-player --test embedded_playback_test -- --ignored --nocapture`
#[ignore = "opens a window, launches mpv and hits the live CDN"]
#[tokio::test(flavor = "multi_thread")]
async fn embedded_viewport_plays_inside_app_window() {
    // 1. Create a host top-level window titled like the app
    let hinstance = unsafe { GetModuleHandleW(None).unwrap() };
    let class_name = windows::core::w!("WinTubeEmbeddedTestHost");
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(test_wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };
    unsafe { RegisterClassExW(&wc) };
    let host = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            windows::core::w!("WinTube Embedded Playback Test"),
            WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 | WS_VISIBLE.0),
            100,
            100,
            1200,
            800,
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
        .expect("host window")
    };

    // 2. Resolve real YouTube streams (same pipeline as the app)
    let client = InnertubeClient::new();
    let info = client.get_video_playback_info("dQw4w9WgXcQ").await.unwrap();
    let video = info
        .video_streams
        .iter()
        .find(|s| s.height.unwrap_or(0) <= 720)
        .or_else(|| info.video_streams.first())
        .expect("video stream")
        .clone();
    let audio = info
        .audio_streams
        .iter()
        .max_by_key(|a| a.bitrate)
        .expect("audio stream")
        .clone();

    // 3. Launch the embedded in-app viewport
    let engine = MediaEngine::new();
    engine.load_and_play(
        "Embedded Test",
        &video.url,
        Some(&audio.url),
        info.metadata.duration_seconds as f64,
        0.0,
    );
    let label = engine
        .launch_embedded_viewport(false)
        .expect("embedded viewport launch");
    println!("viewport: {label}");
    engine.set_volume(5.0);

    // 4. Real mpv telemetry: IPC live + position advancing
    std::thread::sleep(std::time::Duration::from_secs(5));
    assert!(engine.is_embedded_ipc_live(), "mpv IPC must be live");
    assert!(engine.is_child_alive(), "mpv process must be running");
    let (state1, p1) = engine.tick(0.0);
    std::thread::sleep(std::time::Duration::from_secs(2));
    let (state2, p2) = engine.tick(0.0);
    println!(
        "state {state1:?} -> {state2:?}, position {:.2} -> {:.2}",
        p1.position_secs, p2.position_secs
    );
    assert!(matches!(
        state2,
        PlaybackState::Playing | PlaybackState::Paused
    ));
    assert!(
        p2.position_secs > p1.position_secs,
        "real mpv position must advance (in-app playback)"
    );

    // 5. In-app seek must reach mpv
    engine.seek_to(30.0);
    std::thread::sleep(std::time::Duration::from_secs(1));
    let (_, p3) = engine.tick(0.0);
    println!("after seek: {:.2}", p3.position_secs);
    assert!(
        (p3.position_secs - 30.0).abs() < 3.0,
        "seek command must take effect in mpv"
    );

    engine.stop_external_player();
    unsafe {
        let _ = DestroyWindow(host);
    }
}
