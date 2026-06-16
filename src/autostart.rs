//! Boot autostart via `pin.ini` (user intent) + HKCU Run (enforcement).
//!
//! On startup we reconcile: if ini says on but Run was cleared (e.g. by AV),
//! re-write Run; if ini says off but Run remains, remove it.

#![cfg(windows)]

use anyhow::{anyhow, Context, Result};
use log::{info, warn};
use std::path::{Path, PathBuf};

const INI_FILE_NAME: &str = "pin.ini";
const INI_KEY: &str = "AutoStart";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE_NAME: &str = "Pin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoStartState {
    pub desired: bool,
    pub effective: bool,
}

/// Path to `pin.ini` next to the running executable.
pub fn ini_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("current_exe")?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("executable has no parent directory"))?;
    Ok(dir.join(INI_FILE_NAME))
}

/// User intent from ini; missing or invalid file => `false`.
pub fn read_desired() -> bool {
    match read_desired_result() {
        Ok(v) => v,
        Err(e) => {
            warn!("read autostart ini: {e}");
            false
        }
    }
}

pub fn is_desired() -> bool {
    read_desired()
}

pub fn apply(on: bool) -> Result<()> {
    write_desired(on)?;
    set_registry(on)?;
    Ok(())
}

/// Read ini, compare Run, repair drift. Returns final state after reconcile.
pub fn reconcile_on_startup() -> Result<AutoStartState> {
    let desired = read_desired_result().unwrap_or(false);
    let effective = registry_points_to_current_exe()?;
    let action = reconcile_action(desired, effective);
    match action {
        ReconcileAction::Noop => {}
        ReconcileAction::EnableRegistry => {
            info!("autostart reconcile: ini=true, run missing — restoring Run");
            set_registry(true)?;
        }
        ReconcileAction::DisableRegistry => {
            info!("autostart reconcile: ini=false, run present — removing Run");
            set_registry(false)?;
        }
    }
    let effective_after = registry_points_to_current_exe()?;
    Ok(AutoStartState {
        desired,
        effective: effective_after,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcileAction {
    Noop,
    EnableRegistry,
    DisableRegistry,
}

fn reconcile_action(desired: bool, effective: bool) -> ReconcileAction {
    if desired && !effective {
        ReconcileAction::EnableRegistry
    } else if !desired && effective {
        ReconcileAction::DisableRegistry
    } else {
        ReconcileAction::Noop
    }
}

fn read_desired_result() -> Result<bool> {
    let path = ini_path()?;
    if !path.is_file() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(parse_ini_autostart(&text))
}

fn write_desired(on: bool) -> Result<()> {
    let path = ini_path()?;
    let body = format_ini_autostart(on);
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))
}

fn parse_ini_autostart(text: &str) -> bool {
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case(INI_KEY) {
                return parse_bool(value.trim());
            }
        }
    }
    false
}

fn format_ini_autostart(on: bool) -> String {
    format!("{INI_KEY}={}\n", if on { "true" } else { "false" })
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn registry_points_to_current_exe() -> Result<bool> {
    let stored = read_run_value()?;
    let Some(stored) = stored else {
        return Ok(false);
    };
    let current = std::env::current_exe().context("current_exe")?;
    Ok(paths_equal(&stored, &current))
}

fn set_registry(on: bool) -> Result<()> {
    if on {
        let exe = std::env::current_exe().context("current_exe")?;
        let quoted = quote_exe_path(&exe);
        write_run_value(&quoted)
    } else {
        delete_run_value()
    }
}

fn quote_exe_path(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    // Best-effort canonical compare for symlink / casing differences.
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}

fn read_run_value() -> Result<Option<PathBuf>> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ,
    };

    unsafe {
        let mut hkey = HKEY::default();
        let subkey: Vec<u16> = encode_wide(RUN_KEY);
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        )
        .map_err(|e| anyhow!("RegOpenKeyExW(Run): {e}"))?;

        let result = (|| {
            let name: Vec<u16> = encode_wide(RUN_VALUE_NAME);
            let mut data_type = REG_SZ.0;
            let mut buf = vec![0u16; 512];
            let mut cb = (buf.len() * 2) as u32;
            let err = RegQueryValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                None,
                Some(&mut data_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut cb),
            );
            if err.is_err() {
                return Ok(None);
            }
            let n = (cb as usize / 2).saturating_sub(1);
            buf.truncate(n);
            let s = OsString::from_wide(&buf);
            let path = PathBuf::from(s.to_string_lossy().trim_matches('"').to_string());
            Ok(Some(path))
        })();

        let _ = RegCloseKey(hkey);
        result
    }
}

fn write_run_value(quoted: &str) -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_SZ,
    };

    unsafe {
        let mut hkey = HKEY::default();
        let subkey: Vec<u16> = encode_wide(RUN_KEY);
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
        .map_err(|e| anyhow!("RegOpenKeyExW(Run): {e}"))?;

        let name: Vec<u16> = encode_wide(RUN_VALUE_NAME);
        let value: Vec<u16> = encode_wide(quoted);
        let bytes =
            std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2 + 2);
        let result = RegSetValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            0,
            REG_SZ,
            Some(bytes),
        )
        .map_err(|e| anyhow!("RegSetValueExW(Run): {e}"));

        let _ = RegCloseKey(hkey);
        result.map(|_| ())
    }
}

fn delete_run_value() -> Result<()> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };

    unsafe {
        let mut hkey = HKEY::default();
        let subkey: Vec<u16> = encode_wide(RUN_KEY);
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
        .is_err()
        {
            return Ok(());
        }

        let name: Vec<u16> = encode_wide(RUN_VALUE_NAME);
        let _ = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
        let _ = RegCloseKey(hkey);
        Ok(())
    }
}

fn encode_wide(s: &str) -> Vec<u16> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ini_true_variants() {
        assert!(parse_ini_autostart("AutoStart=true\n"));
        assert!(parse_ini_autostart("autostart=1"));
        assert!(parse_ini_autostart("AutoStart=yes"));
    }

    #[test]
    fn parse_ini_false_or_missing() {
        assert!(!parse_ini_autostart("AutoStart=false\n"));
        assert!(!parse_ini_autostart(""));
        assert!(!parse_ini_autostart("# comment\n"));
    }

    #[test]
    fn format_ini_roundtrip() {
        assert_eq!(format_ini_autostart(true), "AutoStart=true\n");
        assert!(parse_ini_autostart(&format_ini_autostart(true)));
        assert!(!parse_ini_autostart(&format_ini_autostart(false)));
    }

    #[test]
    fn reconcile_action_matrix() {
        assert_eq!(
            reconcile_action(true, false),
            ReconcileAction::EnableRegistry
        );
        assert_eq!(
            reconcile_action(false, true),
            ReconcileAction::DisableRegistry
        );
        assert_eq!(reconcile_action(true, true), ReconcileAction::Noop);
        assert_eq!(reconcile_action(false, false), ReconcileAction::Noop);
    }

    #[test]
    fn quote_exe_path_wraps() {
        let p = PathBuf::from(r"C:\Program Files\Pin\pin.exe");
        assert_eq!(
            quote_exe_path(&p),
            r#""C:\Program Files\Pin\pin.exe""#
        );
    }
}