//! Windows Event Log receiver.
//!
//! `config`, `xml`, and `bookmark` are plain data/string logic with no
//! Win32 dependency, so they build and are unit-tested on every platform.
//! The actual `EvtSubscribe` wiring lives in `windows_impl`, gated behind
//! `cfg(windows)` -- it only compiles (and only links against
//! `wevtapi.dll`) on Windows, and per the crate-level docs there, it has
//! only been type-checked (`cargo check --target x86_64-pc-windows-msvc`),
//! never run: it needs a real Windows host or CI to be verified.

pub mod bookmark;
pub mod config;
pub mod xml;

#[cfg(windows)]
mod windows_impl;

pub use config::{StartAt, WinEventLogConfig};

#[cfg(windows)]
pub use windows_impl::WinEventLogReceiver;
