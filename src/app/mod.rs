//! Controller: startup, the message loop, and shutdown.
//!
//! All application state lives in one [`App`] owned by a thread-local.
//! Window messages and tray events are decoded into [`AppEvent`]s and handed
//! to [`App::handle`], the only place state changes.

mod messages;
mod state;

use std::cell::RefCell;
use std::sync::LazyLock;

use anyhow::Result;
use log::{error, info, warn};
use tray_icon::menu::MenuEvent;
use tray_icon::TrayIconEvent;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostQuitMessage,
    TranslateMessage, HCURSOR, MSG, WM_DESTROY, WM_ENDSESSION, WM_QUERYENDSESSION,
    WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::autostart;
use crate::domain::window::APP_WINDOW_CLASS;
use crate::ui::tray::{is_activation_click, Tray};
use crate::win::cursor::restore_system_cursors;
use crate::win::instance::SingleInstance;
use crate::win::{enable_per_monitor_dpi_awareness, encode_wide, register_class};
use messages::AppEvent;
use state::{App, Flow};

/// Per logon session, so other users on the same machine can run Pin too.
const INSTANCE_MUTEX: &str = r"Local\sqhh99.Pin.SingleInstance";

static CLASS_NAME: LazyLock<Vec<u16>> = LazyLock::new(|| encode_wide(APP_WINDOW_CLASS));

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn run() -> Result<()> {
    enable_per_monitor_dpi_awareness();

    let Some(_instance) = SingleInstance::acquire(INSTANCE_MUTEX)? else {
        info!("another pin instance is already running; exiting");
        return Ok(());
    };

    install_panic_hook();
    // A previous instance that was killed mid-selection may have left the
    // pin cursor installed system-wide.
    restore_system_cursors();

    let autostart_on = autostart::reconcile_on_startup().unwrap_or_else(|e| {
        warn!("autostart reconcile failed: {e:#}");
        autostart::is_enabled()
    });

    let hwnd = create_app_window()?;
    let tray = Tray::new(autostart_on)?;
    APP.set(Some(App::new(hwnd, tray)));
    info!("pin {} started", env!("CARGO_PKG_VERSION"));

    run_message_loop(hwnd);
    teardown();
    Ok(())
}

/// Never leave the pin cursor installed if we crash (release builds abort
/// on panic, so this hook is the last code that runs).
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_system_cursors();
        error!("panic: {info}");
        default_hook(info);
    }));
}

/// A hidden top-level window rather than an `HWND_MESSAGE` one, so Restart
/// Manager, logoff, and the installer (`FindWindow` + `WM_CLOSE`) can reach
/// it and ask Pin to exit cleanly.
fn create_app_window() -> Result<HWND> {
    register_class(&CLASS_NAME, wnd_proc, HCURSOR::default())?;
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(CLASS_NAME.as_ptr()),
            w!("Pin"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            GetModuleHandleW(None)?,
            None,
        )?
    };
    Ok(hwnd)
}

fn run_message_loop(hwnd: HWND) {
    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
            // tray-icon queues its events while we dispatch messages for its
            // hidden window, so drain them right after dispatch.
            pump_tray_events(hwnd);
        }
    }
}

fn pump_tray_events(hwnd: HWND) {
    let mut events = Vec::new();
    while let Ok(ev) = MenuEvent::receiver().try_recv() {
        events.push(AppEvent::Menu(ev.id));
    }
    while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
        if is_activation_click(&ev) {
            events.push(AppEvent::TrayActivated);
        }
    }
    for event in events {
        dispatch(hwnd, event);
    }
}

fn dispatch(hwnd: HWND, event: AppEvent) {
    if with_app(|app| app.handle(event)) == Some(Flow::Quit) {
        // Outside the borrow: DestroyWindow re-enters wnd_proc (WM_DESTROY).
        let _ = unsafe { DestroyWindow(hwnd) };
    }
}

/// Run `f` against the app state. Re-entrant access (a window message sent
/// synchronously while the state is already borrowed) is logged and dropped
/// instead of panicking inside an `extern "system"` callback.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut app) => app.as_mut().map(f),
        Err(_) => {
            error!("re-entrant app state access; event dropped");
            None
        }
    })
}

/// Unpin everything, close the picker, and drop the state (which removes
/// the tray icon). Idempotent.
fn teardown() {
    let app = APP.with(|cell| cell.try_borrow_mut().ok().and_then(|mut app| app.take()));
    if let Some(mut app) = app {
        app.shutdown();
        info!("pin stopped");
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if let Some(event) = messages::decode(msg, wparam, lparam) {
        dispatch(hwnd, event);
        return LRESULT(0);
    }
    match msg {
        WM_QUERYENDSESSION => LRESULT(1),
        WM_ENDSESSION => {
            // Logoff, shutdown, or Restart Manager (installer) closing us.
            if wparam.0 != 0 {
                teardown();
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            teardown();
            PostQuitMessage(0);
            LRESULT(0)
        }
        // DefWindowProc turns WM_CLOSE into DestroyWindow.
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
