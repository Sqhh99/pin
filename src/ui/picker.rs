//! "Selection mode" via a transparent topmost 1×1 capture window — DeskPins style.
//!
//! Inspired by `DeskPins/pinlayerwnd.cpp:10-65` and `DeskPins/mainwnd.cpp:432-449`.
//!
//! - The window class cursor is built **in memory** from the embedded
//!   `pin_off.png`, and the common system cursors are swapped for it while
//!   the picker is open (the only reliably visible mechanism for a hidden
//!   capture window).
//! - `SetCapture` routes every mouse message to `picker_proc`.
//! - The first left click on a pinnable window, or ESC / any key / right or
//!   middle click / focus or capture loss, posts exactly **one** result to
//!   the app window. The picker never destroys itself: the app drops the
//!   [`Picker`], which destroys the window and restores the cursors.

use std::cell::Cell;
use std::sync::{LazyLock, OnceLock};

use anyhow::Result;
use log::{debug, info};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, PostMessageW, SetCursor,
    SetForegroundWindow, HCURSOR, WM_CAPTURECHANGED, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCDESTROY, WM_RBUTTONDOWN, WM_SETCURSOR, WM_SYSKEYDOWN,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::domain::window::PICKER_WINDOW_CLASS;
use crate::resources::Asset;
use crate::ui::notify;
use crate::win::api::{is_pinnable, top_parent_at};
use crate::win::cursor::{
    load_asset_cursor, override_system_cursors, restore_system_cursors, CursorHotspot,
};
use crate::win::{encode_wide, register_class, set_user_data, take_user_data, user_data};

static CLASS_NAME: LazyLock<Vec<u16>> = LazyLock::new(|| encode_wide(PICKER_WINDOW_CLASS));

/// Picker cursor (`HCURSOR` as `isize`). Set once the class is registered.
static CURSOR: OnceLock<isize> = OnceLock::new();

/// Per-window state, stored in `GWLP_USERDATA`.
struct PickerCtx {
    /// App window that receives the result.
    notify: HWND,
    /// Set once a result has been posted (or the picker is being dropped).
    done: Cell<bool>,
}

/// Owns the picker window. Dropping it destroys the window (releasing
/// capture) and restores the system cursors.
pub struct Picker {
    hwnd: HWND,
    cursors_overridden: bool,
}

impl Picker {
    /// Enter selection mode; the result is posted to `notify`.
    pub fn open(notify: HWND) -> Result<Self> {
        let cursor = ensure_class()?;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
                PCWSTR(CLASS_NAME.as_ptr()),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                GetModuleHandleW(None)?,
                None,
            )?
        };
        set_user_data(
            hwnd,
            PickerCtx {
                notify,
                done: Cell::new(false),
            },
        );
        let mut picker = Picker {
            hwnd,
            cursors_overridden: false,
        };
        picker.cursors_overridden = override_system_cursors(cursor);
        unsafe {
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd); // so WM_KEYDOWN (Esc) reaches picker_proc
            SetCapture(hwnd);
        }
        info!(
            "picker opened (cursors_overridden={})",
            picker.cursors_overridden
        );
        Ok(picker)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        unsafe {
            // Destroying the window triggers WM_CAPTURECHANGED/WM_KILLFOCUS;
            // don't let those post a stale "canceled" result.
            if let Some(ctx) = user_data::<PickerCtx>(self.hwnd) {
                ctx.done.set(true);
            }
            let _ = DestroyWindow(self.hwnd);
        }
        if self.cursors_overridden {
            restore_system_cursors();
        }
        debug!("picker dropped");
    }
}

fn ensure_class() -> Result<HCURSOR> {
    if let Some(&c) = CURSOR.get() {
        return Ok(HCURSOR(c as *mut _));
    }
    let cursor = load_asset_cursor(Asset::PinOff, CursorHotspot::TopLeft)?;
    register_class(&CLASS_NAME, picker_proc, cursor)?;
    let _ = CURSOR.set(cursor.0 as isize);
    Ok(cursor)
}

fn set_picker_cursor() {
    if let Some(&c) = CURSOR.get() {
        unsafe { SetCursor(HCURSOR(c as *mut _)) };
    }
}

/// Post the (single) result to the app window and release capture.
unsafe fn finish(hwnd: HWND, target: Option<HWND>) {
    let Some(ctx) = user_data::<PickerCtx>(hwnd) else {
        return;
    };
    if ctx.done.replace(true) {
        return;
    }
    let picker = LPARAM(hwnd.0 as isize);
    let _ = match target {
        Some(t) => PostMessageW(ctx.notify, notify::PICKED, WPARAM(t.0 as usize), picker),
        None => PostMessageW(ctx.notify, notify::PICK_CANCELED, WPARAM(0), picker),
    };
    // Re-enters picker_proc with WM_CAPTURECHANGED, which `done` ignores.
    let _ = ReleaseCapture();
}

unsafe extern "system" fn picker_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SETCURSOR => {
            set_picker_cursor();
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            // WM_SETCURSOR may not fire while the captured window's hit-test
            // stays HTNOWHERE; reasserting here keeps the cursor visible.
            set_picker_cursor();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let mut pt = POINT::default();
            if GetCursorPos(&mut pt).is_ok() {
                match top_parent_at(pt) {
                    Some(target) if is_pinnable(target) => {
                        debug!("picker click at ({},{}) -> {:?}", pt.x, pt.y, target.0);
                        finish(hwnd, Some(target));
                    }
                    other => {
                        debug!("picker click at ({},{}) ignored ({other:?})", pt.x, pt.y);
                    }
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_KEYDOWN | WM_SYSKEYDOWN | WM_KILLFOCUS
        | WM_CAPTURECHANGED => {
            finish(hwnd, None);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            drop(take_user_data::<PickerCtx>(hwnd));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
