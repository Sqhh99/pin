//! Boot autostart via `pin.ini` (user intent) + HKCU `Run` (enforcement).
//!
//! Settings live in `%APPDATA%\Pin\pin.ini` so they are writable no matter
//! where Pin is installed. Older versions kept `pin.ini` next to `pin.exe`;
//! that file is migrated on first read.
//!
//! On startup we reconcile: if the ini says on but `Run` was cleared (e.g. by
//! AV), re-write `Run`; if the ini says off but `Run` remains, remove it.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use log::{info, warn};
use windows::Win32::System::Registry::{KEY_QUERY_VALUE, KEY_SET_VALUE};

use crate::domain::settings::{
    parse_run_command_path, quote_exe_path, reconcile_action, ReconcileAction, RunState, Settings,
    INI_FILE_NAME,
};
use crate::win::registry::RegKey;

const APP_DIR_NAME: &str = "Pin";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "Pin";

/// `%APPDATA%\Pin\pin.ini`.
pub fn settings_path() -> Result<PathBuf> {
    let appdata = std::env::var_os("APPDATA").ok_or_else(|| anyhow!("%APPDATA% is not set"))?;
    Ok(PathBuf::from(appdata)
        .join(APP_DIR_NAME)
        .join(INI_FILE_NAME))
}

/// Pre-0.1.12 location: next to the executable.
fn legacy_settings_path() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join(INI_FILE_NAME))
}

/// Load settings; a missing or unreadable file yields defaults.
pub fn load_settings() -> Settings {
    match try_load_settings() {
        Ok(s) => s,
        Err(e) => {
            warn!("load settings: {e:#}");
            Settings::default()
        }
    }
}

fn try_load_settings() -> Result<Settings> {
    let path = settings_path()?;
    if path.is_file() {
        return read_settings(&path);
    }
    if let Some(legacy) = legacy_settings_path().filter(|p| p.is_file()) {
        let settings = read_settings(&legacy)?;
        info!("migrating settings from {}", legacy.display());
        if let Err(e) = save_settings(settings) {
            warn!("migrate settings: {e:#}");
        }
        return Ok(settings);
    }
    Ok(Settings::default())
}

fn read_settings(path: &Path) -> Result<Settings> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(Settings::parse(&text))
}

fn save_settings(settings: Settings) -> Result<()> {
    let path = settings_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::write(&path, settings.to_ini()).with_context(|| format!("write {}", path.display()))
}

/// Whether the user wants Pin to start with Windows.
pub fn is_enabled() -> bool {
    load_settings().autostart
}

/// Record the user's choice, then enforce it in the registry. The ini is
/// written first so a registry failure is repaired by the next startup
/// reconcile rather than silently forgotten.
pub fn set_enabled(on: bool) -> Result<()> {
    let mut settings = load_settings();
    settings.autostart = on;
    save_settings(settings)?;
    set_run_value(on)
}

/// Read ini, compare `Run`, repair drift. Returns the user's intent.
pub fn reconcile_on_startup() -> Result<bool> {
    let desired = is_enabled();
    match reconcile_action(desired, run_state()?) {
        ReconcileAction::Noop => {}
        ReconcileAction::EnableRegistry => {
            info!("autostart reconcile: ini=on, Run is not this exe — restoring Run");
            set_run_value(true)?;
        }
        ReconcileAction::DisableRegistry => {
            info!("autostart reconcile: ini=off, Run present — removing Run");
            set_run_value(false)?;
        }
    }
    Ok(desired)
}

fn run_state() -> Result<RunState> {
    let key = match RegKey::open_current_user(RUN_KEY, KEY_QUERY_VALUE) {
        Ok(key) => key,
        // A missing Run key simply means nothing is registered.
        Err(_) => return Ok(RunState::Missing),
    };
    let Some(path) = key
        .get_string(RUN_VALUE_NAME)?
        .as_deref()
        .and_then(parse_run_command_path)
    else {
        return Ok(RunState::Missing);
    };
    let current = std::env::current_exe().context("current_exe")?;
    Ok(if paths_equal(&path, &current) {
        RunState::CurrentExe
    } else {
        RunState::Other
    })
}

fn set_run_value(on: bool) -> Result<()> {
    let key = RegKey::open_current_user(RUN_KEY, KEY_SET_VALUE)?;
    if on {
        let exe = std::env::current_exe().context("current_exe")?;
        key.set_string(RUN_VALUE_NAME, &quote_exe_path(&exe))
    } else {
        key.delete_value(RUN_VALUE_NAME)
    }
}

/// Best-effort comparison tolerant of casing and symlink differences.
fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}
