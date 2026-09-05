//! Explicit platform capabilities. Linux is implemented; other OS capabilities remain unavailable.
pub mod diagnostics;
pub mod executor;
#[cfg(target_os = "linux")]
pub mod git;
pub mod llama;
pub mod ports;
#[cfg(target_os = "linux")]
pub mod process;
pub mod provider;
pub mod web_http;
#[cfg(target_os = "linux")]
pub mod workspace;
#[cfg(target_os = "linux")]
pub mod workspace_lifecycle;

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

#[cfg(target_os = "linux")]
pub mod source;

#[cfg(target_os = "linux")]
pub mod retrieval;

#[cfg(target_os = "linux")]
pub mod file_tools;
pub mod image_tools;

#[cfg(target_os = "linux")]
pub mod search_tools;

#[cfg(target_os = "linux")]
pub mod git_tools;

#[cfg(target_os = "linux")]
pub mod shell_tools;

#[cfg(target_os = "linux")]
pub mod extension_custom_models;
#[cfg(target_os = "linux")]
pub mod extension_host;
#[cfg(target_os = "linux")]
pub mod extension_inference;
#[cfg(target_os = "linux")]
pub mod extension_models;
#[cfg(target_os = "linux")]
mod extension_partial_json;
#[cfg(target_os = "linux")]
pub mod extension_providers;
#[cfg(target_os = "linux")]
pub mod extension_tools;
#[cfg(target_os = "linux")]
pub mod job_tools;
#[cfg(target_os = "linux")]
pub mod package_execution;
#[cfg(target_os = "linux")]
pub mod package_queries;
#[cfg(target_os = "linux")]
pub mod package_query_git;
#[cfg(target_os = "linux")]
pub mod package_settings;
#[cfg(target_os = "linux")]
pub mod resource_fs;
#[cfg(target_os = "linux")]
pub mod resource_overrides;
#[cfg(target_os = "linux")]
pub mod resource_patterns;
#[cfg(target_os = "linux")]
pub mod resources;
#[cfg(target_os = "linux")]
pub mod skill_creator;
pub mod web_tools;

#[cfg(target_os = "linux")]
pub mod target_tools;

pub mod extension_auth;

pub mod extension_auth_bridge;

pub mod extension_login;
