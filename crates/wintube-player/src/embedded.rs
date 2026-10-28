//! Native in-app video embedding for Windows.
//!
//! Locates the WinTube top-level window, computes the deterministic video-band
//! rectangle (mirrored by the Iced layout), and owns a borderless child HWND
//! that mpv renders into via `--wid`.

use tracing::debug;

use windows::core::w;
use windows::Win32::Foundation::{BOOL, HINSTANCE, HWND, LPARAM, LRESULT, RECT};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::HMENU;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, EnumWindows, GetClientRect, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, RegisterClassExW, SetWindowPos, ShowWindow,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WNDCLASSEXW,
    WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// Shared layout contract (logical px). These MUST mirror the Iced view code.
pub const SIDEBAR_W_DESKTOP: f32 = 190.0;
pub const SIDEBAR_W_TV: f32 = 230.0;
pub const TOPBAR_H: f32 = 56.0;
pub const RULE_H: f32 = 1.0;
pub const FOOTER_H: f32 = 30.0;
/// Fixed height of the in-app video band on the Now Playing tab.
pub const VIDEO_BAND_H: f32 = 420.0;

const SURFACE_CLASS_NAME: windows::core::PCWSTR = w!("WinTubeVideoSurface");

/// Registers the black-background child window class once per process.
fn ensure_surface_class() {
    use std::sync::Once;
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| unsafe {
        let hinstance = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: Default::default(),
            lpfnWndProc: Some(surface_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance.into(),
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszMenuName: windows::core::PCWSTR::null(),
            lpszClassName: SURFACE_CLASS_NAME,
            hIconSm: Default::default(),
        };
        if RegisterClassExW(&wc) == 0 {
            // Already registered (e.g. second process start in same session) — safe to ignore.
            debug!("WinTubeVideoSurface class already registered or registration failed");
        }
    });
}

struct FindCtx {
    pid: u32,
    found: Option<HWND>,
}

unsafe extern "system" fn surface_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn enum_main_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindCtx);
    if IsWindowVisible(hwnd).as_bool() {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == ctx.pid {
            let mut buf = [0u16; 128];
            let len = GetWindowTextW(hwnd, &mut buf);
            let title = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
            if title.starts_with("WinTube") {
                ctx.found = Some(hwnd);
                return BOOL(0);
            }
        }
    }
    BOOL(1)
}

/// Finds the WinTube app's top-level window (this process, visible, title prefix "WinTube").
pub fn find_main_window_hwnd() -> Option<HWND> {
    let mut ctx = FindCtx {
        pid: std::process::id(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_main_window),
            LPARAM(&mut ctx as *mut FindCtx as isize),
        );
    }
    ctx.found
}

/// Computes the embedded video band rectangle in parent-client (physical) pixels.
/// Mirrors the Iced Now Playing layout: top bar + rule above, footer below,
/// sidebar + rule on the left, fixed-height 16:9-ish video band.
pub fn compute_video_band_rect(parent: HWND, is_tv: bool) -> Option<(i32, i32, i32, i32)> {
    let mut rc = RECT::default();
    unsafe { GetClientRect(parent, &mut rc).ok()? };

    let dpi = unsafe { GetDpiForWindow(parent) } as f32;
    let scale = if dpi > 0.0 { dpi / 96.0 } else { 1.0 };

    let sidebar_w = (if is_tv {
        SIDEBAR_W_TV
    } else {
        SIDEBAR_W_DESKTOP
    }) * scale;
    let x = sidebar_w.round() as i32 + (RULE_H * scale).round() as i32;
    let y = ((TOPBAR_H + RULE_H) * scale).round() as i32;
    let footer = (FOOTER_H * scale).round() as i32;

    let width = (rc.right - x).max(320);
    let band = (VIDEO_BAND_H * scale).round() as i32;
    let height = band.min((rc.bottom - y - footer).max(240));

    Some((x, y, width, height))
}

/// Borderless child window mpv renders into via `--wid`.
///
/// The HWND is stored as `isize` so `MediaEngine` stays `Send` (raw pointers
/// inside `HWND` would poison it across iced task boundaries).
pub struct EmbeddedSurface {
    hwnd: isize,
    pub last_rect: (i32, i32, i32, i32),
}

impl EmbeddedSurface {
    fn as_hwnd(&self) -> HWND {
        HWND(self.hwnd as *mut _)
    }

    /// Creates the child surface (or repositions the existing one) and returns
    /// its HWND for mpv's `--wid` argument. Must be called from the UI thread.
    pub fn ensure(
        existing: &mut Option<EmbeddedSurface>,
        parent: HWND,
        rect: (i32, i32, i32, i32),
    ) -> anyhow::Result<isize> {
        ensure_surface_class();
        match existing {
            Some(surface) => {
                surface.move_to(rect);
                Ok(surface.hwnd())
            }
            None => {
                let (x, y, w, h) = rect;
                let hwnd = unsafe {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        SURFACE_CLASS_NAME,
                        w!("WinTube Video Surface"),
                        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                        x,
                        y,
                        w.max(1),
                        h.max(1),
                        parent,
                        HMENU::default(),
                        HINSTANCE::default(),
                        None,
                    )
                    .map_err(|e| anyhow::anyhow!("CreateWindowExW failed: {e}"))?
                };
                *existing = Some(EmbeddedSurface {
                    hwnd: hwnd.0 as isize,
                    last_rect: rect,
                });
                Ok(hwnd.0 as isize)
            }
        }
    }

    pub fn move_to(&mut self, rect: (i32, i32, i32, i32)) {
        let (x, y, w, h) = rect;
        unsafe {
            let _ = SetWindowPos(
                self.as_hwnd(),
                HWND::default(),
                x,
                y,
                w.max(1),
                h.max(1),
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
        self.last_rect = rect;
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe {
            let _ = ShowWindow(self.as_hwnd(), if visible { SW_SHOW } else { SW_HIDE });
        }
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd
    }
}
