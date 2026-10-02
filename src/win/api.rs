//! Window queries and the production [`WindowApi`].

use std::ffi::c_void;

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetParent, GetWindow, GetWindowLongW, GetWindowRect, IsIconic, IsWindow,
    IsWindowVisible, SetWindowPos, WindowFromPoint, GWL_EXSTYLE, GWL_STYLE, GW_OWNER,
    HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_TOPMOST,
};

use crate::domain::geometry::{normalize_dpi, Rect};
use crate::domain::window::{is_pinnable_candidate, WindowApi, WindowCandidate, WindowId};

pub struct RealWindowApi;

impl WindowApi for RealWindowApi {
    fn set_topmost(&self, w: WindowId, on: bool) -> Result<()> {
        set_topmost(w.into(), on)
    }

    fn is_window(&self, w: WindowId) -> bool {
        is_window(w.into())
    }
}

pub fn set_topmost(hwnd: HWND, on: bool) -> Result<()> {
    let insert_after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
    unsafe {
        SetWindowPos(
            hwnd,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
    .map_err(|e| anyhow!("SetWindowPos failed: {e}"))
}

pub fn is_window(hwnd: HWND) -> bool {
    !hwnd.0.is_null() && unsafe { IsWindow(hwnd) }.as_bool()
}

pub fn is_topmost(hwnd: HWND) -> bool {
    (unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32) & WS_EX_TOPMOST.0 != 0
}

/// Effective DPI of the monitor `hwnd` is on (96 if unknown).
pub fn dpi_for(hwnd: HWND) -> u32 {
    normalize_dpi(unsafe { GetDpiForWindow(hwnd) })
}

/// Raw `GetWindowRect`, including the invisible resize borders.
pub fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r) }.ok()?;
    Some(r.into())
}

/// Visible frame of `hwnd`: DWM's extended frame bounds, which exclude the
/// invisible resize borders `GetWindowRect` reports on Windows 10+ (and the
/// off-screen overhang of maximized windows). Falls back to `GetWindowRect`.
pub fn frame_bounds(hwnd: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    let dwm = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    match dwm {
        Ok(()) if r.right > r.left && r.bottom > r.top => Some(r.into()),
        _ => window_rect(hwnd),
    }
}

/// Top-level window at the screen point, following both parent and owner
/// chains — same logic as `DeskPins/util.cpp:29-64::getTopParent`.
pub fn top_parent_at(pt: POINT) -> Option<HWND> {
    let mut h = unsafe { WindowFromPoint(pt) };
    if h.0.is_null() {
        return None;
    }
    loop {
        let parent = unsafe { GetParent(h) }.unwrap_or_default();
        if !parent.0.is_null() && parent != h {
            h = parent;
            continue;
        }
        let owner = unsafe { GetWindow(h, GW_OWNER) }.unwrap_or_default();
        if !owner.0.is_null() && owner != h {
            h = owner;
            continue;
        }
        return Some(h);
    }
}

/// Snapshot the properties [`is_pinnable_candidate`] needs.
pub fn candidate_for(hwnd: HWND) -> Option<WindowCandidate> {
    let rect = window_rect(hwnd)?;

    let mut class_buf = [0u16; 256];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class_buf) };
    if class_len <= 0 {
        return None;
    }
    let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
    let parent = unsafe { GetParent(hwnd) }.unwrap_or_default();

    Some(WindowCandidate {
        class_name,
        style: unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32,
        ex_style: unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32,
        rect,
        is_window: is_window(hwnd),
        is_visible: unsafe { IsWindowVisible(hwnd) }.as_bool(),
        is_iconic: unsafe { IsIconic(hwnd) }.as_bool(),
        has_parent: !parent.0.is_null(),
    })
}

pub fn is_pinnable(hwnd: HWND) -> bool {
    candidate_for(hwnd).is_some_and(|c| is_pinnable_candidate(&c))
}
