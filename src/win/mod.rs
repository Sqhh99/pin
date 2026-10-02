//! Thin Win32 wrappers. No application logic lives here: these modules turn
//! raw `windows`-crate calls into safe(ish) RAII types and domain values.

pub mod api;
pub mod cursor;
pub mod gdi;
pub mod instance;
pub mod registry;

pub use api::RealWindowApi;

use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{RegisterClassExW, HCURSOR, WNDCLASSEXW};

use crate::domain::geometry::Rect;
use crate::domain::window::WindowId;

impl From<HWND> for WindowId {
    fn from(h: HWND) -> Self {
        WindowId(h.0 as isize)
    }
}

impl From<WindowId> for HWND {
    fn from(w: WindowId) -> Self {
        HWND(w.0 as *mut _)
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

/// NUL-terminated UTF-16 copy of `s`, for `PCWSTR` parameters.
pub fn encode_wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

pub type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

/// Register a window class for this module. `class_name` must stay alive
/// for the lifetime of the process (use a `static`).
pub fn register_class(
    class_name: &'static [u16],
    wnd_proc: WndProc,
    cursor: HCURSOR,
) -> Result<()> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(anyhow!(
                "RegisterClassExW({}) failed: {}",
                String::from_utf16_lossy(class_name.strip_suffix(&[0]).unwrap_or(class_name)),
                windows::core::Error::from_win32()
            ));
        }
    }
    Ok(())
}

/// Opt into per-monitor-v2 DPI awareness so window rects and our own
/// windows use physical pixels. The embedded manifest normally does this
/// already (then this call fails harmlessly); it matters for builds where
/// the resource compiler was unavailable.
pub fn enable_per_monitor_dpi_awareness() {
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Read the pointer stashed in `GWLP_USERDATA`, or `None` if unset.
///
/// # Safety
/// The caller must know that `hwnd`'s user data, if non-zero, is a valid `*const T`.
pub unsafe fn user_data<'a, T>(hwnd: HWND) -> Option<&'a T> {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWLP_USERDATA};
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const T).as_ref()
}

/// Move `value` to the heap and stash it in `hwnd`'s `GWLP_USERDATA`.
/// Reclaim it with [`take_user_data`] in `WM_NCDESTROY`.
pub fn set_user_data<T>(hwnd: HWND, value: T) {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_USERDATA};
    let ptr = Box::into_raw(Box::new(value));
    unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize) };
}

/// Clear and return the value stored by [`set_user_data`].
///
/// # Safety
/// `hwnd`'s user data must be unset or have been set by `set_user_data::<T>`.
pub unsafe fn take_user_data<T>(hwnd: HWND) -> Option<Box<T>> {
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_USERDATA};
    let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut T;
    (!ptr.is_null()).then(|| Box::from_raw(ptr))
}
