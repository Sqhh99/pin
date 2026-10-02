//! Screen-space geometry and overlay layout rules.

use crate::domain::window::WindowId;

/// Screen rectangle in physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn center(&self) -> (i32, i32) {
        ((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }
}

/// DPI that Windows treats as 100% scaling.
pub const BASE_DPI: u32 = 96;

/// Default overlay icon side length in physical pixels at 96 DPI.
/// Picked to be roughly the height of the caption button glyph strip so the
/// pin reads as a sibling of the min/max/close icons rather than a tiny dot.
pub const BASE_ICON_PX: i32 = 24;

/// Width of one caption button (min/max/close) at 96 DPI.
const CAPTION_BUTTON_PX: f32 = 46.0;
const CAPTION_BUTTON_COUNT: i32 = 3;
const ICON_GAP_PX: f32 = 4.0;
const ICON_TOP_PX: f32 = 3.0;

/// `GetDpiForWindow` returns 0 on failure; treat that as 100%.
pub fn normalize_dpi(dpi: u32) -> u32 {
    if dpi == 0 {
        BASE_DPI
    } else {
        dpi
    }
}

fn scale_for_dpi(dpi: u32) -> f32 {
    (normalize_dpi(dpi) as f32 / BASE_DPI as f32).max(1.0)
}

fn scaled(px: f32, dpi: u32) -> i32 {
    (px * scale_for_dpi(dpi)).round() as i32
}

/// Overlay icon side length for a target on a monitor with `dpi`.
pub fn icon_px_for_dpi(dpi: u32) -> i32 {
    scaled(BASE_ICON_PX as f32, dpi)
}

/// Compute the screen rectangle for the floating icon, given the target's
/// *visible* frame bounds.
///
/// Position follows DeskPins' `placeOnCaption` (`DeskPins/pinwnd.cpp:323-361`):
/// `x = right - (3 * caption_btn_w + icon_w + 4)`, `y = top + 3`, all DPI-scaled.
pub fn compute_overlay_rect(window: Rect, dpi: u32) -> Rect {
    let icon = icon_px_for_dpi(dpi);
    let strip = scaled(CAPTION_BUTTON_PX, dpi) * CAPTION_BUTTON_COUNT;
    let right = window.right - strip - scaled(ICON_GAP_PX, dpi);
    let top = window.top + scaled(ICON_TOP_PX, dpi);
    Rect {
        left: right - icon,
        top,
        right,
        bottom: top + icon,
    }
}

/// Decide whether the overlay should be visible given what
/// `WindowFromPoint` returns at its center: show it only when the point hits
/// the overlay itself, the target, or one of the target's children — i.e.
/// nothing unrelated covers the target's title bar there.
pub fn should_show_overlay_for_hit(
    hit: Option<WindowId>,
    overlay: WindowId,
    target: WindowId,
    hit_is_target_child: bool,
) -> bool {
    match hit {
        None => false,
        Some(hit) => hit == overlay || hit == target || hit_is_target_child,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OVERLAY: WindowId = WindowId(0x1000);
    const TARGET: WindowId = WindowId(0x2000);
    const TARGET_CHILD: WindowId = WindowId(0x2001);
    const OTHER: WindowId = WindowId(0x3000);

    #[test]
    fn overlay_rect_inside_window_at_96_dpi() {
        let win = Rect {
            left: 100,
            top: 200,
            right: 900,
            bottom: 800,
        };
        let r = compute_overlay_rect(win, 96);
        assert!(r.left > win.left && r.right < win.right);
        assert_eq!(r.width(), BASE_ICON_PX);
        assert_eq!(r.height(), BASE_ICON_PX);
    }

    #[test]
    fn overlay_rect_sits_left_of_caption_buttons() {
        let win = Rect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 600,
        };
        let r = compute_overlay_rect(win, 96);
        assert_eq!(r.right, 1000 - 3 * 46 - 4);
        assert_eq!(r.top, 3);
    }

    #[test]
    fn overlay_rect_scales_with_dpi() {
        let win = Rect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 600,
        };
        let r96 = compute_overlay_rect(win, 96);
        let r192 = compute_overlay_rect(win, 192);
        assert_eq!(r192.width(), r96.width() * 2);
        assert_eq!(r192.height(), r96.height() * 2);
    }

    #[test]
    fn overlay_rect_anchored_near_top_right() {
        let win = Rect {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 600,
        };
        let r = compute_overlay_rect(win, 96);
        assert!(r.top < 30, "top={} should be in title bar", r.top);
        assert!(r.right > win.right * 8 / 10, "should sit near right edge");
    }

    #[test]
    fn zero_dpi_is_treated_as_100_percent() {
        assert_eq!(normalize_dpi(0), BASE_DPI);
        assert_eq!(icon_px_for_dpi(0), BASE_ICON_PX);
        assert_eq!(icon_px_for_dpi(144), 36);
    }

    #[test]
    fn overlay_visibility_keeps_visible_for_owning_windows() {
        assert!(should_show_overlay_for_hit(
            Some(OVERLAY),
            OVERLAY,
            TARGET,
            false
        ));
        assert!(should_show_overlay_for_hit(
            Some(TARGET),
            OVERLAY,
            TARGET,
            false
        ));
        assert!(should_show_overlay_for_hit(
            Some(TARGET_CHILD),
            OVERLAY,
            TARGET,
            true
        ));
    }

    #[test]
    fn overlay_visibility_hides_for_empty_or_unrelated_hits() {
        assert!(!should_show_overlay_for_hit(None, OVERLAY, TARGET, false));
        assert!(!should_show_overlay_for_hit(
            Some(OTHER),
            OVERLAY,
            TARGET,
            false
        ));
    }
}
