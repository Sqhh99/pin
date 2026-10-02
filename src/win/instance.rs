//! Single-instance guard backed by a named mutex.

use anyhow::{Context, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
use windows::Win32::System::Threading::CreateMutexW;

use crate::win::encode_wide;

/// Held for as long as this process should count as "the" running instance.
pub struct SingleInstance(HANDLE);

impl SingleInstance {
    /// Returns `Ok(None)` if another process already holds `name`.
    ///
    /// Use a `Local\` name: instances are per logon session, so a second
    /// user on the same machine (fast user switching, RDP) can run their own.
    pub fn acquire(name: &str) -> Result<Option<Self>> {
        let name_w = encode_wide(name);
        let handle = unsafe { CreateMutexW(None, false, PCWSTR(name_w.as_ptr())) }
            .with_context(|| format!("CreateMutexW({name})"))?;
        // CreateMutexW succeeds and sets ERROR_ALREADY_EXISTS when the mutex
        // was already there.
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = unsafe { CloseHandle(handle) };
            return Ok(None);
        }
        Ok(Some(Self(handle)))
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
