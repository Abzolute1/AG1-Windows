//! Project views are independent of the presentation and process owner.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Web,
    Native,
    Rust,
    Tauri,
    Godot,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Idle,
    Starting,
    Ready,
    Exited,
    Failed,
    Stopped,
    RecoveryRequired,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Availability {
    pub available: bool,
    pub reason: String,
}
impl Availability {
    pub fn yes() -> Self {
        Self {
            available: true,
            reason: String::new(),
        }
    }
    pub fn no(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: reason.into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub kind: Kind,
    pub workspace: String,
    pub manifests: Vec<String>,
    pub toolchain: Vec<String>,
    pub capabilities: BTreeMap<String, Availability>,
    pub lifecycle: Lifecycle,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub session: String,
    pub targets: Vec<Target>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub source: String,
    pub severity: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub code: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    pub id: String,
    pub target: String,
    pub action: String,
    pub commands: Vec<Vec<String>>,
    pub source_revision: String,
    pub lifecycle: Lifecycle,
    pub exits: Vec<Option<i32>>,
    pub logs: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub elapsed_ms: u64,
    pub ready_ms: Option<u64>,
    pub first_log_ms: Option<u64>,
    pub stop_ms: Option<u64>,
    pub target_rss_bytes: u64,
    pub browser_rss_bytes: u64,
    pub display_rss_bytes: u64,
}

/// Unsupported runners expose a concrete capability response without compiling
/// Linux ownership code on another OS.
pub fn platform_capability(os: &str) -> Availability {
    if os == "linux" {
        Availability::yes()
    } else {
        Availability::no(format!(
            "Native project supervision is not implemented for {os}; use a supported Linux host or explicitly configured external target. No tuned llama backend substitution is supported."
        ))
    }
}
