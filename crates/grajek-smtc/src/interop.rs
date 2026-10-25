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
