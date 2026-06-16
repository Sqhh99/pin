//! Windows cursor helpers built from embedded PNG assets.

#![cfg(windows)]

use std::ffi::c_void;
use std::ptr;

use anyhow::{anyhow, Result};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HCURSOR, ICONINFO};

use crate::resources::{decode_png_resized, rgba_to_bgra, Rgba};

/// Standard cursor side length we ship. 32x32 is the canonical Windows cursor.
pub const CURSOR_SIZE_PX: u32 = 32;

pub enum CursorHotspot {
    TopLeft,
    Center,
}

impl CursorHotspot {
    fn coords(&self, size_px: u32) -> (u32, u32) {
        match self {
            Self::TopLeft => (0, 0),
            Self::Center => (size_px / 2, size_px / 2),
        }
    }
}

/// Build an `HCURSOR` from a raw PNG by decoding, resizing, and passing a
/// 32-bit ARGB DIB section to `CreateIconIndirect`.
pub fn build_cursor_from_png(
    png_bytes: &[u8],
    size_px: u32,
    hotspot: CursorHotspot,
) -> Result<HCURSOR> {
    let img = decode_png_resized(png_bytes, size_px, size_px)?;
    let (hotspot_x, hotspot_y) = hotspot.coords(size_px);
    unsafe { create_cursor_from_rgba(&img, hotspot_x, hotspot_y) }
}

unsafe fn create_cursor_from_rgba(rgba: &Rgba, hotspot_x: u32, hotspot_y: u32) -> Result<HCURSOR> {
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
    let h_color: HBITMAP =
        match CreateDIBSection(screen_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
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

    // Copy BGRA (non-premultiplied) into the DIB section. Per MSDN, the color
    // bitmap supplied to `CreateIconIndirect` is non-premultiplied ARGB.
    let bgra = rgba_to_bgra(&rgba.pixels);
    let dst = std::slice::from_raw_parts_mut(bits as *mut u8, bgra.len());
    dst.copy_from_slice(&bgra);

    // Monochrome mask: all zero. When the color bitmap carries alpha, Windows
    // largely ignores the mask; it still must be sized correctly.
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
        xHotspot: hotspot_x,
        yHotspot: hotspot_y,
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
