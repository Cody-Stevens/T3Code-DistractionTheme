//! Reviewed FFI boundary. Handles either use std RAII or local resource guards.

mod input;
mod process;
mod security;
pub(crate) use input::*;
pub(crate) use process::*;
pub(crate) use security::*;

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use windows::Win32::Foundation::HANDLE;

fn error(error: windows::core::Error) -> dt_core::Error {
    std::io::Error::from_raw_os_error(error.code().0 & 0xffff).into()
}

fn last_error() -> dt_core::Error {
    std::io::Error::last_os_error().into()
}

fn raw(handle: &impl AsRawHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

fn owned(handle: HANDLE) -> OwnedHandle {
    // SAFETY: callers pass a successful create/open result, transferring unique ownership.
    unsafe { OwnedHandle::from_raw_handle(handle.0) }
}
