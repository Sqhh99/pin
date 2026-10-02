//! Floating `pin_on` badge that hovers over a pinned target's title bar.
//!
//! Lifecycle (DeskPins-inspired, see `DeskPins/pinwnd.cpp:170-269`):
//! - [`Overlay`] is an RAII handle owned by the app's pinned set; dropping it
//!   destroys the window. The overlay never destroys itself and never
//!   un-topmosts its target.
//! - Clicking the badge, or the target disappearing, posts
//!   [`notify::UNPIN_REQUESTED`] to the app window, which unpins the target
//!   and drops the overlay.
//! - A ~60 fps timer keeps the badge on the target's caption, re-applies
//!   `HWND_TOPMOST` if the target lost it (DeskPins' `fixTopStyle`), and
//!   re-renders on DPI changes. Each tick only issues `SetWindowPos` /
//!   `ShowWindow` when something actually changed.

use std::cell::Cell;
use std::sync::{LazyLock, OnceLock};

use anyhow::{anyhow, Result};
use log::{debug, warn};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindow, IsChild, KillTimer, PostMessageW,
    SetCursor, SetTimer, SetWindowPos, ShowWindow, UpdateLayeredWindow, WindowFromPoint,
    GW_HWNDPREV, HCURSOR, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_NCDESTROY, WM_SETCURSOR,
    WM_TIMER, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::domain::geometry::{
    compute_overlay_rect, icon_px_for_dpi, should_show_overlay_for_hit, Rect,
};
use crate::domain::window::{WindowId, OVERLAY_WINDOW_CLASS};
use crate::resources::Asset;
use crate::ui::notify;
use crate::win::api::{dpi_for, frame_bounds, is_topmost, is_window, set_topmost};
use crate::win::cursor::{load_asset_cursor, CursorHotspot};
use crate::win::gdi::{Bitmap, MemDc, ScreenDc};
use crate::win::{encode_wide, register_class, set_user_data, take_user_data, user_data};

static CLASS_NAME: LazyLock<Vec<u16>> = LazyLock::new(|| encode_wide(OVERLAY_WINDOW_CLASS));

/// Badge hover cursor (`HCURSOR` as `isize`). Set once the class is registered.
static CURSOR: OnceLock<isize> = OnceLock::new();

const TIMER_ID: usize = 1;
/// ~60 fps — visually instant follow during a window drag without the
/// complexity of `SetWinEventHook`.
const TIMER_INTERVAL_MS: u32 = 16;

/// Per-window state, stored in `GWLP_USERDATA`. Only touched on the UI
/// thread; `Cell`s keep re-entrant window-proc calls sound.
struct OverlayCtx {
    target: HWND,
    notify: HWND,
    /// DPI the badge bitmap was rendered for.
    dpi: Cell<u32>,
    /// Last position we moved the badge to.
    last_rect: Cell<Option<Rect>>,
    visible: Cell<bool>,
    unpin_requested: Cell<bool>,
}

/// RAII handle to a badge window. Dropping it destroys the window.
pub struct Overlay {
    hwnd: HWND,
}

impl Overlay {
    /// Create a hidden badge for `target`; the first timer tick positions and
    /// shows it. Requests to unpin are posted to `notify`.
    pub fn create(target: WindowId, notify: HWND) -> Result<Self> {
        ensure_class()?;
        let target: HWND = target.into();
        let dpi = dpi_for(target);
        let icon_px = icon_px_for_dpi(dpi);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                PCWSTR(CLASS_NAME.as_ptr()),
                w!(""),
                WS_POPUP,
                0,
                0,
                icon_px,
                icon_px,
                None,
                None,
                GetModuleHandleW(None)?,
                None,
            )?
        };
        let overlay = Overlay { hwnd };
        set_user_data(
            hwnd,
            OverlayCtx {
                target,
                notify,
                dpi: Cell::new(dpi),
                last_rect: Cell::new(None),
                visible: Cell::new(false),
                unpin_requested: Cell::new(false),
            },
        );
        paint(hwnd, icon_px)?;
        if unsafe { SetTimer(hwnd, TIMER_ID, TIMER_INTERVAL_MS, None) } == 0 {
            return Err(anyhow!(
                "SetTimer failed: {}",
                windows::core::Error::from_win32()
            ));
        }
        Ok(overlay)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if is_window(self.hwnd) {
            let _ = unsafe { DestroyWindow(self.hwnd) };
        }
    }
}

fn ensure_class() -> Result<()> {
    if CURSOR.get().is_some() {
        return Ok(());
    }
    let cursor = load_asset_cursor(Asset::Cancel, CursorHotspot::Center)?;
    register_class(&CLASS_NAME, wnd_proc, cursor)?;
    let _ = CURSOR.set(cursor.0 as isize);
    Ok(())
}

