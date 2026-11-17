//! Windows boundary for `glowtube.py` input and `service.py` process operations.
//! Phase-two stubs; no hooks, clipboard reads or process mutations occur at construction/import.
//! Public signatures use std/core values so future platforms can implement the same contracts.

pub mod input;
pub mod process;
