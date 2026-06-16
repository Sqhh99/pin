//! Main thread message loop. Owns the [`PinnedSet`], tray, and picker state.

#![cfg(windows)]

use anyhow::{anyhow, Result};
use log::{debug, error, info, warn};
use std::cell::RefCell;

use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    RegisterClassExW, TranslateMessage, HWND_MESSAGE, MSG, WNDCLASSEXW, WS_OVERLAPPED,
};

use crate::autostart;
use crate::overlay::{create_overlay, destroy_overlay, UNPIN_NOTIFY};
use crate::pinned::{PinnedEntry, PinnedSet};
use crate::resources::{PIN_OFF_PNG, PIN_ON_PNG};
use crate::selection::{Picker, PICKED_MSG, PICK_CANCELED_MSG};
use crate::tray::Tray;
use crate::win::{real::RealWindowApi, WindowId};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;

const CLASS_NAME: PCWSTR = w!("PinAppMsgWindow");

thread_local! {
    static STATE: RefCell<Option<AppState>> = RefCell::new(None);
}

struct AppState {
    pinned: PinnedSet,
    tray: Tray,
    api: RealWindowApi,
    picker: Option<Picker>,
    main_hwnd: HWND,
}

pub fn run() -> Result<()> {
    if another_instance_running()? {
        info!("another pin instance is already running; exiting");
        return Ok(());
    }

    match autostart::reconcile_on_startup() {
        Ok(state) => {
            debug!(
                "autostart: desired={} effective={}",
                state.desired, state.effective
            );
        }
        Err(e) => warn!("autostart reconcile failed: {e}"),
    }

    let main_hwnd = create_message_window()?;
    let tray = Tray::new()?;
    STATE.with(|s| {
        *s.borrow_mut() = Some(AppState {
            pinned: PinnedSet::new(),
            tray,
            api: RealWindowApi,
            picker: None,
            main_hwnd,
        });
    });

    info!("pin started");

    unsafe {
        let mut msg = MSG::default();
        loop {
            let r = GetMessageW(&mut msg, None, 0, 0);
            if r.0 <= 0 {
                break;
            }
            drain_tray_events();
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Clean shutdown: drop picker; destroy overlays (overlay's WM_DESTROY
    // will clear TOPMOST on its target); then belt-and-suspenders drain set.
    STATE.with(|s| {
        if let Some(state) = s.borrow_mut().as_mut() {
            state.picker = None;
            let drained = state.pinned.drain(&state.api);
            for (_w, entry) in drained {
                destroy_overlay_isize(entry.overlay);
            }
        }
    });
    Ok(())
}

fn create_message_window() -> Result<HWND> {
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
            return Err(anyhow!("RegisterClassExW for app window failed"));
        }
        let hwnd = CreateWindowExW(
            Default::default(),
            CLASS_NAME,
            w!("pin"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )?;
        Ok(hwnd)
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        m if m == PICKED_MSG => {
            let w = WindowId(wparam.0 as isize);
            info!("picked window {:#x}", w.0);
            STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    state.picker = None;
                    if let Err(e) = pin_window(state, w) {
                        warn!("pin failed: {e}");
                    }
                }
            });
            LRESULT(0)
        }
        m if m == PICK_CANCELED_MSG => {
            debug!("pick canceled");
            STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    state.picker = None;
                }
            });
            LRESULT(0)
        }
        m if m == UNPIN_NOTIFY => {
            let w = WindowId(wparam.0 as isize);
            info!("unpin notify {:#x}", w.0);
            STATE.with(|s| {
                if let Some(state) = s.borrow_mut().as_mut() {
                    // Overlay already cleared TOPMOST + destroyed itself;
                    // we only need to drop the bookkeeping entry.
                    let _ = state.pinned.unpin(&state.api, w);
                }
            });
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn pin_window(state: &mut AppState, w: WindowId) -> Result<()> {
    if state.pinned.contains(w) {
        info!("window {:#x} already pinned; ignoring", w.0);
        return Ok(());
    }
    debug!("pin_window: creating overlay for {:#x}", w.0);
    let overlay = create_overlay(w, state.main_hwnd, PIN_ON_PNG)?;
    debug!("pin_window: overlay hwnd = {:?}", overlay.0);
    let entry = PinnedEntry { overlay: overlay.0 as isize };
    if !state.pinned.pin(&state.api, w, entry)? {
        destroy_overlay(overlay);
    } else {
        info!("pinned window {:#x}", w.0);
    }
    Ok(())
}

fn destroy_overlay_isize(h: isize) {
    if h != 0 {
        destroy_overlay(HWND(h as *mut _));
    }
}

fn another_instance_running() -> Result<bool> {
    unsafe {
        let name = encode_wide("Global\\sqhh99.Pin.SingleInstance");
        let _ = CreateMutexW(None, true, PCWSTR(name.as_ptr()))?;
        Ok(GetLastError() == ERROR_ALREADY_EXISTS)
    }
}

fn encode_wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// Drain tray and menu event channels. Called once per message-loop iteration.
fn drain_tray_events() {
    let menu_rx = MenuEvent::receiver();
    while let Ok(ev) = menu_rx.try_recv() {
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().as_mut() {
                if ev.id == state.tray.menu_id_quit {
                    info!("quit requested");
                    state.picker = None;
                    let drained = state.pinned.drain(&state.api);
                    for (_w, entry) in drained {
                        destroy_overlay_isize(entry.overlay);
                    }
                    unsafe { PostQuitMessage(0) };
                } else if ev.id == state.tray.menu_id_unpin_all {
                    info!("unpin all");
                    let drained = state.pinned.drain(&state.api);
                    for (_w, entry) in drained {
                        destroy_overlay_isize(entry.overlay);
                    }
                } else if ev.id == state.tray.menu_id_autostart {
                    let new_on = !autostart::is_desired();
                    match autostart::apply(new_on) {
                        Ok(()) => {
                            let _ = state.tray.autostart_item.set_checked(new_on);
                            info!(
                                "autostart {}",
                                if new_on { "enabled" } else { "disabled" }
                            );
                        }
                        Err(e) => warn!("autostart toggle failed: {e}"),
                    }
                }
            }
        });
    }

    let tray_rx = TrayIconEvent::receiver();
    while let Ok(ev) = tray_rx.try_recv() {
        // `Click` fires twice per single click (Down then Up). Trigger only on
        // the left-button Up edge so a single click toggles picker once.
        let trigger = matches!(
            ev,
            TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            }
        );
        if !trigger {
            continue;
        }
        STATE.with(|s| {
            if let Some(state) = s.borrow_mut().as_mut() {
                if state.picker.is_some() {
                    info!("toggle picker off via tray");
                    state.picker = None;
                    return;
                }
                match Picker::open(state.main_hwnd, PIN_OFF_PNG) {
                    Ok(p) => state.picker = Some(p),
                    Err(e) => error!("open picker: {e}"),
                }
            }
        });
    }
}
