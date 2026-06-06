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

const WS_CHILD_BITS: u32 = 0x4000_0000;
const WS_CAPTION_BITS: u32 = 0x00C0_0000;
const WS_SYSMENU_BITS: u32 = 0x0008_0000;
const WS_THICKFRAME_BITS: u32 = 0x0004_0000;
const WS_EX_TOOLWINDOW_BITS: u32 = 0x0000_0080;
const WS_EX_APPWINDOW_BITS: u32 = 0x0004_0000;

const MIN_PINNABLE_WIDTH: i32 = 32;
const MIN_PINNABLE_HEIGHT: i32 = 32;

const NON_PINNABLE_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "DV2ControlHost",
    "MSTaskListWClass",
    "NotifyIconOverflowWindow",
    "PinPickerClass",
    "PinOverlayClass",
    "PinAppMsgWindow",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowCandidate {
    pub class_name: String,
    pub style: u32,
    pub ex_style: u32,
    pub rect: Rect,
    pub is_window: bool,
    pub is_visible: bool,
    pub is_iconic: bool,
    pub has_parent: bool,
}

/// Returns true when a picked HWND looks like a regular user-facing
/// application window. This intentionally excludes shell surfaces, tool
/// windows, our own helper windows, minimized windows, and tiny overlays.
pub fn is_pinnable_candidate(candidate: &WindowCandidate) -> bool {
    if !candidate.is_window
        || !candidate.is_visible
        || candidate.is_iconic
        || candidate.has_parent
        || candidate.rect.width() < MIN_PINNABLE_WIDTH
        || candidate.rect.height() < MIN_PINNABLE_HEIGHT
    {
        return false;
    }

    if candidate.style & WS_CHILD_BITS != 0 {
        return false;
    }

    if candidate.ex_style & WS_EX_TOOLWINDOW_BITS != 0 {
        return false;
    }

    if NON_PINNABLE_CLASSES
        .iter()
        .any(|name| name.eq_ignore_ascii_case(candidate.class_name.as_str()))
    {
        return false;
    }

    let has_app_window = candidate.ex_style & WS_EX_APPWINDOW_BITS != 0;
    let has_caption = candidate.style & WS_CAPTION_BITS == WS_CAPTION_BITS;
    let has_sys_menu = candidate.style & WS_SYSMENU_BITS != 0;
    let has_thick_frame = candidate.style & WS_THICKFRAME_BITS != 0;

    has_app_window || (has_caption && (has_sys_menu || has_thick_frame))
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

#[cfg(test)]
mod candidate_tests {
    use super::*;

    fn app_candidate() -> WindowCandidate {
        WindowCandidate {
            class_name: "Chrome_WidgetWin_1".to_string(),
            style: WS_CAPTION_BITS | WS_SYSMENU_BITS | WS_THICKFRAME_BITS,
            ex_style: 0,
            rect: Rect {
                left: 10,
                top: 10,
                right: 810,
                bottom: 610,
            },
            is_window: true,
            is_visible: true,
            is_iconic: false,
            has_parent: false,
        }
    }

    #[test]
    fn ordinary_application_window_is_pinnable() {
        assert!(is_pinnable_candidate(&app_candidate()));
    }

    #[test]
    fn shell_surfaces_are_not_pinnable() {
        for class_name in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ] {
            let mut candidate = app_candidate();
            candidate.class_name = class_name.to_string();
            assert!(
                !is_pinnable_candidate(&candidate),
                "{class_name} should be rejected"
            );
        }
    }

    #[test]
    fn own_helper_windows_are_not_pinnable() {
        for class_name in ["PinPickerClass", "PinOverlayClass", "PinAppMsgWindow"] {
            let mut candidate = app_candidate();
            candidate.class_name = class_name.to_string();
            assert!(
                !is_pinnable_candidate(&candidate),
                "{class_name} should be rejected"
            );
        }
    }

    #[test]
    fn hidden_minimized_child_and_tool_windows_are_not_pinnable() {
        let mut hidden = app_candidate();
        hidden.is_visible = false;
        assert!(!is_pinnable_candidate(&hidden));

        let mut minimized = app_candidate();
        minimized.is_iconic = true;
        assert!(!is_pinnable_candidate(&minimized));

        let mut child = app_candidate();
        child.has_parent = true;
        assert!(!is_pinnable_candidate(&child));

        let mut child_style = app_candidate();
        child_style.style |= WS_CHILD_BITS;
        assert!(!is_pinnable_candidate(&child_style));

        let mut tool = app_candidate();
        tool.ex_style |= WS_EX_TOOLWINDOW_BITS;
        assert!(!is_pinnable_candidate(&tool));
    }

    #[test]
    fn tiny_or_empty_targets_are_not_pinnable() {
        let mut tiny = app_candidate();
        tiny.rect.right = tiny.rect.left + 16;
        tiny.rect.bottom = tiny.rect.top + 16;
        assert!(!is_pinnable_candidate(&tiny));

        let mut empty = app_candidate();
        empty.rect.right = empty.rect.left;
        assert!(!is_pinnable_candidate(&empty));
    }

    #[test]
    fn appwindow_extended_style_can_be_pinnable_without_caption() {
        let mut candidate = app_candidate();
        candidate.style = 0;
        candidate.ex_style = WS_EX_APPWINDOW_BITS;
        assert!(is_pinnable_candidate(&candidate));
    }

    #[test]
    fn borderless_non_appwindow_is_not_pinnable() {
        let mut candidate = app_candidate();
        candidate.style = 0;
        candidate.ex_style = 0;
        assert!(!is_pinnable_candidate(&candidate));
    }
}
