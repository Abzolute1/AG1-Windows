//! Public configuration only. Import is bounded, inert and never reads a path,
//! resolves a command/credential, expands a variable or contacts an endpoint.
use super::{
    Revision,
    catalog::CatalogScope,
    http::Endpoint,
    invalid,
    wire::{Limits, decode_json},
};
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const MAX_SERVERS: usize = 128;
pub const MAX_DEFINITION_BYTES: usize = 16 * 1024;
/// Actual local process access, independent of workspace availability/tool grants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LocalAccess {
    #[default]
    HostAccount,
    ReadOnlyDirectory,
}
impl LocalAccess {
    pub(crate) fn is_host_account(&self) -> bool {
        *self == Self::HostAccount
    }
    pub(crate) fn validate(self, configuration: &Configuration) -> Result<(), DomainError> {
        if self == Self::HostAccount {
            return Ok(());
        }
        let Configuration::Local { cwd, .. } = configuration else {
            return Err(invalid("local access requires a local server"));
        };
        let path = Path::new(cwd);
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        }) || path.parent().is_none()
            || [
                "/usr", "/lib", "/lib64", "/bin", "/sbin", "/proc", "/dev", "/run", "/ag1", "/etc",
            ]
            .iter()
            .any(|reserved| path.starts_with(reserved))
        {
            return Err(invalid(
                "read-only MCP directory must be an explicit directory outside reserved system roots",
            ));
        }
        Ok(())
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::HostAccount => {
                "Host-account code execution. Dependencies are mutable; workspace availability and tool rules do not provide OS containment."
            }
            Self::ReadOnlyDirectory => {
                "Read-only access to the reviewed directory and system runtime files, with networking disabled. Other user directories are hidden. Dependencies within the allowed paths remain mutable. Runtimes requiring writes or sockets are unsupported in this mode; AG1 never silently falls back to host-account access."
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "camelCase", deny_unknown_fields)]
pub enum Configuration {
    Local {
        executable: String,
        arguments: Vec<String>,
        cwd: String,
        public_environment: BTreeMap<String, String>,
        /// Names only. Private entry/resolution remains a separate native flow.
        secret_environment: BTreeSet<String>,
    },
    Http {
        url: String,
        loopback_development: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub enum Provenance {
    Manual,
    Registry {
        registry: String,
        name: String,
        version: String,
        sha256: String,
        retrieved_at: u64,
    },
    Project {
        path: String,
        sha256: String,
        entry: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub configuration: Configuration,
    pub protocol: String,
    /// Opaque MCP profile identity only; no credential value. Profiles are
    /// inspectable but cannot connect until MCP authentication is implemented.
    pub auth_profile: Option<String>,
    pub provenance: Provenance,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Availability {
    pub enabled: bool,
    pub all_workspaces: bool,
    pub workspaces: BTreeSet<String>,
    pub general_chat: bool,
    pub denied_workspaces: BTreeSet<String>,
}
impl Availability {
    pub fn allows(&self, scope: &CatalogScope) -> bool {
        self.enabled
            && match scope {
                CatalogScope::GeneralChat => self.general_chat,
                CatalogScope::Workspace(id) => {
                    !self.denied_workspaces.contains(id)
                        && (self.all_workspaces || self.workspaces.contains(id))
                }
            }
    }
    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        if self.workspaces.len() > 128 || self.denied_workspaces.len() > 128 {
            return Err(invalid("workspace grant count"));
        }
        for id in self.workspaces.iter().chain(&self.denied_workspaces) {
            validate_scope(&CatalogScope::Workspace(id.clone()))?;
        }
        Ok(())
    }
}
pub(crate) fn label(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub(crate) fn validate_scope(scope: &CatalogScope) -> Result<(), DomainError> {
    if matches!(scope, CatalogScope::Workspace(id) if !label(id, 128)) {
        return Err(invalid("canonical workspace identity"));
    }
    Ok(())
}
fn env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
impl Draft {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !label(&self.name, 256) || self.auth_profile.as_ref().is_some_and(|s| !label(s, 128)) {
            return Err(invalid("definition label/auth profile"));
        }
        Revision::parse(&self.protocol)?;
        match &self.configuration {
            Configuration::Local {
                executable,
                arguments,
                cwd,
                public_environment,
                secret_environment,
            } => {
                if !label(executable, 4096)
                    || !Path::new(executable).is_absolute()
                    || !label(cwd, 4096)
                    || !Path::new(cwd).is_absolute()
                    || arguments.len() > 64
                    || arguments.iter().any(|a| a.len() > 4096 || a.contains('\0'))
                    || public_environment.len() + secret_environment.len() > 32
                    || public_environment
                        .iter()
                        .any(|(k, v)| !env_name(k) || v.len() > 4096 || v.contains('\0'))
                    || secret_environment
                        .iter()
                        .any(|k| !env_name(k) || public_environment.contains_key(k))
                {
                    return Err(invalid("local configuration bounds/explicit paths"));
                }
            }
            Configuration::Http {
                url,
                loopback_development,
            } => {
                if *loopback_development {
                    Endpoint::loopback_development(url)?;
                } else {
                    Endpoint::public_https(url)?;
                }
            }
        }
        if let Provenance::Project {
            path,
            sha256,
            entry,
        } = &self.provenance
            && (!label(path, 4096)
                || !label(entry, 256)
                || sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        {
            return Err(invalid("project provenance"));
        }
        if let Provenance::Registry {
            registry,
            name,
            version,
            sha256,
            retrieved_at,
        } = &self.provenance
        {
            if registry != "https://api.mcp.github.com"
                || !label(name, 256)
                || !label(version, 256)
                || *retrieved_at == 0
                || sha256.len() != 64
                || !sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err(invalid("registry provenance"));
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| invalid("definition encoding"))?
            .len()
            > MAX_DEFINITION_BYTES
        {
            return Err(DomainError::LimitExceeded("MCP definition bytes".into()));
        }
        Ok(())
    }
}

/// Documented minimal format: {"mcpServers":{"name":{"command":absolute,
/// "args":[...],"cwd":absolute?}}}. Unknown fields, environment values and remote
/// imports are refused rather than silently discarded. The native caller supplies
/// the canonical project directory; this function never opens it.
pub fn import_project(
    bytes: &[u8],
    source_path: &str,
    project_directory: &str,
) -> Result<Vec<Draft>, DomainError> {
    if !label(source_path, 4096) || !Path::new(project_directory).is_absolute() {
        return Err(invalid("project import source"));
    }
    let root = decode_json(bytes, Limits::new(256 * 1024, 16, 4096)?)?;
    let root = root
        .as_object()
        .filter(|v| v.len() == 1)
        .ok_or_else(|| invalid("project import root"))?;
    let servers = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .filter(|v| !v.is_empty() && v.len() <= 32)
        .ok_or_else(|| invalid("project import servers"))?;
    let hash = format!("{:x}", Sha256::digest(bytes));
    let mut drafts = Vec::new();
    for (name, row) in servers {
        let row = row
            .as_object()
            .ok_or_else(|| invalid("project server record"))?;
        if row
            .keys()
            .any(|k| !["command", "args", "cwd"].contains(&k.as_str()))
        {
            return Err(DomainError::Unavailable(
                "MCP import fields require manual configuration/private entry".into(),
            ));
        }
        let executable = row
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| invalid("import command"))?;
        let arguments: Vec<String> = row
            .get("args")
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|_| invalid("import arguments"))?
            .unwrap_or_default();
        let cwd = match row.get("cwd") {
            Some(v) => v.as_str().ok_or_else(|| invalid("import cwd"))?,
            None => project_directory,
        };
        let draft = Draft {
            name: name.clone(),
            configuration: Configuration::Local {
                executable: executable.into(),
                arguments,
                cwd: cwd.into(),
                public_environment: BTreeMap::new(),
                secret_environment: BTreeSet::new(),
            },
            protocol: Revision::Current20260728.as_str().into(),
            auth_profile: None,
            provenance: Provenance::Project {
                path: source_path.into(),
                sha256: hash.clone(),
                entry: name.clone(),
            },
        };
        draft.validate()?;
        drafts.push(draft);
    }
    Ok(drafts)
}