fn set_overlay_cursor() {
    if let Some(&c) = CURSOR.get() {
        unsafe { SetCursor(HCURSOR(c as *mut _)) };
    }
}

/// Render the badge at `icon_px` into the layered window.
fn paint(hwnd: HWND, icon_px: i32) -> Result<()> {
    let icon = Asset::PinOn.render(icon_px as u32)?;
    let screen = ScreenDc::get()?;
    let mem = MemDc::compatible_with(&screen)?;
    // Layered windows with AC_SRC_ALPHA need premultiplied BGRA.
    let bitmap = Bitmap::from_bgra(&screen, icon.width, icon.height, &icon.to_bgra(true))?;
    let _selected = mem.select(&bitmap);

    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let size = SIZE {
        cx: icon.width as i32,
        cy: icon.height as i32,
    };
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            screen.handle(),
            None,
            Some(&size),
            mem.handle(),
            Some(&POINT::default()),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
    .map_err(|e| anyhow!("UpdateLayeredWindow: {e}"))
}

/// Ask the app to unpin our target (once), hiding the badge meanwhile.
unsafe fn request_unpin(overlay: HWND, ctx: &OverlayCtx) {
    if ctx.unpin_requested.replace(true) {
        return;
    }
    let _ = ShowWindow(overlay, SW_HIDE);
    ctx.visible.set(false);
    let _ = PostMessageW(
        ctx.notify,
        notify::UNPIN_REQUESTED,
        WPARAM(ctx.target.0 as usize),
        LPARAM(overlay.0 as isize),
    );
}

unsafe fn tick(overlay: HWND, ctx: &OverlayCtx) {
    if ctx.unpin_requested.get() {
        return;
    }
    let target = ctx.target;
    if !is_window(target) {
        debug!("overlay: target {:?} gone", target.0);
        request_unpin(overlay, ctx);
        return;
    }
    // Some apps clear WS_EX_TOPMOST themselves; keep the pin in force.
    if !is_topmost(target) {
        let _ = set_topmost(target, true);
    }

    let dpi = dpi_for(target);
    if dpi != ctx.dpi.get() {
        // Target moved to a monitor with a different scale factor.
        if let Err(e) = paint(overlay, icon_px_for_dpi(dpi)) {
            warn!("overlay: re-render at {dpi} dpi failed: {e}");
        }
        ctx.dpi.set(dpi);
        ctx.last_rect.set(None);
    }

    let Some(frame) = frame_bounds(target) else {
        return;
    };
    let rect = compute_overlay_rect(frame, dpi);
    if ctx.last_rect.get() != Some(rect) {
        let _ = SetWindowPos(
            overlay,
            None,
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        ctx.last_rect.set(Some(rect));
    }

    keep_directly_above(overlay, target);

    let show = is_unoccluded(overlay, target, rect);
    if show != ctx.visible.get() {
        let _ = ShowWindow(overlay, if show { SW_SHOWNOACTIVATE } else { SW_HIDE });
        ctx.visible.set(show);
    }
}

/// Keep `overlay` immediately above `target` in z-order: above the target,
/// but below whatever already covers it. `SetWindowPos(overlay, target, …)`
/// would place the overlay *behind* the target — wrong for a clickable badge.
unsafe fn keep_directly_above(overlay: HWND, target: HWND) {
    let above = GetWindow(target, GW_HWNDPREV).unwrap_or_default();
    if above == overlay {
        return;
    }
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE;
    if above.0.is_null() {
        // Target is the top window: slide it just below the overlay.
        let _ = SetWindowPos(target, overlay, 0, 0, 0, 0, flags);
    } else {
        let _ = SetWindowPos(overlay, above, 0, 0, 0, 0, flags);
    }
}

/// True when nothing unrelated covers the badge at its center.
unsafe fn is_unoccluded(overlay: HWND, target: HWND, rect: Rect) -> bool {
    let (x, y) = rect.center();
    let hit = WindowFromPoint(POINT { x, y });
    let hit = (!hit.0.is_null()).then_some(hit);
    should_show_overlay_for_hit(
        hit.map(WindowId::from),
        overlay.into(),
        target.into(),
        hit.is_some_and(|h| IsChild(target, h).as_bool()),
    )
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SETCURSOR => {
            set_overlay_cursor();
            LRESULT(1)
        }
        WM_MOUSEMOVE => {
            set_overlay_cursor();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            if let Some(ctx) = user_data::<OverlayCtx>(hwnd) {
                request_unpin(hwnd, ctx);
            }
            LRESULT(0)
        }
        WM_TIMER if wparam.0 == TIMER_ID => {
            if let Some(ctx) = user_data::<OverlayCtx>(hwnd) {
                tick(hwnd, ctx);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let _ = KillTimer(hwnd, TIMER_ID);
            drop(take_user_data::<OverlayCtx>(hwnd));
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
