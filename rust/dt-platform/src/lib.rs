//! Windows boundary for `glowtube.py` input and `service.py` process operations.
//! No hooks, clipboard reads or process mutations occur at construction/import.
//! Public signatures use std/core values so future platforms can implement the same contracts.

pub mod input;
pub mod process;

// Pointer/handle operations stay here; safe callers never receive raw pointers.
#[cfg(windows)]
#[allow(unsafe_code)]
mod win32;

#[cfg(not(windows))]
fn unsupported<T>() -> dt_core::Result<T> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Windows platform operation",
    )
    .into())
}
