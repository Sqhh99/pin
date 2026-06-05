//! Floating `pin_on` icon window that hovers over a pinned target's title bar.
//!
//! - Geometry math ([`compute_overlay_rect`]) is platform-free and unit-tested.
//! - Window creation, layered painting, click-to-unpin, and a ~75 ms tracking
//!   timer live behind `#[cfg(windows)]`.
//!
//! Lifecycle (DeskPins-inspired, see `DeskPins/pinwnd.cpp:170-269`):
//! - The overlay owns itself: clicking it (`WM_LBUTTONDOWN`) triggers
//!   `DestroyWindow(self)`; `WM_DESTROY` clears `HWND_TOPMOST` on the target
//!   and posts [`UNPIN_NOTIFY`] to the main thread so it can drop the entry.
//! - A 75 ms `SetTimer` repaints/repositions the overlay and re-applies
//!   `HWND_TOPMOST` on the target — equivalent to DeskPins' `fixTopStyle`.

use crate::win::Rect;

/// Default overlay icon side length in physical pixels at 96 DPI.
/// Picked to be roughly the height of the caption button glyph strip so the
/// pin reads as a sibling of the min/max/close icons rather than a tiny dot.
pub const BASE_ICON_PX: i32 = 24;

/// Compute the screen rectangle for the floating icon.
///
/// Position follows DeskPins' `placeOnCaption` (`DeskPins/pinwnd.cpp:323-361`):
/// `x = right - (3 * caption_btn_w + icon_w + 4)`, `y = top + 3`, all DPI-scaled.
pub fn compute_overlay_rect(window: Rect, dpi: u32) -> Rect {
    let scale = (dpi as f32 / 96.0).max(1.0);
    let icon = (BASE_ICON_PX as f32 * scale).round() as i32;
    let caption_button_w = (46.0 * scale).round() as i32;
    let strip = caption_button_w * 3;
    let pad = (4.0 * scale).round() as i32;

    let right = window.right - strip - pad;
    let left = right - icon;
    let top = window.top + (3.0 * scale).round() as i32;
    let bottom = top + icon;
    Rect { left, top, right, bottom }
}

#[cfg(windows)]
pub use win_impl::{create_overlay, destroy_overlay, UNPIN_NOTIFY};

#[cfg(windows)]
mod win_impl {
    use super::*;
    use crate::resources::{decode_png_resized, premultiply_bgra_inplace, rgba_to_bgra, Rgba};
    use crate::win::WindowId;
    use anyhow::{anyhow, Result};
    use log::debug;
    use once_cell::sync::OnceCell;
    use std::ffi::c_void;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetWindowLongPtrW, GetWindowLongW,
        GetWindowRect, IsWindow, KillTimer, PostMessageW, RegisterClassExW, SetTimer,
        SetWindowLongPtrW, SetWindowPos, ShowWindow, UpdateLayeredWindow, CW_USEDEFAULT,
        GWLP_USERDATA, GWL_EXSTYLE, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SW_SHOWNOACTIVATE, ULW_ALPHA, WM_APP, WM_LBUTTONDOWN, WM_NCDESTROY, WM_TIMER, WNDCLASSEXW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    /// Posted to the main thread after the overlay self-destroys (user clicked
    /// it, or its target window vanished). `wparam` = target HWND as `isize`.
    pub const UNPIN_NOTIFY: u32 = WM_APP + 6;

    const CLASS_NAME: PCWSTR = w!("PinOverlayClass");
    const TIMER_ID: usize = 1;
    /// ~60 fps — visually instant follow during a window drag without the
    /// complexity of `SetWinEventHook`.
    const TIMER_INTERVAL_MS: u32 = 16;

    static CLASS_REGISTERED: OnceCell<()> = OnceCell::new();

    struct OverlayCtx {
        target: isize,
        main_hwnd: isize,
        icon_px: u32,
    }

