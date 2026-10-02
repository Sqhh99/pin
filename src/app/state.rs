//! Application state and the single place where it changes.

use log::{debug, error, info, warn};
use windows::Win32::Foundation::HWND;

use crate::app::messages::AppEvent;
use crate::autostart;
use crate::domain::pinned::PinnedSet;
use crate::domain::window::WindowId;
use crate::ui::overlay::Overlay;
use crate::ui::picker::Picker;
use crate::ui::tray::{Tray, TrayCommand};
use crate::win::RealWindowApi;

/// What the message loop should do after an event.
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Quit,
}

pub(crate) struct App {
    /// App window: receives view notifications.
    hwnd: HWND,
    api: RealWindowApi,
    /// Each pinned window owns its badge; removing the entry destroys it.
    pinned: PinnedSet<Overlay>,
    tray: Tray,
    picker: Option<Picker>,
}

impl App {
    pub fn new(hwnd: HWND, tray: Tray) -> Self {
        Self {
            hwnd,
            api: RealWindowApi,
            pinned: PinnedSet::new(),
            tray,
            picker: None,
        }
    }

    pub fn handle(&mut self, event: AppEvent) -> Flow {
        match event {
            AppEvent::Picked { picker, target } => {
                if self.finish_picker(picker) {
                    self.pin(target);
                }
            }
            AppEvent::PickCanceled { picker } => {
                if self.finish_picker(picker) {
                    debug!("pick canceled");
                }
            }
            AppEvent::UnpinRequested { target, overlay } => {
                // Ignore requests from a badge that has since been replaced.
                if self.pinned.get(target).is_some_and(|o| o.hwnd() == overlay) {
                    self.unpin(target);
                }
            }
            AppEvent::TrayActivated => self.toggle_picker(),
            AppEvent::Menu(id) => match self.tray.command_for(&id) {
                Some(TrayCommand::UnpinAll) => self.unpin_all(),
                Some(TrayCommand::ToggleAutostart) => self.toggle_autostart(),
                Some(TrayCommand::Quit) => {
                    info!("quit requested");
                    return Flow::Quit;
                }
                None => {}
            },
        }
        Flow::Continue
    }

    /// Release everything we changed in other windows and the system.
    pub fn shutdown(&mut self) {
        self.picker = None;
        self.unpin_all();
    }

    /// Close the active picker if `hwnd` is it. Results from a picker that
    /// was already closed are ignored.
    fn finish_picker(&mut self, hwnd: HWND) -> bool {
        if self.picker.as_ref().is_some_and(|p| p.hwnd() == hwnd) {
            self.picker = None;
            true
        } else {
            debug!("ignoring result from stale picker {:?}", hwnd.0);
            false
        }
    }

    fn toggle_picker(&mut self) {
        if self.picker.take().is_some() {
            info!("selection cancelled via tray");
            return;
        }
        match Picker::open(self.hwnd) {
            Ok(p) => self.picker = Some(p),
            Err(e) => error!("open picker: {e:#}"),
        }
    }

    fn pin(&mut self, target: WindowId) {
        if self.pinned.contains(target) {
            info!("window {:#x} already pinned; ignoring", target.0);
            return;
        }
        // If pinning fails the overlay is dropped (and its window destroyed).
        let result = Overlay::create(target, self.hwnd)
            .and_then(|overlay| self.pinned.pin(&self.api, target, overlay));
        match result {
            Ok(true) => info!("pinned window {:#x}", target.0),
            Ok(false) => {}
            Err(e) => warn!("pin {:#x} failed: {e:#}", target.0),
        }
    }

    fn unpin(&mut self, target: WindowId) {
        if self.pinned.unpin(&self.api, target).is_some() {
            info!("unpinned window {:#x}", target.0);
        }
    }

    fn unpin_all(&mut self) {
        let removed = self.pinned.unpin_all(&self.api);
        if !removed.is_empty() {
            info!("unpinned {} window(s)", removed.len());
        }
    }

    fn toggle_autostart(&mut self) {
        let on = !autostart::is_enabled();
        match autostart::set_enabled(on) {
            Ok(()) => {
                info!("autostart {}", if on { "enabled" } else { "disabled" });
                self.tray.set_autostart_checked(on);
            }
            Err(e) => {
                warn!("autostart toggle failed: {e:#}");
                self.tray.set_autostart_checked(autostart::is_enabled());
            }
        }
    }
}
