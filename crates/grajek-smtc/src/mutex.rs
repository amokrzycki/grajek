//! An existence marker held until process shutdown, including while hidden in the tray.
#![allow(
    unsafe_code,
    reason = "Windows mutex ownership and handle release require FFI"
)]

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::Threading::CreateMutexW,
    },
    core::{PCWSTR, w},
};

/// Owns exactly one reference to the installer-visible named mutex.
pub struct AppMutexGuard(HANDLE);

fn hold_mutex(name: PCWSTR) -> Option<AppMutexGuard> {
    // SAFETY: no security attributes are borrowed; name is live and NUL-terminated.
    // We do not request mutex ownership, only a reference keeping the object alive.
    unsafe { CreateMutexW(None, false, name) }
        .ok()
        .map(AppMutexGuard)
}

/// Inno Setup's AppMutex blocks install/uninstall while any Grajek instance exists.
pub fn hold_app_mutex() -> Option<AppMutexGuard> {
    hold_mutex(w!("Local\\org.grajek.desktop"))
}

impl Drop for AppMutexGuard {
    fn drop(&mut self) {
        // SAFETY: this guard owns the handle returned by CreateMutexW and closes it once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::System::Threading::{MUTEX_MODIFY_STATE, OpenMutexW},
        core::Result,
    };

    #[test]
    fn installer_marker_lives_until_the_last_instance_drops() -> Result<()> {
        let name: Vec<u16> = format!("Local\\org.grajek.desktop.test.{}\0", std::process::id())
            .encode_utf16()
            .collect();
        let name = PCWSTR(name.as_ptr());
        let first = hold_mutex(name).ok_or_else(windows::core::Error::from_thread)?;
        let second = hold_mutex(name).ok_or_else(windows::core::Error::from_thread)?;
        drop(first);
        // SAFETY: name's allocation remains live; this independently opened handle is closed below.
        let opened = unsafe { OpenMutexW(MUTEX_MODIFY_STATE, false, name) }?;
        // SAFETY: opened owns a valid handle returned by OpenMutexW.
        unsafe { CloseHandle(opened) }?;
        drop(second);
        // SAFETY: name remains live; no process in this test retains an object reference.
        assert!(unsafe { OpenMutexW(MUTEX_MODIFY_STATE, false, name) }.is_err());
        Ok(())
    }
}
