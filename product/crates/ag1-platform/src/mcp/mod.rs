//! MCP intake, native discovery and manager-owned tool execution.
//!
//! Live transports must enforce their own aggregate budgets and deadlines before
//! using these parsers. Catalog validation is never permission to execute a tool.
pub mod auth;
pub mod catalog;
pub mod definitions;
pub mod discovery;
#[cfg(feature = "mcp-fixtures")]
pub mod fixtures;
pub mod http;
pub mod input;
#[cfg(target_os = "linux")]
pub mod manager;
#[cfg(windows)]
#[path = "manager_windows.rs"]
pub mod manager;
pub mod oauth;
pub(crate) mod operation;
pub mod registry;
#[cfg(target_os = "linux")]
pub mod service;
mod sse;
#[cfg(any(target_os = "linux", windows))]
pub mod stdio;
pub mod wire;

use ag1_core::DomainError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Revision {
    Legacy20251125,
    Current20260728,
}

impl Revision {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "2025-11-25" => Ok(Self::Legacy20251125),
            "2026-07-28" => Ok(Self::Current20260728),
            _ => Err(DomainError::Unavailable("MCP protocol revision".into())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Legacy20251125 => "2025-11-25",
            Self::Current20260728 => "2026-07-28",
        }
    }
}

fn invalid(reason: &str) -> DomainError {
    // Never interpolate server-controlled content into diagnostics.
    DomainError::InvalidInput(format!("MCP: {reason}"))
}

mod media;
