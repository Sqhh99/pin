//! "Selection mode" via a transparent topmost 1x1 capture window — DeskPins style.
//!
//! Inspired by `DeskPins/pinlayerwnd.cpp:10-65` and `DeskPins/mainwnd.cpp:432-449`.
//!
//! - We register a window class whose `hCursor` is built **in memory** from the
//!   embedded `pin_off.png` (no file I/O, no `LoadCursorFromFileW` — we used to
//!   do that but `LoadCursorFromFileW` is finicky about PNG-encoded ICO/CUR).
//! - `SetCapture` routes every mouse message to our `picker_proc`. While the
//!   picker holds the capture, the system displays the class cursor everywhere.
//! - The first `WM_LBUTTONDOWN` resolves the top-level window under the cursor
//!   and posts it to the main thread.
//! - ESC / right click / middle click / focus loss / capture lost all cancel.

#![cfg(windows)]

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicIsize, Ordering};

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use once_cell::sync::OnceCell;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateIconIndirect, CreateWindowExW, DefWindowProcW, DestroyCursor, DestroyWindow,
    GetParent, GetWindow, GetWindowLongPtrW, LoadCursorW, PostMessageW, RegisterClassExW,
    SetCursor, SetForegroundWindow, SetSystemCursor, SetWindowLongPtrW,
    SystemParametersInfoW, GWLP_USERDATA, GW_OWNER, HCURSOR, ICONINFO, IDC_ARROW, OCR_APPSTARTING,
    OCR_CROSS, OCR_HAND, OCR_IBEAM, OCR_NO, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS,
    OCR_SIZENWSE, OCR_SIZEWE, OCR_UP, OCR_WAIT, SPIF_SENDCHANGE, SPI_SETCURSORS, SYSTEM_CURSOR_ID,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WM_APP, WM_CAPTURECHANGED, WM_KEYDOWN, WM_KILLFOCUS,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCDESTROY, WM_RBUTTONDOWN, WM_SETCURSOR,
    WM_SYSKEYDOWN, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::resources::{decode_png_resized, rgba_to_bgra};

/// Posted when the user picks a target window. `wparam` = top-level HWND as `isize`.
pub const PICKED_MSG: u32 = WM_APP + 4;
/// Posted when the selection is cancelled (ESC / right click / focus lost).
pub const PICK_CANCELED_MSG: u32 = WM_APP + 5;

/// Standard cursor side length we ship — 32×32 is the canonical Windows cursor.
pub const CURSOR_SIZE_PX: u32 = 32;

const CLASS_NAME: PCWSTR = w!("PinPickerClass");

static CLASS_REGISTERED: OnceCell<()> = OnceCell::new();

/// Picker cursor HCURSOR cached as an `isize`. Read inside `WM_SETCURSOR` /
/// `WM_MOUSEMOVE` to keep the cursor displayed even when the captured 1×1
/// window's hit-test code wouldn't make `DefWindowProc` install the class
/// cursor on its own.
static PICKER_CURSOR: AtomicIsize = AtomicIsize::new(0);

/// Owns the picker window's lifetime. Dropping it destroys the window
/// (which also releases capture) and restores the system cursors.
pub struct Picker {
    hwnd: HWND,
    cursors_overridden: bool,
}

impl Picker {
    /// Open the picker. `cursor_png_bytes` is the raw `pin_off.png`.
    pub fn open(main_hwnd: HWND, cursor_png_bytes: &[u8]) -> Result<Self> {
        ensure_class(cursor_png_bytes)?;

        // Globally swap the cursor — the only way that works reliably without
        // depending on WM_SETCURSOR being delivered to our hidden 1×1 popup.
        let cursors_overridden = {
            let c = PICKER_CURSOR.load(Ordering::SeqCst);
            if c != 0 {
                override_system_cursors(HCURSOR(c as *mut _))
            } else {
                false
            }
        };
        info!("picker cursors_overridden={}", cursors_overridden);

        unsafe {
            let hinstance = GetModuleHandleW(None)?;
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                hinstance,
                None,
            )?;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, main_hwnd.0 as isize);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetFocus(hwnd); // ensure WM_KEYDOWN (Esc) reaches picker_proc
            let prev = SetCapture(hwnd);
            debug!("picker opened hwnd={:?} prev_capture={:?}", hwnd.0, prev.0);
            info!("picker opened");
            Ok(Self { hwnd, cursors_overridden })
        }
    }
}

impl Drop for Picker {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.0.is_null() {
                let _ = DestroyWindow(self.hwnd);
            }
            if self.cursors_overridden {
                let _ = SystemParametersInfoW(
                    SPI_SETCURSORS,
                    0,
                    None,
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(SPIF_SENDCHANGE.0),
                );
            }
        }
        debug!("picker dropped");
    }
}

