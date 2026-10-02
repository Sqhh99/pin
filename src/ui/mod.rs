//! Windows that Pin owns. These are views: they draw, track, and report user
//! input to the controller (`crate::app`) by posting [`notify`] messages to
//! the app window. They never touch the pinned set or settings themselves.

pub mod overlay;
pub mod picker;
pub mod tray;

/// Messages views post to the app window (the view → controller protocol).
pub mod notify {
    use windows::Win32::UI::WindowsAndMessaging::WM_APP;

    /// The user picked a window. `wparam` = target HWND, `lparam` = picker HWND.
    pub const PICKED: u32 = WM_APP + 4;
    /// Selection was cancelled. `lparam` = picker HWND.
    pub const PICK_CANCELED: u32 = WM_APP + 5;
    /// A badge was clicked or its target vanished. `wparam` = target HWND,
    /// `lparam` = overlay HWND.
    pub const UNPIN_REQUESTED: u32 = WM_APP + 6;
}
