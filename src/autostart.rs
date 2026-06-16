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
    set_registry(on)?;
    write_desired(on)?;
    Ok(())
}

pub fn current_state() -> Result<AutoStartState> {
    let desired = read_desired_result().unwrap_or(false);
    let run_state = current_run_state()?;
    Ok(AutoStartState {
        desired,
        effective: run_state == RunState::CurrentExe,
    })
}

pub fn next_toggle_state() -> Result<bool> {
    Ok(toggle_target(current_state()?))
}

/// Read ini, compare Run, repair drift. Returns final state after reconcile.
pub fn reconcile_on_startup() -> Result<AutoStartState> {
    let desired = read_desired_result().unwrap_or(false);
    let run_state = current_run_state()?;
    let action = reconcile_action(desired, run_state);
    match action {
        ReconcileAction::Noop => {}
        ReconcileAction::EnableRegistry => {
            info!("autostart reconcile: ini=true, Run is not current — restoring Run");
            set_registry(true)?;
        }
        ReconcileAction::DisableRegistry => {
            info!("autostart reconcile: ini=false, run present — removing Run");
            set_registry(false)?;
        }
    }
    let effective_after = current_run_state()? == RunState::CurrentExe;
    Ok(AutoStartState {
        desired,
        effective: effective_after,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunState {
    Missing,
    CurrentExe,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcileAction {
    Noop,
    EnableRegistry,
    DisableRegistry,
}

fn reconcile_action(desired: bool, run_state: RunState) -> ReconcileAction {
    match (desired, run_state) {
        (true, RunState::CurrentExe) | (false, RunState::Missing | RunState::Other) => {
            ReconcileAction::Noop
        }
        (true, RunState::Missing | RunState::Other) => ReconcileAction::EnableRegistry,
        (false, RunState::CurrentExe) => ReconcileAction::DisableRegistry,
    }
}

fn toggle_target(state: AutoStartState) -> bool {
    !state.desired
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
    let mut parsed = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim().eq_ignore_ascii_case(INI_KEY) {
                parsed = Some(parse_bool(value.trim()));
            }
        }
    }
    parsed.unwrap_or(false)
}

fn format_ini_autostart(on: bool) -> String {
    format!("{INI_KEY}={}\n", if on { "true" } else { "false" })
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn current_run_state() -> Result<RunState> {
    let stored = read_run_value()?;
    let Some(stored) = stored else {
        return Ok(RunState::Missing);
    };
    let current = std::env::current_exe().context("current_exe")?;
    if paths_equal(&stored, &current) {
        Ok(RunState::CurrentExe)
    } else {
        Ok(RunState::Other)
    }
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
        .ok()
        .map_err(|e| anyhow!("RegOpenKeyExW(Run): {e}"))?;

        let result = (|| {
            let name: Vec<u16> = encode_wide(RUN_VALUE_NAME);
            let mut data_type = REG_SZ;
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
            let mut n = cb as usize / 2;
            if n > 0 && buf.get(n - 1) == Some(&0) {
                n -= 1;
            }
            buf.truncate(n);
            let s = OsString::from_wide(&buf);
            let raw = s.to_string_lossy();
            let Some(path) = parse_run_command_path(&raw) else {
                return Ok(None);
            };
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
        .ok()
        .map_err(|e| anyhow!("RegOpenKeyExW(Run): {e}"))?;

        let name: Vec<u16> = encode_wide(RUN_VALUE_NAME);
        let value: Vec<u16> = encode_wide(quoted);
        let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
        let result = RegSetValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            0,
            REG_SZ,
            Some(bytes),
        )
        .ok()
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

fn parse_run_command_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        let path = &rest[..end];
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    } else {
        let exe_end = value
            .to_ascii_lowercase()
            .find(".exe")
            .map(|idx| idx + ".exe".len())
            .unwrap_or_else(|| value.find(char::is_whitespace).unwrap_or(value.len()));
        let path = value[..exe_end].trim();
        if path.is_empty() {
            None
        } else {
            Some(PathBuf::from(path))
        }
    }
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
    fn parse_ini_last_autostart_value_wins() {
        assert!(parse_ini_autostart("AutoStart=false\nAutoStart=true\n"));
        assert!(!parse_ini_autostart("AutoStart=true\nAutoStart=false\n"));
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
            reconcile_action(true, RunState::Missing),
            ReconcileAction::EnableRegistry
        );
        assert_eq!(
            reconcile_action(true, RunState::Other),
            ReconcileAction::EnableRegistry
        );
        assert_eq!(
            reconcile_action(false, RunState::CurrentExe),
            ReconcileAction::DisableRegistry
        );
        assert_eq!(
            reconcile_action(true, RunState::CurrentExe),
            ReconcileAction::Noop
        );
        assert_eq!(reconcile_action(false, RunState::Missing), ReconcileAction::Noop);
        assert_eq!(reconcile_action(false, RunState::Other), ReconcileAction::Noop);
    }

    #[test]
    fn quote_exe_path_wraps() {
        let p = PathBuf::from(r"C:\Program Files\Pin\pin.exe");
        assert_eq!(
            quote_exe_path(&p),
            r#""C:\Program Files\Pin\pin.exe""#
        );
    }

    #[test]
    fn parse_run_command_path_supports_quoted_path() {
        assert_eq!(
            parse_run_command_path(r#""C:\Program Files\Pin\pin.exe""#),
            Some(PathBuf::from(r"C:\Program Files\Pin\pin.exe"))
        );
    }

    #[test]
    fn parse_run_command_path_supports_quoted_path_with_args() {
        assert_eq!(
            parse_run_command_path(r#""C:\Program Files\Pin\pin.exe" --minimized"#),
            Some(PathBuf::from(r"C:\Program Files\Pin\pin.exe"))
        );
    }

    #[test]
    fn parse_run_command_path_supports_unquoted_exe_path() {
        assert_eq!(
            parse_run_command_path(r"C:\Tools\pin.exe --minimized"),
            Some(PathBuf::from(r"C:\Tools\pin.exe"))
        );
    }

    #[test]
    fn parse_run_command_path_rejects_empty_or_unclosed_quote() {
        assert_eq!(parse_run_command_path(""), None);
        assert_eq!(parse_run_command_path(r#""C:\Tools\pin.exe"#), None);
    }

    #[test]
    fn toggle_target_uses_desired_state() {
        assert!(!toggle_target(AutoStartState {
            desired: true,
            effective: false,
        }));
        assert!(toggle_target(AutoStartState {
            desired: false,
            effective: true,
        }));
    }
}