/// Override the common system cursor slots so the user sees `pin_off` while
/// selection mode is active. `SetSystemCursor` takes ownership of each cursor
/// passed to it, so we hand it a fresh `CopyIcon` of the master each slot.
fn override_system_cursors(master: HCURSOR) -> bool {
    const SLOTS: &[SYSTEM_CURSOR_ID] = &[
        OCR_NORMAL,
        OCR_IBEAM,
        OCR_HAND,
        OCR_CROSS,
        OCR_APPSTARTING,
        OCR_NO,
        OCR_SIZEALL,
        OCR_SIZENESW,
        OCR_SIZENS,
        OCR_SIZENWSE,
        OCR_SIZEWE,
        OCR_UP,
        OCR_WAIT,
    ];
    let mut any = false;
    for which in SLOTS {
        unsafe {
            let copy = match CopyIcon(
                windows::Win32::UI::WindowsAndMessaging::HICON(master.0),
            ) {
                Ok(c) => c,
                Err(e) => {
                    warn!("CopyIcon({:?}): {e}", which);
                    continue;
                }
            };
            match SetSystemCursor(HCURSOR(copy.0), *which) {
                Ok(()) => any = true,
                Err(e) => {
                    warn!("SetSystemCursor({:?}): {e}", which);
                    let _ = DestroyCursor(HCURSOR(copy.0));
                }
            }
        }
    }
    any
}

fn ensure_class(cursor_png_bytes: &[u8]) -> Result<()> {
    if CLASS_REGISTERED.get().is_some() {
        return Ok(());
    }
    let cursor = match build_cursor_from_png(cursor_png_bytes, CURSOR_SIZE_PX) {
        Ok(c) => {
            info!("picker cursor built {}x{}", CURSOR_SIZE_PX, CURSOR_SIZE_PX);
            c
        }
        Err(e) => {
            warn!("picker cursor fallback to IDC_ARROW: {e}");
            unsafe { LoadCursorW(None, IDC_ARROW)? }
        }
    };
    PICKER_CURSOR.store(cursor.0 as isize, Ordering::SeqCst);
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(picker_proc),
            hInstance: hinstance.into(),
            hCursor: cursor,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            return Err(anyhow!("RegisterClassExW(PinPickerClass) failed"));
        }
    }
    let _ = CLASS_REGISTERED.set(());
    Ok(())
}

