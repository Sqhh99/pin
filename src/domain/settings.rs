//! User settings (`pin.ini`) and the pure parts of autostart reconciliation.
//!
//! File and registry I/O live in `crate::autostart`; everything here is
//! string/enum logic so it can be tested on any host.

use std::path::{Path, PathBuf};

pub const INI_FILE_NAME: &str = "pin.ini";
const INI_KEY_AUTOSTART: &str = "AutoStart";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// User intent for "Start with Windows".
    pub autostart: bool,
}

impl Settings {
    /// Parse ini text. Unknown keys and malformed lines are ignored; when a
    /// key appears more than once, the last value wins.
    pub fn parse(text: &str) -> Self {
        let mut settings = Settings::default();
        for line in text.lines() {
            let line = line.split(['#', ';']).next().unwrap_or(line).trim();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case(INI_KEY_AUTOSTART) {
                settings.autostart = parse_bool(value.trim());
            }
        }
        settings
    }

    pub fn to_ini(self) -> String {
        format!("{INI_KEY_AUTOSTART}={}\r\n", self.autostart)
    }
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

/// What the HKCU `Run` value currently points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Missing,
    CurrentExe,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileAction {
    Noop,
    EnableRegistry,
    DisableRegistry,
}

/// Bring the `Run` value in line with the ini (user intent). A `Run` value
/// pointing at a *different* exe is left alone when autostart is off — it
/// may belong to another install.
pub fn reconcile_action(desired: bool, run_state: RunState) -> ReconcileAction {
    match (desired, run_state) {
        (true, RunState::CurrentExe) | (false, RunState::Missing | RunState::Other) => {
            ReconcileAction::Noop
        }
        (true, RunState::Missing | RunState::Other) => ReconcileAction::EnableRegistry,
        (false, RunState::CurrentExe) => ReconcileAction::DisableRegistry,
    }
}

/// Command line stored in the `Run` value for `exe`.
pub fn quote_exe_path(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

/// Extract the executable path from a `Run` command line, quoted or not.
pub fn parse_run_command_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    let path = if let Some(rest) = value.strip_prefix('"') {
        &rest[..rest.find('"')?]
    } else {
        let exe_end = value
            .to_ascii_lowercase()
            .find(".exe")
            .map(|idx| idx + ".exe".len())
            .unwrap_or_else(|| value.find(char::is_whitespace).unwrap_or(value.len()));
        value[..exe_end].trim()
    };
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn autostart(text: &str) -> bool {
        Settings::parse(text).autostart
    }

    #[test]
    fn parse_true_variants() {
        assert!(autostart("AutoStart=true\n"));
        assert!(autostart("autostart=1"));
        assert!(autostart("AutoStart = yes"));
        assert!(autostart("AutoStart=ON\r\n"));
    }

    #[test]
    fn parse_false_or_missing() {
        assert!(!autostart("AutoStart=false\n"));
        assert!(!autostart(""));
        assert!(!autostart("# comment\n"));
        assert!(!autostart("; AutoStart=true\n"));
        assert!(!autostart("garbage line\n[Section]\n"));
    }

    #[test]
    fn parse_strips_trailing_comments() {
        assert!(autostart("AutoStart=true # enabled by installer"));
    }

    #[test]
    fn last_value_wins() {
        assert!(autostart("AutoStart=false\nAutoStart=true\n"));
        assert!(!autostart("AutoStart=true\nAutoStart=false\n"));
    }

    #[test]
    fn ini_roundtrip() {
        for on in [true, false] {
            let s = Settings { autostart: on };
            assert_eq!(Settings::parse(&s.to_ini()), s);
        }
        assert_eq!(Settings { autostart: true }.to_ini(), "AutoStart=true\r\n");
    }

    #[test]
    fn reconcile_action_matrix() {
        use ReconcileAction::*;
        use RunState::*;
        assert_eq!(reconcile_action(true, Missing), EnableRegistry);
        assert_eq!(reconcile_action(true, Other), EnableRegistry);
        assert_eq!(reconcile_action(true, CurrentExe), Noop);
        assert_eq!(reconcile_action(false, CurrentExe), DisableRegistry);
        assert_eq!(reconcile_action(false, Missing), Noop);
        assert_eq!(reconcile_action(false, Other), Noop);
    }

    #[test]
    fn quote_exe_path_wraps() {
        let p = PathBuf::from(r"C:\Program Files\Pin\pin.exe");
        assert_eq!(quote_exe_path(&p), r#""C:\Program Files\Pin\pin.exe""#);
    }

    #[test]
    fn run_command_quoted_path() {
        assert_eq!(
            parse_run_command_path(r#""C:\Program Files\Pin\pin.exe""#),
            Some(PathBuf::from(r"C:\Program Files\Pin\pin.exe"))
        );
    }

    #[test]
    fn run_command_quoted_path_with_args() {
        assert_eq!(
            parse_run_command_path(r#""C:\Program Files\Pin\pin.exe" --minimized"#),
            Some(PathBuf::from(r"C:\Program Files\Pin\pin.exe"))
        );
    }

    #[test]
    fn run_command_unquoted_exe_path() {
        assert_eq!(
            parse_run_command_path(r"C:\Tools\pin.exe --minimized"),
            Some(PathBuf::from(r"C:\Tools\pin.exe"))
        );
    }

    #[test]
    fn run_command_rejects_empty_or_unclosed_quote() {
        assert_eq!(parse_run_command_path(""), None);
        assert_eq!(parse_run_command_path("   "), None);
        assert_eq!(parse_run_command_path(r#""""#), None);
        assert_eq!(parse_run_command_path(r#""C:\Tools\pin.exe"#), None);
    }
}
