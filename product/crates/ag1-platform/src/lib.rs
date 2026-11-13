//! Explicit platform capabilities. Linux is implemented; other OS capabilities remain unavailable.
#[cfg(any(target_os = "linux", windows))]
pub mod change_receipts;
#[cfg(any(target_os = "linux", windows))]
pub mod change_undo;
pub mod diagnostics;
pub mod executor;
#[cfg(target_os = "linux")]
pub mod git;
#[cfg(target_os = "linux")]
pub mod github;
pub mod llama;
#[cfg(any(target_os = "linux", windows))]
pub mod mcp;
pub mod ports;
#[cfg(target_os = "linux")]
pub mod process;
#[cfg(windows)]
#[path = "process_windows.rs"]
pub mod process;
pub mod provider;
#[cfg(target_os = "linux")]
pub mod session;
#[cfg(windows)]
#[path = "session_windows.rs"]
pub mod session;
pub mod web_http;
#[cfg(windows)]
#[path = "client/website_icons.rs"]
pub mod website_icons;
#[cfg(target_os = "linux")]
pub mod workspace;
#[cfg(windows)]
#[path = "workspace_windows.rs"]
pub mod workspace;
#[cfg(target_os = "linux")]
pub mod workspace_lifecycle;

pub fn native_available() -> bool {
    // Windows now has a real local session, workspace and process path. This
    // flag describes native capability, not whether every Linux feature has
    // reached parity yet.
    cfg!(any(target_os = "linux", windows))
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

pub mod source;

#[cfg(target_os = "linux")]
pub mod retrieval;

#[cfg(any(target_os = "linux", windows))]
pub mod file_tools;
#[cfg(any(target_os = "linux", windows))]
pub mod folder_access;
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
#[cfg(any(target_os = "linux", windows))]
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

#[cfg(target_os = "linux")]
pub mod extension_auth;

#[cfg(target_os = "linux")]
pub mod extension_auth_bridge;

#[cfg(target_os = "linux")]
pub mod extension_login;

#[cfg(target_os = "linux")]
pub mod artifact_tools;
#[cfg(target_os = "linux")]
pub mod memory_tools;
#[cfg(target_os = "linux")]
pub mod repo_tools;
#[cfg(target_os = "linux")]
pub mod task_tools;

#[cfg(target_os = "linux")]
pub mod source_tools;

#[cfg(target_os = "linux")]
pub mod todo_tools;

#[cfg(target_os = "linux")]
pub mod observation_tools;

#[cfg(target_os = "linux")]
pub mod episode_tools;

#[cfg(target_os = "linux")]
pub mod projects;

#[cfg(target_os = "linux")]
pub mod client;

#[cfg(target_os = "linux")]
pub mod cutover;
#[cfg(windows)]
#[path = "cutover_windows.rs"]
pub mod cutover;
#[cfg(windows)]
#[path = "local_runtime_windows.rs"]
pub mod local_runtime;