unsafe extern "system" fn picker_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SETCURSOR => {
            let c = PICKER_CURSOR.load(Ordering::SeqCst);
            if c != 0 {
                SetCursor(HCURSOR(c as *mut _));
                return LRESULT(1); // we handled it
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            // Belt + suspenders: WM_SETCURSOR may not fire while the captured
            // window's hit-test stays HTNOWHERE; reasserting here keeps the
            // pin_off cursor visible as the user moves across the screen.
            let c = PICKER_CURSOR.load(Ordering::SeqCst);
            if c != 0 {
                SetCursor(HCURSOR(c as *mut _));
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let main_raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if main_raw != 0 {
                let mut pt = POINT::default();
                if windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt).is_ok() {
                    let target = top_parent_at(pt);
                    let main = HWND(main_raw as *mut _);
                    if !target.0.is_null() {
                        debug!("picker click at ({},{}) -> target {:?}", pt.x, pt.y, target.0);
                        let _ = PostMessageW(
                            main,
                            PICKED_MSG,
                            WPARAM(target.0 as usize),
                            LPARAM(0),
                        );
                    } else {
                        debug!("picker click at ({},{}) -> no window", pt.x, pt.y);
                        let _ = PostMessageW(main, PICK_CANCELED_MSG, WPARAM(0), LPARAM(0));
                    }
                }
            }
            let _ = ReleaseCapture();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_KEYDOWN | WM_SYSKEYDOWN | WM_KILLFOCUS
        | WM_CAPTURECHANGED => {
            let main_raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            if main_raw != 0 {
                let main = HWND(main_raw as *mut _);
                let _ = PostMessageW(main, PICK_CANCELED_MSG, WPARAM(0), LPARAM(0));
            }
            let _ = ReleaseCapture();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Top-level window at the screen point, following both parent and owner
/// chains — same logic as `DeskPins/util.cpp:29-64::getTopParent`.
fn top_parent_at(pt: POINT) -> HWND {
    use windows::Win32::UI::WindowsAndMessaging::WindowFromPoint;
    let mut h = unsafe { WindowFromPoint(pt) };
    if h.0.is_null() {
        return h;
    }
    loop {
        let parent = unsafe { GetParent(h) }.unwrap_or(HWND(ptr::null_mut()));
        if !parent.0.is_null() && parent.0 != h.0 {
            h = parent;
            continue;
        }
        let owner = unsafe { GetWindow(h, GW_OWNER) }.unwrap_or(HWND(ptr::null_mut()));
        if !owner.0.is_null() && owner.0 != h.0 {
            h = owner;
            continue;
        }
        return h;
    }
}

/// Build an `HCURSOR` from a raw PNG by decoding, resizing to `size_px` square,
/// then handing the resulting 32-bit ARGB DIB section to `CreateIconIndirect`.
///
/// Per MSDN: the color bitmap supplied to `CreateIconIndirect` is in
/// **non-premultiplied** ARGB. The mask is a 1-bpp monochrome bitmap; when the
/// color bitmap carries alpha the mask is largely ignored, so we ship an
/// all-zero mask sized to match.
fn build_cursor_from_png(png_bytes: &[u8], size_px: u32) -> Result<HCURSOR> {
    let img = decode_png_resized(png_bytes, size_px, size_px)?;
    unsafe { create_cursor_from_rgba(&img) }
}

unsafe fn create_cursor_from_rgba(rgba: &crate::resources::Rgba) -> Result<HCURSOR> {
    let w = rgba.width as i32;
    let h = rgba.height as i32;

    let screen_dc = GetDC(None);

    let mut bmi = BITMAPINFO::default();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h; // top-down
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB.0;

    let mut bits: *mut c_void = ptr::null_mut();
    let h_color: HBITMAP = match CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
    {
        Ok(h) => h,
        Err(e) => {
            ReleaseDC(None, screen_dc);
            return Err(anyhow!("CreateDIBSection: {e}"));
        }
    };
    if bits.is_null() {
        let _ = DeleteObject(h_color);
        ReleaseDC(None, screen_dc);
        return Err(anyhow!("CreateDIBSection: null bits"));
    }

    // Copy BGRA (non-premultiplied) into the DIB section.
    let bgra = rgba_to_bgra(&rgba.pixels);
    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, bgra.len());
    dst.copy_from_slice(&bgra);

    // Monochrome mask: all zero. Stride = ((w + 15) / 16) * 2 bytes per row.
    let mask_stride = ((w as usize + 15) / 16) * 2;
    let mask_buf = vec![0u8; mask_stride * h as usize];
    let h_mask = CreateBitmap(w, h, 1, 1, Some(mask_buf.as_ptr() as *const _));
    if h_mask.is_invalid() {
        let _ = DeleteObject(h_color);
        ReleaseDC(None, screen_dc);
        return Err(anyhow!("CreateBitmap(mask) failed"));
    }

    let info = ICONINFO {
        fIcon: BOOL(0), // 0 = cursor (TRUE would mean icon)
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: h_mask,
        hbmColor: h_color,
    };
    let hicon_res = CreateIconIndirect(&info);

    let _ = DeleteObject(h_color);
    let _ = DeleteObject(h_mask);
    ReleaseDC(None, screen_dc);

    let hicon = hicon_res.map_err(|e| anyhow!("CreateIconIndirect: {e}"))?;
    Ok(HCURSOR(hicon.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::PIN_OFF_PNG;

    #[test]
    fn picker_msg_ids_are_distinct() {
        assert_ne!(PICKED_MSG, PICK_CANCELED_MSG);
        assert!(PICKED_MSG >= WM_APP);
        assert!(PICK_CANCELED_MSG >= WM_APP);
    }

    #[test]
    fn pin_off_decodes_to_cursor_size() {
        let img = decode_png_resized(PIN_OFF_PNG, CURSOR_SIZE_PX, CURSOR_SIZE_PX)
            .expect("decode pin_off resized");
        assert_eq!(img.width, CURSOR_SIZE_PX);
        assert_eq!(img.height, CURSOR_SIZE_PX);
        assert_eq!(img.pixels.len() as u32, CURSOR_SIZE_PX * CURSOR_SIZE_PX * 4);
    }

    #[test]
    fn pin_off_has_visible_pixels() {
        // Smoke check: after resizing, the cursor isn't fully transparent /
        // fully empty. If this fails, the source PNG is broken.
        let img = decode_png_resized(PIN_OFF_PNG, CURSOR_SIZE_PX, CURSOR_SIZE_PX).unwrap();
        let opaque_count = img
            .pixels
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .count();
        assert!(opaque_count > 0, "pin_off has zero opaque pixels after resize");
    }
}
