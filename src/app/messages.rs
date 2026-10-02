//! Everything the controller reacts to, decoded from window messages and
//! tray-icon events.

use tray_icon::menu::MenuId;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};

use crate::domain::window::WindowId;
use crate::ui::notify;

#[derive(Debug)]
pub(crate) enum AppEvent {
    /// The picker identified `target`.
    Picked { picker: HWND, target: WindowId },
    /// The picker was cancelled.
    PickCanceled { picker: HWND },
    /// A badge was clicked or its target disappeared.
    UnpinRequested { target: WindowId, overlay: HWND },
    /// Left click on the tray icon.
    TrayActivated,
    /// A tray menu item was chosen.
    Menu(MenuId),
}

/// Decode a [`notify`] message posted by a view; `None` for anything else.
pub(crate) fn decode(msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<AppEvent> {
    let wparam_window = WindowId(wparam.0 as isize);
    let lparam_hwnd = HWND(lparam.0 as *mut _);
    match msg {
        notify::PICKED => Some(AppEvent::Picked {
            picker: lparam_hwnd,
            target: wparam_window,
        }),
        notify::PICK_CANCELED => Some(AppEvent::PickCanceled {
            picker: lparam_hwnd,
        }),
        notify::UNPIN_REQUESTED => Some(AppEvent::UnpinRequested {
            target: wparam_window,
            overlay: lparam_hwnd,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{WM_APP, WM_USER};

    #[test]
    fn notify_ids_are_distinct_app_messages() {
        let ids = [
            notify::PICKED,
            notify::PICK_CANCELED,
            notify::UNPIN_REQUESTED,
        ];
        for (i, a) in ids.iter().enumerate() {
            assert!(*a >= WM_APP && *a < 0xC000);
            for b in &ids[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert!(decode(WM_USER, WPARAM(0), LPARAM(0)).is_none());
    }

    #[test]
    fn decode_round_trips_handles() {
        let ev = decode(notify::PICKED, WPARAM(0x1234), LPARAM(0x5678)).unwrap();
        match ev {
            AppEvent::Picked { picker, target } => {
                assert_eq!(target, WindowId(0x1234));
                assert_eq!(picker.0 as isize, 0x5678);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
