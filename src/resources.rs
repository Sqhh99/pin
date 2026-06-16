//! Embedded icon/image resources, decoded lazily.

use anyhow::{Context, Result};

pub const PIN_ON_PNG: &[u8] = include_bytes!("../resource/icon/pin_on.png");
pub const PIN_OFF_PNG: &[u8] = include_bytes!("../resource/icon/pin_off.png");
pub const CANCEL_PNG: &[u8] = include_bytes!("../resource/icon/cancel.png");
pub const PIN_OFF_ICO: &[u8] = include_bytes!("../resource/icon/pin_off.ico");
pub const PIN_ICO: &[u8] = include_bytes!("../resource/icon/pin.ico");

pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn decode_png(bytes: &[u8]) -> Result<Rgba> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .context("decode png")?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Rgba {
        width,
        height,
        pixels: img.into_raw(),
    })
}

/// Decode `bytes` and resize to exactly `width` × `height`. We keep the PNG
/// source at high resolution (256×256) and let consumers downscale on demand
/// to the size they need (tray icon ≈ 32px, overlay = BASE_ICON_PX×dpi/96).
pub fn decode_png_resized(bytes: &[u8], width: u32, height: u32) -> Result<Rgba> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .context("decode png")?
        .to_rgba8();
    let resized = image::imageops::resize(
        &img,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    Ok(Rgba {
        width,
        height,
        pixels: resized.into_raw(),
    })
}

pub fn pin_on() -> Result<Rgba> {
    decode_png(PIN_ON_PNG)
}

pub fn pin_off() -> Result<Rgba> {
    decode_png(PIN_OFF_PNG)
}

/// Reorder a tightly-packed RGBA buffer into BGRA in-place style — returns a
/// new owned buffer. The DIB section / `UpdateLayeredWindow` paths expect BGRA;
/// callers that need premultiplied alpha then run [`premultiply_bgra_inplace`].
pub fn rgba_to_bgra(src: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; src.len()];
    for (i, chunk) in src.chunks_exact(4).enumerate() {
        let o = i * 4;
        out[o] = chunk[2];     // B
        out[o + 1] = chunk[1]; // G
        out[o + 2] = chunk[0]; // R
        out[o + 3] = chunk[3]; // A
    }
    out
}

/// Multiply BGR by alpha in place. Required for layered windows
/// (`UpdateLayeredWindow` with `AC_SRC_ALPHA`).
pub fn premultiply_bgra_inplace(buf: &mut [u8]) {
    for chunk in buf.chunks_exact_mut(4) {
        let a = chunk[3] as u32;
        chunk[0] = ((chunk[0] as u32 * a) / 255) as u8;
        chunk[1] = ((chunk[1] as u32 * a) / 255) as u8;
        chunk[2] = ((chunk[2] as u32 * a) / 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_on_decodes() {
        let img = pin_on().expect("pin_on png");
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(img.pixels.len() as u32, img.width * img.height * 4);
    }

    #[test]
    fn pin_off_decodes() {
        let img = pin_off().expect("pin_off png");
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(img.pixels.len() as u32, img.width * img.height * 4);
    }

    #[test]
    fn cancel_decodes() {
        let img = decode_png(CANCEL_PNG).expect("cancel png");
        assert!(img.width > 0 && img.height > 0);
        assert_eq!(img.pixels.len() as u32, img.width * img.height * 4);
    }

    #[test]
    fn decode_png_resized_pin_on_to_32() {
        let img = decode_png_resized(PIN_ON_PNG, 32, 32).expect("resize pin_on");
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 32);
        assert_eq!(img.pixels.len(), 32 * 32 * 4);
    }

    #[test]
    fn decode_png_resized_pin_off_to_16() {
        let img = decode_png_resized(PIN_OFF_PNG, 16, 16).expect("resize pin_off");
        assert_eq!(img.width, 16);
        assert_eq!(img.height, 16);
        assert_eq!(img.pixels.len(), 16 * 16 * 4);
    }

    #[test]
    fn decode_png_resized_cancel_to_32_has_visible_pixels() {
        let img = decode_png_resized(CANCEL_PNG, 32, 32).expect("resize cancel");
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 32);
        assert_eq!(img.pixels.len(), 32 * 32 * 4);
        assert!(img.pixels.chunks_exact(4).any(|p| p[3] > 0));
    }

    #[test]
    fn rgba_to_bgra_swaps_red_and_blue() {
        let src: [u8; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
        // first pixel rgba (10,20,30,40) → bgra (30,20,10,40)
        // second pixel rgba (50,60,70,80) → bgra (70,60,50,80)
        let out = rgba_to_bgra(&src);
        assert_eq!(out, [30, 20, 10, 40, 70, 60, 50, 80]);
    }

    #[test]
    fn rgba_to_bgra_returns_same_length() {
        let src = vec![0u8; 16 * 16 * 4];
        let out = rgba_to_bgra(&src);
        assert_eq!(out.len(), src.len());
    }

    #[test]
    fn premultiply_bgra_inplace_zero_alpha_zeros_color() {
        let mut buf = [100, 150, 200, 0, 255, 255, 255, 0];
        premultiply_bgra_inplace(&mut buf);
        assert_eq!(buf[0..3], [0, 0, 0]);
        assert_eq!(buf[3], 0);
        assert_eq!(buf[4..7], [0, 0, 0]);
        assert_eq!(buf[7], 0);
    }

    #[test]
    fn premultiply_bgra_inplace_full_alpha_keeps_color() {
        let mut buf = [10u8, 20, 30, 255, 40, 50, 60, 255];
        premultiply_bgra_inplace(&mut buf);
        assert_eq!(buf[0..3], [10, 20, 30]);
        assert_eq!(buf[3], 255);
        assert_eq!(buf[4..7], [40, 50, 60]);
        assert_eq!(buf[7], 255);
    }

    #[test]
    fn premultiply_bgra_inplace_half_alpha_halves_color() {
        // a=128 (≈0.5) → channel * 128/255 ≈ channel/2
        let mut buf = [200u8, 200, 200, 128];
        premultiply_bgra_inplace(&mut buf);
        // 200 * 128 / 255 = 100
        assert_eq!(buf[0..3], [100, 100, 100]);
        assert_eq!(buf[3], 128);
    }
}
