//! Integration test: create a real hidden window on Windows, set it topmost
//! through the production [`RealWindowApi`], and verify the WS_EX_TOPMOST bit.
//!
//! Skipped on non-Windows.

#![cfg(windows)]

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongW, RegisterClassExW,
    UnregisterClassW, GWL_EXSTYLE, WNDCLASSEXW, WS_EX_TOPMOST, WS_OVERLAPPEDWINDOW,
};

use pin::win::{RealWindowApi, WindowApi};

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

fn ensure_class() {
    unsafe {
        let hinst = GetModuleHandleW(None).expect("module handle");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(test_wnd_proc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // Ignore failure (class may already exist between tests in the same proc).
        let _ = RegisterClassExW(&wc);
    }
}

fn create_test_window() -> HWND {
    ensure_class();
    unsafe {
        let hinst = GetModuleHandleW(None).expect("module handle");
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
            hinst,
            None,
        )
        .expect("CreateWindowExW")
    }
}

fn ex_style(hwnd: HWND) -> u32 {
    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 }
}

#[test]
fn pin_then_unpin_toggles_wsex_topmost() {
    let hwnd = create_test_window();
    let api = RealWindowApi;

    assert_eq!(ex_style(hwnd) & WS_EX_TOPMOST.0, 0, "should start non-topmost");

    api.set_topmost(hwnd.into(), true).expect("set topmost");
    assert_ne!(ex_style(hwnd) & WS_EX_TOPMOST.0, 0, "should now be topmost");

    api.set_topmost(hwnd.into(), false).expect("clear topmost");
    assert_eq!(ex_style(hwnd) & WS_EX_TOPMOST.0, 0, "should be non-topmost again");

    unsafe {
        let _ = DestroyWindow(hwnd);
        let hinst = GetModuleHandleW(None).expect("module handle");
        let _ = UnregisterClassW(CLASS, hinst);
    }
}

#[test]
fn window_rect_returns_nonempty() {
    let hwnd = create_test_window();
    let api = RealWindowApi;
    let r = api.window_rect(hwnd.into()).expect("rect");
    assert!(r.width() > 0);
    assert!(r.height() > 0);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
}
