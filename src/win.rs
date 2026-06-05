//! Thin abstraction over the Win32 calls we need for window pinning.
//!
//! Exposing a [`WindowApi`] trait lets us unit-test [`crate::pinned`] without
//! touching real HWNDs. The [`RealWindowApi`] implementation calls into the
//! `windows` crate.

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, RECT};

/// Opaque platform window handle. Wrapped so tests can fabricate values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(pub isize);

#[cfg(windows)]
impl From<HWND> for WindowId {
    fn from(h: HWND) -> Self {
        WindowId(h.0 as isize)
    }
}

#[cfg(windows)]
impl From<WindowId> for HWND {
    fn from(w: WindowId) -> Self {
        HWND(w.0 as *mut _)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[cfg(windows)]
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

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}

/// Operations the pinned-set logic needs from the OS. Mockable for tests.
pub trait WindowApi {
    fn set_topmost(&self, w: WindowId, on: bool) -> anyhow::Result<()>;
    fn window_rect(&self, w: WindowId) -> anyhow::Result<Rect>;
    fn is_window(&self, w: WindowId) -> bool;
}

#[cfg(windows)]
pub use real::RealWindowApi;

#[cfg(windows)]
pub mod real {
    use super::*;
    use anyhow::{anyhow, Result};
    use windows::Win32::Foundation::{GetLastError, HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, IsWindow, SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SET_WINDOW_POS_FLAGS,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    pub struct RealWindowApi;

    impl WindowApi for RealWindowApi {
        fn set_topmost(&self, w: WindowId, on: bool) -> Result<()> {
            let hwnd: HWND = w.into();
            let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let flags: SET_WINDOW_POS_FLAGS = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
            unsafe { SetWindowPos(hwnd, insert_after, 0, 0, 0, 0, flags) }
                .map_err(|e| anyhow!("SetWindowPos failed: {e}"))
        }

        fn window_rect(&self, w: WindowId) -> Result<Rect> {
            let hwnd: HWND = w.into();
            let mut r = RECT::default();
            unsafe { GetWindowRect(hwnd, &mut r) }
                .map_err(|e| anyhow!("GetWindowRect failed: {e}"))?;
            Ok(r.into())
        }

        fn is_window(&self, w: WindowId) -> bool {
            let hwnd: HWND = w.into();
            unsafe { IsWindow(hwnd) }.as_bool()
        }
    }

    /// Top-level window under the screen point, or None.
    pub fn top_level_window_at(pt_x: i32, pt_y: i32) -> Option<WindowId> {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT};
        let hwnd = unsafe { WindowFromPoint(POINT { x: pt_x, y: pt_y }) };
        if hwnd.0.is_null() {
            return None;
        }
        let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
        if root.0.is_null() {
            None
        } else {
            Some(root.into())
        }
    }

    // Silence unused-import warnings if features change.
    #[allow(dead_code)]
    fn _last_error_touch() {
        let _ = unsafe { GetLastError() };
    }
}
