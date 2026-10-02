//! Minimal RAII wrapper over the registry calls autostart needs.

use anyhow::{anyhow, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};

use crate::win::encode_wide;

/// An open registry key, closed on drop.
pub struct RegKey(HKEY);

impl RegKey {
    /// Open an existing subkey of `HKEY_CURRENT_USER`.
    pub fn open_current_user(subkey: &str, access: REG_SAM_FLAGS) -> Result<Self> {
        let subkey_w = encode_wide(subkey);
        let mut hkey = HKEY::default();
        unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey_w.as_ptr()),
                0,
                access,
                &mut hkey,
            )
        }
        .ok()
        .map_err(|e| anyhow!("RegOpenKeyExW(HKCU\\{subkey}): {e}"))?;
        Ok(Self(hkey))
    }

    /// Read a `REG_SZ` / `REG_EXPAND_SZ` value. `Ok(None)` if it doesn't exist.
    pub fn get_string(&self, name: &str) -> Result<Option<String>> {
        let name_w = encode_wide(name);
        let name_p = PCWSTR(name_w.as_ptr());
        // The value can change between the size query and the read, so retry.
        for _ in 0..3 {
            let mut kind = REG_VALUE_TYPE::default();
            let mut cb = 0u32;
            let err = unsafe {
                RegQueryValueExW(self.0, name_p, None, Some(&mut kind), None, Some(&mut cb))
            };
            if err == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check(err, name)?;
            if kind != REG_SZ && kind != REG_EXPAND_SZ {
                return Err(anyhow!(
                    "registry value {name} has type {}, expected a string",
                    kind.0
                ));
            }

            let mut buf = vec![0u16; (cb as usize).div_ceil(2) + 1];
            let mut cb_read = (buf.len() * 2) as u32;
            let err = unsafe {
                RegQueryValueExW(
                    self.0,
                    name_p,
                    None,
                    None,
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut cb_read),
                )
            };
            if err == windows::Win32::Foundation::ERROR_MORE_DATA {
                continue;
            }
            check(err, name)?;
            buf.truncate(cb_read as usize / 2);
            while buf.last() == Some(&0) {
                buf.pop();
            }
            return Ok(Some(String::from_utf16_lossy(&buf)));
        }
        Err(anyhow!("registry value {name} kept changing size"))
    }

    pub fn set_string(&self, name: &str, value: &str) -> Result<()> {
        let name_w = encode_wide(name);
        let value_w = encode_wide(value);
        let bytes: Vec<u8> = value_w.iter().flat_map(|c| c.to_le_bytes()).collect();
        let err =
            unsafe { RegSetValueExW(self.0, PCWSTR(name_w.as_ptr()), 0, REG_SZ, Some(&bytes)) };
        check(err, name)
    }

    /// Delete a value; succeeds if it is already absent.
    pub fn delete_value(&self, name: &str) -> Result<()> {
        let name_w = encode_wide(name);
        let err = unsafe { RegDeleteValueW(self.0, PCWSTR(name_w.as_ptr())) };
        if err == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(err, name)
    }
}

impl Drop for RegKey {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn check(err: WIN32_ERROR, name: &str) -> Result<()> {
    err.ok().map_err(|e| anyhow!("registry value {name}: {e}"))
}
