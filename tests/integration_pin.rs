//! Integration test: create real hidden windows on Windows and drive them
//! through the production [`RealWindowApi`] and window queries.
//!
//! Skipped on non-Windows.

#![cfg(windows)]

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongW, RegisterClassExW, GWL_EXSTYLE,
    WNDCLASSEXW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
};

use pin::domain::window::WindowApi;
use pin::win::api::{frame_bounds, is_window};
use pin::win::RealWindowApi;

const CLASS: PCWSTR = w!("PinTestWindowClass");

/// Trampoline because windows-rs's `DefWindowProcW` is an `unsafe fn`, not
/// `unsafe extern "system" fn`, and `WNDCLASSEXW::lpfnWndProc` requires the
/// latter.
unsafe extern "system" fn test_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Register the class exactly once per process and never unregister it:
/// tests run in parallel, so unregistering in one test races window
/// creation in another.
fn ensure_class() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| unsafe {
        let hinst = GetModuleHandleW(None).expect("module handle");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(test_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        assert_ne!(RegisterClassExW(&wc), 0, "RegisterClassExW");
    });
}

/// Hidden test window, destroyed on drop.
struct TestWindow(HWND);

impl TestWindow {
    fn new() -> Self {
        ensure_class();
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                CLASS,
                w!("pin-test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                200,
                100,
                None,
                None,
                GetModuleHandleW(None).expect("module handle"),
                None,
            )
            .expect("CreateWindowExW")
        };
        Self(hwnd)
    }

    fn ex_style(&self) -> u32 {
        unsafe { GetWindowLongW(self.0, GWL_EXSTYLE) as u32 }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

#[test]
fn pin_then_unpin_toggles_wsex_topmost() {
    let win = TestWindow::new();
    let api = RealWindowApi;

    assert_eq!(
        win.ex_style() & WS_EX_TOPMOST.0,
        0,
        "should start non-topmost"
    );

    api.set_topmost(win.0.into(), true).expect("set topmost");
    assert_ne!(win.ex_style() & WS_EX_TOPMOST.0, 0, "should now be topmost");

    api.set_topmost(win.0.into(), false).expect("clear topmost");
    assert_eq!(
        win.ex_style() & WS_EX_TOPMOST.0,
        0,
        "should be non-topmost again"
    );
}

#[test]
fn frame_bounds_returns_nonempty() {
    let win = TestWindow::new();
    let r = frame_bounds(win.0).expect("rect");
    assert!(r.width() > 0);
    assert!(r.height() > 0);
}

#[test]
fn is_window_tracks_destruction() {
    let win = TestWindow::new();
    let hwnd = win.0;
    assert!(RealWindowApi.is_window(hwnd.into()));
    drop(win);
    assert!(!is_window(hwnd));
    assert!(!is_window(HWND::default()));
}
