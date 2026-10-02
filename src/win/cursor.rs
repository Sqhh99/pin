//! Cursors built from embedded PNG assets, and the global system-cursor
//! override used by selection mode.

use anyhow::{anyhow, Result};
use log::warn;
use windows::Win32::Foundation::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateIconIndirect, DestroyCursor, GetSystemMetrics, LoadCursorW, SetSystemCursor,
    SystemParametersInfoW, HCURSOR, HICON, ICONINFO, IDC_ARROW, OCR_APPSTARTING, OCR_CROSS,
    OCR_HAND, OCR_IBEAM, OCR_NO, OCR_NORMAL, OCR_SIZEALL, OCR_SIZENESW, OCR_SIZENS, OCR_SIZENWSE,
    OCR_SIZEWE, OCR_UP, OCR_WAIT, SM_CXCURSOR, SPIF_SENDCHANGE, SPI_SETCURSORS, SYSTEM_CURSOR_ID,
};

use crate::resources::Asset;
use crate::win::gdi::{Bitmap, ScreenDc};

/// Fallback cursor side length; 32×32 is the canonical size at 100% scaling.
const DEFAULT_CURSOR_PX: u32 = 32;

#[derive(Clone, Copy)]
pub enum CursorHotspot {
    TopLeft,
    Center,
}

impl CursorHotspot {
    fn coords(self, size_px: u32) -> (u32, u32) {
        match self {
            Self::TopLeft => (0, 0),
            Self::Center => (size_px / 2, size_px / 2),
        }
    }
}

/// Nominal system cursor size, which scales with the primary monitor's DPI
/// (32px at 100%, 48px at 150%, ...).
pub fn cursor_size_px() -> u32 {
    match unsafe { GetSystemMetrics(SM_CXCURSOR) } {
        n if n > 0 => n as u32,
        _ => DEFAULT_CURSOR_PX,
    }
}

/// Build a cursor from `asset` at the system cursor size, falling back to
/// the stock arrow if that fails. The handle lives for the whole process.
pub fn load_asset_cursor(asset: Asset, hotspot: CursorHotspot) -> Result<HCURSOR> {
    match cursor_from_asset(asset, cursor_size_px(), hotspot) {
        Ok(c) => Ok(c),
        Err(e) => {
            warn!("{asset:?} cursor fallback to IDC_ARROW: {e}");
            Ok(unsafe { LoadCursorW(None, IDC_ARROW)? })
        }
    }
}

/// Decode, resize, and hand a 32-bit straight-alpha DIB section to
/// `CreateIconIndirect`.
fn cursor_from_asset(asset: Asset, size_px: u32, hotspot: CursorHotspot) -> Result<HCURSOR> {
    let img = asset.render(size_px)?;
    let (x_hotspot, y_hotspot) = hotspot.coords(size_px);

    let screen = ScreenDc::get()?;
    // Per MSDN, the color bitmap for CreateIconIndirect is non-premultiplied.
    let color = Bitmap::from_bgra(&screen, img.width, img.height, &img.to_bgra(false))?;
    // When the color bitmap carries alpha Windows ignores the mask, but it
    // still has to exist and be sized correctly.
    let mask = Bitmap::zeroed_mask(img.width as i32, img.height as i32)?;

    let info = ICONINFO {
        fIcon: BOOL(0), // FALSE = cursor
        xHotspot: x_hotspot,
        yHotspot: y_hotspot,
        hbmMask: mask.handle(),
        hbmColor: color.handle(),
    };
    // CreateIconIndirect copies the bitmaps; ours are freed on drop.
    let icon =
        unsafe { CreateIconIndirect(&info) }.map_err(|e| anyhow!("CreateIconIndirect: {e}"))?;
    Ok(HCURSOR(icon.0))
}

const OVERRIDDEN_SLOTS: &[SYSTEM_CURSOR_ID] = &[
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

/// Replace the common system cursor slots with `master`. `SetSystemCursor`
/// takes ownership of each cursor passed to it, so every slot gets its own
/// `CopyIcon`. Returns true if at least one slot was replaced; undo with
/// [`restore_system_cursors`].
pub fn override_system_cursors(master: HCURSOR) -> bool {
    let mut any = false;
    for &slot in OVERRIDDEN_SLOTS {
        unsafe {
            let copy = match CopyIcon(HICON(master.0)) {
                Ok(c) => HCURSOR(c.0),
                Err(e) => {
                    warn!("CopyIcon({slot:?}): {e}");
                    continue;
                }
            };
            match SetSystemCursor(copy, slot) {
                Ok(()) => any = true,
                Err(e) => {
                    warn!("SetSystemCursor({slot:?}): {e}");
                    let _ = DestroyCursor(copy);
                }
            }
        }
    }
    any
}

/// Reload the user's cursor scheme from the registry, undoing any
/// [`override_system_cursors`]. Safe to call at any time.
pub fn restore_system_cursors() {
    if let Err(e) = unsafe { SystemParametersInfoW(SPI_SETCURSORS, 0, None, SPIF_SENDCHANGE) } {
        warn!("restore system cursors: {e}");
    }
}
