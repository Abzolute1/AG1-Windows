//! MCP identities and conservative schema admission in the ordinary tool layer.
//! Serialized identities describe provenance; only native owners issue capabilities.
use super::{
    policy::{Decision, Mode, Policy},
    registry::ValidatedCall,
    schema::{self, Schema},
};
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod capability;
pub mod discovery;
pub use capability::Kind;

pub const MAX_SELECTED_TOOLS: usize = 16;
pub const MAX_SELECTION_BYTES: usize = 64 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "workspace",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Scope {
    GeneralChat,
    Workspace(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub server_id: String,
    pub scope: Scope,
    pub config_revision: u64,
    pub auth_profile: String,
    pub credential_generation: u64,
    pub connection_generation: u64,
    pub protocol: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    /// Native conversation owner, independent of the canonical project grant.
    /// Deserializing this provenance never creates an executable capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_scope: Option<ExecutionScope>,
    #[serde(default, skip_serializing_if = "Kind::is_tool")]
    pub kind: Kind,
    pub binding: Binding,
    pub original_name: String,
    pub metadata_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionScope {
    pub workspace: String,
    pub session: String,
    pub epoch: u64,
}
impl ExecutionScope {
    pub fn from_native(scope: &super::registry::Scope) -> Result<Self, DomainError> {
        let value = Self {
            workspace: scope.workspace.clone(),
            session: scope.session.clone(),
            epoch: scope.epoch,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), DomainError> {
        if !label(&self.workspace) || !label(&self.session) || self.epoch == 0 {
            return Err(invalid());
        }
        Ok(())
    }
}
fn invalid() -> DomainError {
    DomainError::InvalidInput("MCP tool identity or metadata".into())
}
fn label(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
fn server_id(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn hash(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
impl Binding {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !server_id(&self.server_id)
            || !label(&self.auth_profile)
            || self.config_revision == 0
            || self.connection_generation == 0
            || !["2025-11-25", "2026-07-28"].contains(&self.protocol.as_str())
            || matches!(&self.scope,Scope::Workspace(id) if !label(id))
        {
            return Err(invalid());
        }
        Ok(())
    }
}
impl Identity {
    pub fn validate(&self) -> Result<(), DomainError> {
        self.binding.validate()?;
        if let Some(owner) = &self.execution_scope {
            owner.validate()?;
        }
        let valid_name = if matches!(self.kind, Kind::Resource | Kind::ResourceTemplate) {
            !self.original_name.is_empty()
                && self.original_name.len() <= 4096
                && !self.original_name.chars().any(char::is_control)
        } else {
            label(&self.original_name)
        };
        if !valid_name
            || self.metadata_digest.len() != 64
            || !self
                .metadata_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        Ok(())
    }
    pub fn wire_name(&self) -> String {
        if self.kind == Kind::Tool {
            wire_name(&self.binding.server_id, &self.original_name)
        } else {
            format!(
                "mcp_{:.56}",
                hash(&json!([
                    self.binding.server_id,
                    self.kind,
                    self.original_name
                ]))
            )
        }
    }
    pub fn target_revision(&self) -> String {
        format!("mcp:{}", hash(&json!(self)))
    }
    pub fn check_scope(&self, scope: &super::registry::Scope) -> Result<(), DomainError> {
        if let Some(owner) = &self.execution_scope {
            if owner.workspace != scope.workspace
                || owner.session != scope.session
                || owner.epoch != scope.epoch
            {
                return Err(DomainError::PermissionDenied);
            }
        } else if matches!(&self.binding.scope,Scope::Workspace(id) if id != &scope.workspace) {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
}
pub fn wire_name(server_id: &str, original_name: &str) -> String {
    format!("mcp_{:.56}", hash(&json!([server_id, original_name])))
}
/// Same structural subset used for inspection and registry admission. Unsupported
/// constraints remain inspectable; neither path strips or flattens them.
pub fn structural_schema(value: &Value) -> Option<Schema> {
    fn compatible(value: &Value) -> bool {
        let Some(object) = value.as_object() else {
            return value.is_boolean();
        };
        for (key, child) in object {
            match key.as_str() {
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
                | "pattern" | "enum" | "const" | "uniqueItems" | "anyOf" | "allOf" | "oneOf"
                | "not" => return false,
                "properties" => {
                    if !child
                        .as_object()
                        .is_some_and(|rows| rows.values().all(compatible))
                    {
                        return false;
                    }
                }
                "items" | "additionalProperties" if !compatible(child) => return false,
                _ => {}
            }
        }
        true
    }
    // Check depth/bytes before the recursive compatibility walk.
    if schema::bounded(value, 256 * 1024, 32).is_err() || !compatible(value) {
        return None;
    }
    Schema::parse(value).ok()
}
#[derive(Clone, Debug)]
pub struct SelectedDefinition {
    pub(crate) identity: Identity,
    pub(crate) metadata: Value,
}
impl SelectedDefinition {
    /// Only whitelisted descriptive fields become tool definitions. Peer-supplied
    /// sourceInfo, promptGuidelines, risk, trust and execution fields remain data.
    pub fn new(binding: Binding, original: &Value) -> Result<Self, DomainError> {
        binding.validate()?;
        schema::bounded(original, 1024 * 1024, 32)?;
        let name = original["name"]
            .as_str()
            .filter(|s| label(s))
            .ok_or_else(invalid)?;
        if original
            .get("description")
            .is_some_and(|d| !d.as_str().is_some_and(|s| s.len() <= 16 * 1024))
            || original.get("annotations").is_some_and(|a| !a.is_object())
        {
            return Err(invalid());
        }
        if original["inputSchema"]["type"] != "object"
            || structural_schema(&original["inputSchema"]).is_none()
        {
            return Err(DomainError::Unavailable(
                "MCP input schema is unsupported".into(),
            ));
        }
        if let Some(output) = original.get("outputSchema")
            && ((binding.protocol == "2025-11-25" && output["type"] != "object")
                || structural_schema(output).is_none())
        {
            return Err(DomainError::Unavailable(
                "MCP output schema is unsupported".into(),
            ));
        }
        let identity = Identity {
            execution_scope: None,
            kind: Kind::Tool,
            binding,
            original_name: name.into(),
            metadata_digest: hash(original),
        };
        let metadata = json!({"name":identity.wire_name(),"description":original.get("description").unwrap_or(&json!("")),
            "parameters":original["inputSchema"],"sourceInfo":{"source":"mcp","serverId":identity.binding.server_id,
                "originalName":identity.original_name,"scope":identity.binding.scope,"identityDigest":identity.target_revision()}});
        Ok(Self { identity, metadata })
    }
    /// Host-only typed publication: peer metadata cannot select this owner.
    pub fn for_execution_scope(
        mut self,
        owner: Option<&ExecutionScope>,
    ) -> Result<Self, DomainError> {
        if let Some(owner) = owner {
            owner.validate()?;
        }
        self.identity.execution_scope = owner.cloned();
        self.metadata["sourceInfo"]["identityDigest"] = json!(self.identity.target_revision());
        Ok(self)
    }
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
    pub fn metadata(&self) -> &Value {
        &self.metadata
    }
}
/// Until reviewed MCP read/write rules exist, every unknown operation requires
/// exact consent. Generic trusted-tools/disabled-policy bypass and server hints
/// cannot classify it as a safe read; planning/headless cannot perform it.
pub(crate) fn decision(call: &ValidatedCall, policy: &Policy) -> Result<Decision, DomainError> {
    let identity = call.mcp_identity().ok_or_else(invalid)?;
    identity.validate()?;
    if matches!(policy.mode, Mode::Planning | Mode::Headless) {
        return Ok(Decision {
            allow: false,
            reason: Some("MCP action risk has not been reviewed for this mode".into()),
            approval_request: None,
        });
    }
    Ok(Decision {
        allow: true,
        reason: None,
        approval_request: Some(json!({
            "mode":policy.mode,"toolName":call.name(),"risk":"unknown","summary":format!("Approve MCP {}",identity.kind.method()),
            "details":format!("Server: {}\nOperation: {}\nTarget: {}\nRequest: {}",identity.binding.server_id,identity.kind.method(),identity.original_name,identity.protocol_params(call.args())?),
            "source":"mcp","identity":identity,"request":identity.protocol_params(call.args())?,"classificationBasis":"No native read/write rule has been reviewed; server annotations do not grant authority"
        })),
    })
}