    fn ensure_class() -> Result<()> {
        if CLASS_REGISTERED.get().is_some() {
            return Ok(());
        }
        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: CLASS_NAME,
                ..Default::default()
            };
            if RegisterClassExW(&wc) == 0 {
                return Err(anyhow!("RegisterClassExW(PinOverlayClass) failed"));
            }
        }
        let _ = CLASS_REGISTERED.set(());
        Ok(())
    }

    /// Create a layered overlay anchored to `target`. The PNG is decoded and
    /// resized to the per-monitor DPI-scaled icon size before the window is
    /// even created — this avoids the previous bug where a 256×256 source
    /// was clipped to a 16×16 window.
    pub fn create_overlay(
        target: WindowId,
        main_thread_hwnd: HWND,
        icon_png_bytes: &[u8],
    ) -> Result<HWND> {
        ensure_class()?;
        unsafe {
            let target_hwnd: HWND = target.into();
            let dpi = {
                let d = GetDpiForWindow(target_hwnd);
                if d == 0 { 96 } else { d }
            };
            let scale = (dpi as f32 / 96.0).max(1.0);
            let icon_px = (BASE_ICON_PX as f32 * scale).round() as u32;
            let icon = decode_png_resized(icon_png_bytes, icon_px, icon_px)?;

            let hinstance = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                icon_px as i32,
                icon_px as i32,
                None,
                None,
                hinstance,
                None,
            )?;

            let ctx = Box::new(OverlayCtx {
                target: target.0,
                main_hwnd: main_thread_hwnd.0 as isize,
                icon_px,
            });
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(ctx) as isize);

            paint_layered(hwnd, &icon)?;
            reposition(hwnd, target_hwnd, icon_px as i32);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetTimer(hwnd, TIMER_ID, TIMER_INTERVAL_MS, None);
            Ok(hwnd)
        }
    }

    pub fn destroy_overlay(hwnd: HWND) {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    /// Re-place the overlay over its target's title bar. If the target has
    /// somehow lost `WS_EX_TOPMOST` (some apps clear it themselves), restore
    /// it FIRST so the subsequent overlay reposition lands on top in the
    /// z-order — avoiding the target/overlay flicker we used to get when
    /// the topmost reassertion happened unconditionally and last.
    unsafe fn reposition(overlay: HWND, target: HWND, icon_px: i32) {
        let mut r = RECT::default();
        if GetWindowRect(target, &mut r).is_err() {
            return;
        }
        let dpi = {
            let d = GetDpiForWindow(target);
            if d == 0 { 96 } else { d }
        };
        let rect = compute_overlay_rect(r.into(), dpi);

        let ex_style = GetWindowLongW(target, GWL_EXSTYLE) as u32;
        if (ex_style & WS_EX_TOPMOST.0) == 0 {
            let _ = SetWindowPos(
                target,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        let _ = SetWindowPos(
            overlay,
            HWND_TOPMOST,
            rect.left,
            rect.top,
            icon_px,
            icon_px,
            SWP_NOACTIVATE,
        );
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_LBUTTONDOWN => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_ID => {
                let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const OverlayCtx;
                if !ctx_ptr.is_null() {
                    let ctx = &*ctx_ptr;
                    let target = HWND(ctx.target as *mut _);
                    if !IsWindow(target).as_bool() {
                        debug!("overlay: target {:?} gone, self-destruct", target.0);
                        let _ = DestroyWindow(hwnd);
                    } else {
                        reposition(hwnd, target, ctx.icon_px as i32);
                    }
                }
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ctx_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayCtx;
                if !ctx_ptr.is_null() {
                    let ctx_box = Box::from_raw(ctx_ptr);
                    let target = HWND(ctx_box.target as *mut _);
                    let main = HWND(ctx_box.main_hwnd as *mut _);
                    let _ = KillTimer(hwnd, TIMER_ID);
                    if IsWindow(target).as_bool() {
                        let _ = SetWindowPos(
                            target,
                            windows::Win32::UI::WindowsAndMessaging::HWND_NOTOPMOST,
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                    }
                    if !main.0.is_null() {
                        let _ = PostMessageW(
                            main,
                            UNPIN_NOTIFY,
                            WPARAM(target.0 as usize),
                            LPARAM(0),
                        );
                    }
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(ctx_box);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    fn paint_layered(hwnd: HWND, icon: &Rgba) -> Result<()> {
        unsafe {
            let screen_dc = GetDC(None);
            let mem_dc = CreateCompatibleDC(screen_dc);

            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = icon.width as i32;
            bmi.bmiHeader.biHeight = -(icon.height as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;

            let mut bits: *mut c_void = std::ptr::null_mut();
            let hbmp: HBITMAP =
                CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
            if bits.is_null() {
                let _ = DeleteDC(mem_dc);
                ReleaseDC(None, screen_dc);
                return Err(anyhow!("CreateDIBSection: null bits"));
            }

            // RGBA → BGRA, then premultiply for AC_SRC_ALPHA.
            let mut bgra = rgba_to_bgra(&icon.pixels);
            premultiply_bgra_inplace(&mut bgra);
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, bgra.len());
            dst.copy_from_slice(&bgra);

            let old = SelectObject(mem_dc, hbmp);

            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let size = windows::Win32::Foundation::SIZE {
                cx: icon.width as i32,
                cy: icon.height as i32,
            };
            let src_pt = POINT { x: 0, y: 0 };
            UpdateLayeredWindow(
                hwnd,
                screen_dc,
                None,
                Some(&size),
                mem_dc,
                Some(&src_pt),
                windows::Win32::Foundation::COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )?;

            SelectObject(mem_dc, old);
            let _ = DeleteObject(hbmp);
            let _ = DeleteDC(mem_dc);
            ReleaseDC(None, screen_dc);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_rect_inside_window_at_96_dpi() {
        let win = Rect { left: 100, top: 200, right: 900, bottom: 800 };
        let r = compute_overlay_rect(win, 96);
        assert!(r.left > win.left && r.right < win.right);
        assert_eq!(r.width(), BASE_ICON_PX);
        assert_eq!(r.height(), BASE_ICON_PX);
    }

    #[test]
    fn overlay_rect_scales_with_dpi() {
        let win = Rect { left: 0, top: 0, right: 1000, bottom: 600 };
        let r96 = compute_overlay_rect(win, 96);
        let r192 = compute_overlay_rect(win, 192);
        assert!(r192.width() > r96.width());
        assert!(r192.height() > r96.height());
    }

    #[test]
    fn overlay_rect_anchored_near_top_right() {
        let win = Rect { left: 0, top: 0, right: 1000, bottom: 600 };
        let r = compute_overlay_rect(win, 96);
        // Top-right anchored: should be in the upper portion and right side.
        assert!(r.top < 30, "top={} should be in title bar", r.top);
        assert!(r.right > win.right * 8 / 10, "should sit near right edge");
    }
}
