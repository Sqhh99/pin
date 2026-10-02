//! RAII wrappers for the handful of GDI objects we create, plus the shared
//! "32-bit top-down BGRA DIB section" helper used by cursors and overlays.

use std::ffi::c_void;

use anyhow::{anyhow, Result};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
    SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};

/// The screen device context (`GetDC(NULL)`).
pub struct ScreenDc(HDC);

impl ScreenDc {
    pub fn get() -> Result<Self> {
        let dc = unsafe { GetDC(None) };
        if dc.is_invalid() {
            return Err(anyhow!("GetDC(NULL) failed"));
        }
        Ok(Self(dc))
    }

    pub fn handle(&self) -> HDC {
        self.0
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(None, self.0) };
    }
}

/// A memory DC compatible with the screen.
pub struct MemDc(HDC);

impl MemDc {
    pub fn compatible_with(dc: &ScreenDc) -> Result<Self> {
        let mem = unsafe { CreateCompatibleDC(dc.handle()) };
        if mem.is_invalid() {
            return Err(anyhow!("CreateCompatibleDC failed"));
        }
        Ok(Self(mem))
    }

    pub fn handle(&self) -> HDC {
        self.0
    }

    /// Select `bitmap` into this DC until the guard is dropped.
    pub fn select<'a>(&'a self, bitmap: &'a Bitmap) -> Selection<'a> {
        let previous = unsafe { SelectObject(self.0, bitmap.handle()) };
        Selection { dc: self, previous }
    }
}

impl Drop for MemDc {
    fn drop(&mut self) {
        let _ = unsafe { DeleteDC(self.0) };
    }
}

/// Restores the previously selected object when dropped.
pub struct Selection<'a> {
    dc: &'a MemDc,
    previous: HGDIOBJ,
}

impl Drop for Selection<'_> {
    fn drop(&mut self) {
        unsafe { SelectObject(self.dc.0, self.previous) };
    }
}

/// An owned `HBITMAP`.
pub struct Bitmap(HBITMAP);

impl Bitmap {
    pub fn handle(&self) -> HBITMAP {
        self.0
    }

    /// Monochrome bitmap with every bit cleared.
    pub fn zeroed_mask(width: i32, height: i32) -> Result<Self> {
        // Monochrome rows are WORD-aligned.
        let stride = (width as usize).div_ceil(16) * 2;
        let bits = vec![0u8; stride * height as usize];
        let bmp =
            unsafe { CreateBitmap(width, height, 1, 1, Some(bits.as_ptr() as *const c_void)) };
        if bmp.is_invalid() {
            return Err(anyhow!("CreateBitmap(mask) failed"));
        }
        Ok(Self(bmp))
    }

    /// 32-bit top-down DIB section initialised with `bgra` pixels.
    pub fn from_bgra(dc: &ScreenDc, width: u32, height: u32, bgra: &[u8]) -> Result<Self> {
        let expected = width as usize * height as usize * 4;
        if bgra.len() != expected {
            return Err(anyhow!(
                "pixel buffer is {} bytes, expected {expected}",
                bgra.len()
            ));
        }
        let mut bmi = BITMAPINFO::default();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width as i32;
        bmi.bmiHeader.biHeight = -(height as i32); // negative = top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut c_void = std::ptr::null_mut();
        let hbmp =
            unsafe { CreateDIBSection(dc.handle(), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) }
                .map_err(|e| anyhow!("CreateDIBSection: {e}"))?;
        let bitmap = Self(hbmp);
        if bits.is_null() {
            return Err(anyhow!("CreateDIBSection: null bits"));
        }
        unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, expected) }.copy_from_slice(bgra);
        Ok(bitmap)
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(self.0) };
    }
}
