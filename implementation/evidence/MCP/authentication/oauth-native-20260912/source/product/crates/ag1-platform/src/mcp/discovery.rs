//! Shared discovery contracts and revision checks for native transports.
use super::{Revision, catalog::CatalogBinding, invalid};
use ag1_core::DomainError;
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct DiscoveryLimits {
    operation_timeout: Duration,
}
impl Default for DiscoveryLimits {
    fn default() -> Self {
        Self {
            operation_timeout: Duration::from_secs(15),
        }
    }
}
impl DiscoveryLimits {
    pub(crate) fn timeout(&self) -> Duration {
        self.operation_timeout
    }
    pub fn new(operation_timeout: Duration) -> Result<Self, DomainError> {
        if operation_timeout.is_zero() || operation_timeout > Duration::from_secs(30) {
            return Err(invalid("discovery operation timeout"));
        }
        Ok(Self { operation_timeout })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Attached,
    Negotiating,
    Ready,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogKind {
    Resources,
    ResourceTemplates,
    Prompts,
}
impl CatalogKind {
    pub(crate) fn method(self) -> &'static str {
        match self {
            Self::Resources => "resources/list",
            Self::ResourceTemplates => "resources/templates/list",
            Self::Prompts => "prompts/list",
        }
    }
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Resources => "resources",
            Self::ResourceTemplates => "resourceTemplates",
            Self::Prompts => "prompts",
        }
    }
    pub(crate) fn capability(self) -> &'static str {
        match self {
            Self::Resources | Self::ResourceTemplates => "resources",
            Self::Prompts => "prompts",
        }
    }
}

/// Untrusted metadata only. URI/prompt references never become AG1 artifact grants.
pub struct DiscoveryCatalog {
    pub binding: CatalogBinding,
    pub kind: CatalogKind,
    pub entries: Vec<Value>,
}

pub(crate) fn client_info() -> Value {
    json!({"name":"AG1", "version":env!("CARGO_PKG_VERSION")})
}
pub(crate) fn cursor_params(cursor: &Option<String>) -> Value {
    cursor
        .as_ref()
        .map(|c| json!({"cursor":c}))
        .unwrap_or_else(|| json!({}))
}
fn text(value: &Value, limit: usize) -> bool {
    value
        .as_str()
        .is_some_and(|s| !s.is_empty() && s.len() <= limit && !s.chars().any(char::is_control))
}
fn implementation(value: &Value) -> Result<(), DomainError> {
    if !text(&value["name"], 256) || !text(&value["version"], 256) {
        return Err(invalid("server implementation metadata"));
    }
    Ok(())
}
pub(crate) fn validate_entry(kind: CatalogKind, row: &Value) -> Result<(), DomainError> {
    if !text(&row["name"], 256)
        || row
            .get("description")
            .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 16 * 1024))
    {
        return Err(invalid("discovery entry metadata"));
    }
    match kind {
        CatalogKind::Resources if !text(&row["uri"], 8192) => {
            return Err(invalid("resource URI metadata"));
        }
        CatalogKind::ResourceTemplates if !text(&row["uriTemplate"], 8192) => {
            return Err(invalid("resource template metadata"));
        }
        CatalogKind::Prompts => {
            if let Some(args) = row.get("arguments") {
                let args = args
                    .as_array()
                    .filter(|a| a.len() <= 64)
                    .ok_or_else(|| invalid("prompt argument metadata"))?;
                let mut names = BTreeSet::new();
                for arg in args {
                    if !text(&arg["name"], 256)
                        || !names.insert(arg["name"].as_str().unwrap())
                        || arg.get("required").is_some_and(|v| !v.is_boolean())
                        || arg
                            .get("description")
                            .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 16 * 1024))
                    {
                        return Err(invalid("prompt argument metadata"));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_server(revision: Revision, result: &Value) -> Result<(), DomainError> {
    if !result["capabilities"].is_object() {
        return Err(invalid("server capabilities"));
    }
    for name in ["tools", "resources", "prompts"] {
        if result["capabilities"]
            .get(name)
            .is_some_and(|v| !v.is_object())
        {
            return Err(invalid("advertised capability shape"));
        }
    }
    match revision {
        Revision::Legacy20251125 => {
            if result["protocolVersion"] != revision.as_str() {
                return Err(invalid("negotiated revision mismatch"));
            }
            implementation(&result["serverInfo"])?;
        }
        Revision::Current20260728 => {
            let versions = result["supportedVersions"]
                .as_array()
                .ok_or_else(|| invalid("discovery versions"))?;
            if versions.is_empty()
                || versions.len() > 16
                || versions.iter().any(|v| !v.is_string())
                || !versions.iter().any(|v| v == revision.as_str())
                || result["resultType"] != "complete"
                || result["ttlMs"].as_u64().is_none()
                || !matches!(result["cacheScope"].as_str(), Some("public" | "private"))
            {
                return Err(invalid("unsupported or malformed discovery response"));
            }
            if let Some(info) = result
                .get("_meta")
                .and_then(|m| m.get("io.modelcontextprotocol/serverInfo"))
            {
                implementation(info)?;
            }
        }
    }
    Ok(())
}
