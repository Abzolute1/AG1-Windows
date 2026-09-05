//! Explicit platform capabilities. Linux is implemented; other OS capabilities remain unavailable.
pub mod diagnostics;
pub mod executor;
#[cfg(target_os = "linux")]
pub mod git;
pub mod ports;
#[cfg(target_os = "linux")]
pub mod process;
#[cfg(target_os = "linux")]
pub mod workspace;

pub fn native_available() -> bool {
    cfg!(target_os = "linux")
}

pub(crate) fn io_error(error: std::io::Error) -> ag1_core::DomainError {
    use ag1_core::DomainError;
    match error.kind() {
        std::io::ErrorKind::NotFound => DomainError::NotFound,
        std::io::ErrorKind::PermissionDenied => DomainError::PermissionDenied,
        std::io::ErrorKind::TimedOut => DomainError::TimedOut,
        _ => DomainError::Io(error.to_string()),
    }
}
