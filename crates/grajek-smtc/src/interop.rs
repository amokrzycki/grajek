//! The only unsafe boundary: WinRT apartment balance and the desktop HWND factory.
#![allow(
    unsafe_code,
    reason = "Windows exposes apartment initialization and HWND interop only as unsafe FFI"
)]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Media::SystemMediaTransportControls,
    Win32::{
        Foundation::{E_HANDLE, HWND},
        System::WinRT::{
            ISystemMediaTransportControlsInterop, RO_INIT_SINGLETHREADED, RoInitialize,
            RoUninitialize,
        },
    },
    core::{Error, Result, factory},
};

pub(super) struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    pub(super) fn new() -> Result<Self> {
        // SAFETY: this initializes the calling UI thread's STA (GPUI already uses OLE STA).
        // Success, including S_FALSE, owns exactly one matching uninitialization below.
        unsafe {
            RoInitialize(RO_INIT_SINGLETHREADED)?;
        }
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: !Send keeps this on the initializing thread; all owned COM objects drop first.
        unsafe {
            RoUninitialize();
        }
    }
}

pub(super) fn for_window(window: &impl HasWindowHandle) -> Result<SystemMediaTransportControls> {
    let handle = window
        .window_handle()
        .map_err(|_| Error::from_hresult(E_HANDLE))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err(Error::from_hresult(E_HANDLE));
    };
    let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    // SAFETY: HasWindowHandle borrows a live window for this call. GPUI supplies its own
    // top-level process window. GetForWindow validates ownership; we never retain the HWND.
    // HWND is a transparent wrapper over a pointer; the cast is safe.
    unsafe { interop.GetForWindow(HWND(handle.hwnd.get() as *mut _)) }
}

pub(super) fn set_window_visible(window: &impl HasWindowHandle, visible: bool) -> Result<()> {
    use windows::Win32::{
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            GetWindowThreadProcessId, IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow,
            ShowWindowAsync,
        },
    };
    let handle = window
        .window_handle()
        .map_err(|_| Error::from_hresult(E_HANDLE))?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return Err(Error::from_hresult(E_HANDLE));
    };
    let hwnd = HWND(handle.hwnd.get() as *mut _);
    // SAFETY: the handle is borrowed for this call, never retained. Reject calls from
    // other threads before queuing ShowWindowAsync on the owning message loop.
    unsafe {
        if GetWindowThreadProcessId(hwnd, None) != GetCurrentThreadId() {
            return Err(Error::from_hresult(E_HANDLE));
        }
        if visible {
            ShowWindowAsync(
                hwnd,
                if IsIconic(hwnd).as_bool() {
                    SW_RESTORE
                } else {
                    SW_SHOW
                },
            )
            .ok()?;
            // Foreground permission is OS-controlled; visibility does not depend on it.
            let _ = SetForegroundWindow(hwnd);
        } else {
            ShowWindowAsync(hwnd, SW_HIDE).ok()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod visibility_tests {
    use super::*;
    use windows::{
        Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, IsIconic, IsWindowVisible, MSG,
            PM_REMOVE, PeekMessageW, SW_MINIMIZE, ShowWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
            WS_VISIBLE,
        },
        core::w,
    };

    struct TestWindow(HWND);
    impl HasWindowHandle for TestWindow {
        fn window_handle(
            &self,
        ) -> std::result::Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError>
        {
            let hwnd = std::num::NonZeroIsize::new(self.0.0 as isize)
                .ok_or(raw_window_handle::HandleError::Unavailable)?;
            // SAFETY: TestWindow owns this live HWND until its Drop, on this thread.
            Ok(unsafe {
                raw_window_handle::WindowHandle::borrow_raw(
                    raw_window_handle::Win32WindowHandle::new(hwnd).into(),
                )
            })
        }
    }
    impl Drop for TestWindow {
        fn drop(&mut self) {
            // SAFETY: sole owner, destruction on the creating thread.
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    fn dispatch() {
        // SAFETY: dispatch only this test thread's native message queue.
        unsafe {
            let mut message = MSG::default();
            while PeekMessageW(&raw mut message, None, 0, 0, PM_REMOVE).as_bool() {
                DispatchMessageW(&message);
            }
        }
    }

    #[test]
    #[ignore = "requires an interactive native Windows desktop; run with --ignored"]
    fn native_hide_restore_and_minimize_preserve_the_hwnd() -> Result<()> {
        // SAFETY: built-in STATIC class, no callback or borrowed creation data.
        let window = TestWindow(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Grajek visibility test"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                0,
                0,
                200,
                100,
                None,
                None,
                None,
                None,
            )?
        });
        for _ in 0..3 {
            set_window_visible(&window, false)?;
            dispatch();
            // SAFETY: the owned HWND is live on its creating thread.
            unsafe {
                assert!(!IsWindowVisible(window.0).as_bool());
                assert!(!IsIconic(window.0).as_bool());
            }
            set_window_visible(&window, true)?;
            set_window_visible(&window, true)?;
            dispatch();
            // SAFETY: same live HWND; minimization is deliberately separate from Hide.
            unsafe {
                assert!(IsWindowVisible(window.0).as_bool());
                let _ = ShowWindow(window.0, SW_MINIMIZE);
                assert!(IsIconic(window.0).as_bool());
            }
            set_window_visible(&window, true)?;
            dispatch();
            // SAFETY: same live HWND after restoration.
            unsafe {
                assert!(IsWindowVisible(window.0).as_bool());
                assert!(!IsIconic(window.0).as_bool());
            }
        }
        Ok(())
    }
}
