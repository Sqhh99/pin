//! System-tray icon + right-click menu.

#![cfg(windows)]

use anyhow::{anyhow, Context, Result};
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon as TrayIconImg, TrayIcon, TrayIconBuilder};

use crate::autostart;
use crate::resources::{decode_png_resized, Rgba, PIN_OFF_PNG};

/// Side length we resize the tray icon to before handing it to `tray-icon`.
/// Windows scales tray icons to ~16-24px; 32 keeps things crisp at high DPI.
const TRAY_ICON_PX: u32 = 32;

pub struct Tray {
    _icon: TrayIcon,
    pub menu_id_unpin_all: tray_icon::menu::MenuId,
    pub menu_id_autostart: tray_icon::menu::MenuId,
    pub menu_id_quit: tray_icon::menu::MenuId,
    pub autostart_item: CheckMenuItem,
}

impl Tray {
    pub fn new() -> Result<Self> {
        let img = decode_png_resized(PIN_OFF_PNG, TRAY_ICON_PX, TRAY_ICON_PX)
            .context("decode tray icon")?;
        let icon = into_tray_icon(&img)?;

        let menu = Menu::new();
        let unpin_all = MenuItem::new("Unpin all", true, None);
        let autostart_on = autostart::is_desired();
        let autostart_item = CheckMenuItem::new("Start at login", true, autostart_on, None);
        let quit = MenuItem::new("Quit", true, None);
        menu.append(&unpin_all).map_err(|e| anyhow!("menu append: {e}"))?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| anyhow!("menu separator: {e}"))?;
        menu.append(&autostart_item)
            .map_err(|e| anyhow!("menu append: {e}"))?;
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| anyhow!("menu separator: {e}"))?;
        menu.append(&quit).map_err(|e| anyhow!("menu append: {e}"))?;

        let tray = TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Pin — click to select a window")
            .build()
            .map_err(|e| anyhow!("tray build: {e}"))?;

        Ok(Self {
            _icon: tray,
            menu_id_unpin_all: unpin_all.id().clone(),
            menu_id_autostart: autostart_item.id().clone(),
            menu_id_quit: quit.id().clone(),
            autostart_item,
        })
    }
}

fn into_tray_icon(img: &Rgba) -> Result<TrayIconImg> {
    TrayIconImg::from_rgba(img.pixels.clone(), img.width, img.height)
        .map_err(|e| anyhow!("tray icon from rgba: {e}"))
}