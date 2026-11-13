//! Target-specific desktop composition boundary.

#[cfg(windows)]
#[path = "lib_windows.rs"]
mod platform;
#[cfg(not(windows))]
#[path = "lib_linux.rs"]
mod platform;

pub use platform::*;
