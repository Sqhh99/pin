//! System-tray icon + right-click menu.

use anyhow::{anyhow, Context, Result};
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::resources::Asset;

/// Side length we render the tray icon at before handing it to `tray-icon`.
/// Windows scales tray icons to ~16-24px; 32 keeps things crisp at high DPI.
const TRAY_ICON_PX: u32 = 32;

/// Menu actions the app reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    UnpinAll,
    ToggleAutostart,
    Quit,
}

pub struct Tray {
    // Dropping the TrayIcon removes it from the notification area.
    _icon: TrayIcon,
    unpin_all: MenuId,
    autostart: MenuId,
    quit: MenuId,
    autostart_item: CheckMenuItem,
}

impl Tray {
    pub fn new(autostart_on: bool) -> Result<Self> {
        let img = Asset::PinOff
            .render(TRAY_ICON_PX)
            .context("render tray icon")?;
        let icon = Icon::from_rgba(img.pixels, img.width, img.height)
            .map_err(|e| anyhow!("tray icon from rgba: {e}"))?;

        let unpin_all = MenuItem::new("Unpin all", true, None);
        let autostart_item = CheckMenuItem::new("Start with Windows", true, autostart_on, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::with_items(&[
            &unpin_all,
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &PredefinedMenuItem::separator(),
            &quit,
        ])
        .map_err(|e| anyhow!("build tray menu: {e}"))?;

        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Pin — click to select a window")
            .build()
            .map_err(|e| anyhow!("tray build: {e}"))?;

        Ok(Self {
            _icon: tray,
            unpin_all: unpin_all.id().clone(),
            autostart: autostart_item.id().clone(),
            quit: quit.id().clone(),
            autostart_item,
        })
    }

    pub fn command_for(&self, id: &MenuId) -> Option<TrayCommand> {
        if *id == self.unpin_all {
            Some(TrayCommand::UnpinAll)
        } else if *id == self.autostart {
            Some(TrayCommand::ToggleAutostart)
        } else if *id == self.quit {
            Some(TrayCommand::Quit)
        } else {
            None
        }
    }

    pub fn set_autostart_checked(&self, on: bool) {
        self.autostart_item.set_checked(on);
    }
}

/// `Click` fires twice per single click (Down then Up). Only the left-button
/// Up edge toggles selection mode.
pub fn is_activation_click(ev: &TrayIconEvent) -> bool {
    matches!(
        ev,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}
