//! Embedded image assets plus pixel-format helpers.
//!
//! Sources are high-resolution (256×256) PNGs. Each is decoded once on first
//! use and cached; consumers ask for the exact size they need (tray ≈ 32px,
//! cursor = system cursor size, overlay = DPI-scaled badge size).

use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use image::imageops::{self, FilterType};
use image::RgbaImage;

/// Embedded images. The discriminant indexes the decode cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asset {
    /// Badge shown on a pinned window's title bar.
    PinOn = 0,
    /// Tray icon and selection-mode cursor.
    PinOff = 1,
    /// Cursor shown while hovering a badge (click = unpin).
    Cancel = 2,
}

impl Asset {
    const COUNT: usize = 3;

    fn png(self) -> &'static [u8] {
        match self {
            Asset::PinOn => include_bytes!("../resource/icon/pin_on.png"),
            Asset::PinOff => include_bytes!("../resource/icon/pin_off.png"),
            Asset::Cancel => include_bytes!("../resource/icon/cancel.png"),
        }
    }

    fn source(self) -> Result<&'static RgbaImage> {
        static CACHE: [OnceLock<RgbaImage>; Asset::COUNT] =
            [OnceLock::new(), OnceLock::new(), OnceLock::new()];
        let cell = &CACHE[self as usize];
        if let Some(img) = cell.get() {
            return Ok(img);
        }
        let img = image::load_from_memory_with_format(self.png(), image::ImageFormat::Png)
            .with_context(|| format!("decode {self:?} png"))?
            .to_rgba8();
        Ok(cell.get_or_init(|| img))
    }

    /// Decode (cached) and resize to exactly `size` × `size`.
    pub fn render(self, size: u32) -> Result<Rgba> {
        if size == 0 {
            return Err(anyhow!("cannot render {self:?} at size 0"));
        }
        let resized = imageops::resize(self.source()?, size, size, FilterType::Triangle);
        Ok(Rgba {
            width: size,
            height: size,
            pixels: resized.into_raw(),
        })
    }
}

/// Tightly packed, non-premultiplied RGBA8 pixels.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rgba {
    /// Reorder into BGRA, the layout DIB sections and `UpdateLayeredWindow`
    /// expect. Pass `premultiply = true` for layered windows (`AC_SRC_ALPHA`);
    /// cursors (`CreateIconIndirect`) take straight alpha.
    pub fn to_bgra(&self, premultiply: bool) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len());
        for px in self.pixels.chunks_exact(4) {
            let [r, g, b, a] = [px[0], px[1], px[2], px[3]];
            if premultiply {
                let mul = |c: u8| ((c as u32 * a as u32) / 255) as u8;
                out.extend_from_slice(&[mul(b), mul(g), mul(r), a]);
            } else {
                out.extend_from_slice(&[b, g, r, a]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Asset; Asset::COUNT] = [Asset::PinOn, Asset::PinOff, Asset::Cancel];

    fn rgba(pixels: &[u8]) -> Rgba {
        Rgba {
            width: (pixels.len() / 4) as u32,
            height: 1,
            pixels: pixels.to_vec(),
        }
    }

    #[test]
    fn all_assets_decode() {
        for asset in ALL {
            let img = asset.source().expect("decode");
            assert!(img.width() > 0 && img.height() > 0, "{asset:?}");
        }
    }

    #[test]
    fn render_produces_requested_size_with_visible_pixels() {
        for asset in ALL {
            for size in [16, 32, 48] {
                let img = asset.render(size).expect("render");
                assert_eq!((img.width, img.height), (size, size));
                assert_eq!(img.pixels.len(), (size * size * 4) as usize);
                assert!(
                    img.pixels.chunks_exact(4).any(|p| p[3] > 0),
                    "{asset:?} at {size}px is fully transparent"
                );
            }
        }
    }

    #[test]
    fn render_rejects_zero_size() {
        assert!(Asset::PinOn.render(0).is_err());
    }

    #[test]
    fn to_bgra_swaps_red_and_blue() {
        let img = rgba(&[10, 20, 30, 40, 50, 60, 70, 80]);
        assert_eq!(img.to_bgra(false), [30, 20, 10, 40, 70, 60, 50, 80]);
    }

    #[test]
    fn premultiply_zero_alpha_zeros_color() {
        let img = rgba(&[100, 150, 200, 0, 255, 255, 255, 0]);
        assert_eq!(img.to_bgra(true), [0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn premultiply_full_alpha_keeps_color() {
        let img = rgba(&[30, 20, 10, 255]);
        assert_eq!(img.to_bgra(true), [10, 20, 30, 255]);
    }

    #[test]
    fn premultiply_half_alpha_halves_color() {
        // 200 * 128 / 255 = 100
        let img = rgba(&[200, 200, 200, 128]);
        assert_eq!(img.to_bgra(true), [100, 100, 100, 128]);
    }
}
