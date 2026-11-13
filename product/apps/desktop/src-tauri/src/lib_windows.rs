//! Windows desktop library surface.
//!
//! The Windows executable owns the native local-session host directly. Keeping
//! this library limited to shared data avoids accidentally linking the Linux
//! TypeScript worker composition into AG1W.

#[path = "themes.rs"]
pub mod themes;

#[path = "native_host_windows.rs"]
pub mod native_host;
