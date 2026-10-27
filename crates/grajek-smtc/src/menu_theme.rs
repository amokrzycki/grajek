//! Windows native popup-menu theme integration, independent of GPUI's palette.
//! Windows provides these uxtheme entry points by undocumented ordinals, so
//! version and export checks are mandatory; failure leaves the native light menu.
#![allow(
    unsafe_code,
    reason = "Win32 uxtheme exports and OS version detection require a small FFI boundary"
)]

use std::sync::OnceLock;
use windows::{
    Win32::{
        Foundation::HMODULE,
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            SystemInformation::OSVERSIONINFOW,
        },
    },
    core::{PCSTR, w},
};

#[link(name = "ntdll")]
unsafe extern "system" {
    #[link_name = "RtlGetVersion"]
    fn rtl_get_version(version: *mut OSVERSIONINFOW) -> i32;
}

fn supports_build(major: u32, minor: u32, build: u32) -> bool {
    // Ordinal 135 was AllowDarkModeForApp(bool) before Windows 10 1903.
    // Treating it as SetPreferredAppMode(int) on 1809 is ABI-incorrect.
    major == 10 && minor == 0 && build >= 18362
}

fn supported_windows() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let mut version = OSVERSIONINFOW {
            dwOSVersionInfoSize: u32::try_from(std::mem::size_of::<OSVERSIONINFOW>())
                .unwrap_or_default(),
            ..Default::default()
        };
        // SAFETY: valid initialized structure, and RtlGetVersion only writes to it.
        let status = unsafe { rtl_get_version(&raw mut version) };
        status >= 0
            && supports_build(
                version.dwMajorVersion,
                version.dwMinorVersion,
                version.dwBuildNumber,
            )
    })
}

fn export(ordinal: u16) -> Option<unsafe extern "system" fn() -> isize> {
    if !supported_windows() {
        return None;
    }
    // Retain the system DLL for process lifetime; cached raw HMODULE is never freed.
    static LIBRARY: OnceLock<Option<isize>> = OnceLock::new();
    let address = (*LIBRARY.get_or_init(|| {
        // SAFETY: a fixed system DLL name; module lifetime is process-wide.
        unsafe { LoadLibraryW(w!("uxtheme.dll")).ok().map(|module| module.0 as isize) }
    }))?;
    let module = HMODULE(address as *mut _);
    // SAFETY: the low-word pseudo-pointer is the documented Win32 ordinal form,
    // not a string to dereference. A missing export is handled as None.
    unsafe { GetProcAddress(module, PCSTR::from_raw(usize::from(ordinal) as *const u8)) }
}

pub(super) fn allow_dark_menus() -> bool {
    let Some(address) = export(135) else {
        return false;
    };
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    // SAFETY: since Windows 10 build 18362, ordinal 135 has this exact ABI.
    let set_mode: SetPreferredAppMode = unsafe { std::mem::transmute(address) };
    // AllowDark asks the OS to choose according to the Windows apps theme,
    // unlike ForceDark, which could override a light or high-contrast preference.
    unsafe { set_mode(1) };
    if let Some(address) = export(136) {
        // SAFETY: Win32 uxtheme ordinal 136 is FlushMenuThemes(void).
        let flush: unsafe extern "system" fn() = unsafe { std::mem::transmute(address) };
        unsafe { flush() };
    }
    true
}

pub(super) fn apps_use_dark_mode() -> Option<bool> {
    let address = export(132)?;
    // SAFETY: Win32 uxtheme ordinal 132 is ShouldAppsUseDarkMode(void).
    let query: unsafe extern "system" fn() -> bool =
        unsafe { std::mem::transmute(address) };
    Some(unsafe { query() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_app_mode_requires_the_correct_ordinal_135_abi() {
        assert!(!supports_build(6, 1, 7601));
        assert!(!supports_build(10, 0, 17763));
        assert!(!supports_build(10, 0, 18361));
        assert!(supports_build(10, 0, 18362));
        assert!(supports_build(10, 0, 22631));
    }
}
