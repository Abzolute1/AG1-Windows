//! Private connected-server tool metadata, separate from public registry data.
//! No snapshot, schema check or server annotation here authorizes execution.
use super::{
    Revision, invalid,
    wire::{Limits, decode_json},
};
use ag1_core::{DomainError, tools::schema::Schema};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOOLS: usize = 1024;
const MAX_PAGES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogScope {
    GeneralChat,
    /// Native canonical workspace identity, not a server-provided path or root.
    Workspace(String),
}

/// Supplied by the native owner. Not deserializable from server metadata.
/// Config revision must change with endpoint/launch edits; credential generation
/// must change with account, resource, issuer, scope or credential changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogBinding {
    pub server_id: String,
    pub scope: CatalogScope,
    pub config_revision: u64,
    pub auth_profile: String,
    pub credential_generation: u64,
    pub connection_generation: u64,
    pub protocol: Revision,
}

impl CatalogBinding {
    fn validate(&self) -> Result<(), DomainError> {
        let id = self.server_id.as_bytes();
        let uuid = id.len() == 36
            && id.iter().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    *b == b'-'
                } else {
                    b.is_ascii_digit() || (b'a'..=b'f').contains(b)
                }
            });
        let workspace_valid = match &self.scope {
            CatalogScope::GeneralChat => true,
            CatalogScope::Workspace(id) => bounded_identity(id),
        };
        if !uuid
            || !workspace_valid
            || !bounded_identity(&self.auth_profile)
            || self.config_revision == 0
            || self.connection_generation == 0
        {
            return Err(invalid("native catalog binding"));
        }
        Ok(())
    }
}

fn bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchemaSupport {
    Supported,
    UnsupportedInput,
    UnsupportedOutput,
}

#[derive(Clone, Debug)]
pub struct Tool {
    original: Value,
    wire_name: String,
    digest: String,
    support: SchemaSupport,
    input: Option<Schema>,
    output: Option<Schema>,
}

impl Tool {
    pub fn original(&self) -> &Value {
        &self.original
    }
    pub fn wire_name(&self) -> &str {
        &self.wire_name
    }
    /// Includes name, description, schemas and annotations. Changes require review.
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn schema_support(&self) -> SchemaSupport {
        self.support
    }

    /// Strict validation only; never use legacy scalar coercion for MCP.
    /// Successful validation does not establish an execution capability or grant.
    pub fn validate_arguments(&self, bytes: &[u8]) -> Result<Value, DomainError> {
        if self.support != SchemaSupport::Supported {
            return Err(DomainError::Unavailable(
                "MCP tool schema is unsupported".into(),
            ));
        }
        let args = decode_json(bytes, Limits::new(256 * 1024, 32, 20_000)?)?;
        self.input
            .as_ref()
            .ok_or_else(|| invalid("missing input validator"))?
            .validate(&args)?;
        Ok(args)
    }

    pub fn validate_structured_output(&self, bytes: &[u8]) -> Result<Value, DomainError> {
        if self.support != SchemaSupport::Supported {
            return Err(DomainError::Unavailable(
                "MCP tool schema is unsupported".into(),
            ));
        }
        let value = decode_json(bytes, Limits::default())?;
        if let Some(schema) = &self.output {
            schema.validate(&value)?;
        }
        Ok(value)
    }
}

use ag1_core::tools::mcp::structural_schema;

fn parse_tool(row: Value, binding: &CatalogBinding) -> Result<Tool, DomainError> {
    let name = row
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| bounded_identity(s))
        .ok_or_else(|| invalid("tool name"))?;
    if row
        .get("description")
        .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 16 * 1024))
        || row.get("annotations").is_some_and(|v| !v.is_object())
        || !row.get("inputSchema").is_some_and(Value::is_object)
    {
        return Err(invalid("tool metadata"));
    }
    let input = if row["inputSchema"]["type"] == "object" {
        structural_schema(&row["inputSchema"])
    } else {
        None
    };
    let output = row.get("outputSchema").and_then(|schema| {
        if binding.protocol == Revision::Legacy20251125 && schema["type"] != "object" {
            None
        } else {
            structural_schema(schema)
        }
    });
    let support = if input.is_none() {
        SchemaSupport::UnsupportedInput
    } else if row.get("outputSchema").is_some() && output.is_none() {
        SchemaSupport::UnsupportedOutput
    } else {
        SchemaSupport::Supported
    };
    let wire_name = ag1_core::tools::mcp::wire_name(&binding.server_id, name);
    let digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&row).map_err(|_| invalid("tool metadata"))?)
    );
    Ok(Tool {
        original: row,
        wire_name,
        digest,
        support,
        input,
        output,
    })
}

#[derive(Debug)]
pub struct ToolsCatalog {
    binding: CatalogBinding,
    generation: u64,
    tools: BTreeMap<String, Tool>,
}

impl ToolsCatalog {
    pub fn binding(&self) -> &CatalogBinding {
        &self.binding
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn tools(&self) -> impl Iterator<Item = &Tool> {
        self.tools.values()
    }
    pub fn get(&self, original_name: &str) -> Option<&Tool> {
        self.tools.get(original_name)
    }
}

/// One native-owned connection slot. Refresh retires the old live catalog before
/// receiving pages; an incomplete/failed refresh cannot keep stale tools usable.
/// Historical inspection/evidence belongs in retained storage, not this live slot.
#[derive(Debug)]
pub struct CatalogSlot {
    owner: Arc<()>,
    binding: CatalogBinding,
    generation: u64,
    current: Option<ToolsCatalog>,
}

impl CatalogSlot {
    pub fn new(binding: CatalogBinding) -> Result<Self, DomainError> {
        binding.validate()?;
        Ok(Self {
            owner: Arc::new(()),
            binding,
            generation: 0,
            current: None,
        })
    }
    pub fn current(&self) -> Option<&ToolsCatalog> {
        self.current.as_ref()
    }
    pub fn invalidate(&mut self) -> Result<(), DomainError> {
        self.current = None;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        Ok(())
    }
    /// Rebind only from native configuration/auth/scope state; never server fields.
    pub fn rebind(&mut self, binding: CatalogBinding) -> Result<(), DomainError> {
        self.invalidate()?;
        binding.validate()?;
        self.binding = binding;
        Ok(())
    }
    pub fn begin_refresh(&mut self) -> Result<ToolsRefresh, DomainError> {
        self.invalidate()?;
        Ok(ToolsRefresh {
            owner: Arc::clone(&self.owner),
            binding: self.binding.clone(),
            generation: self.generation,
            tools: BTreeMap::new(),
            cursors: BTreeSet::new(),
            pages: 0,
            bytes: 0,
            complete: false,
            failed: false,
        })
    }
    pub fn publish(&mut self, refresh: ToolsRefresh) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&refresh.owner, &self.owner)
            || refresh.binding != self.binding
            || refresh.generation != self.generation
            || refresh.failed
            || !refresh.complete
            || self.current.is_some()
        {
            return Err(DomainError::Conflict);
        }
        self.current = Some(ToolsCatalog {
            binding: refresh.binding,
            generation: refresh.generation,
            tools: refresh.tools,
        });
        Ok(())
    }
}

/// Pages are tools/list result objects after request-ID correlation by the future
/// transport owner. The returned cursor is opaque and must stay on this connection.
pub struct ToolsRefresh {
    owner: Arc<()>,
    binding: CatalogBinding,
    generation: u64,
    tools: BTreeMap<String, Tool>,
    cursors: BTreeSet<String>,
    pages: usize,
    bytes: usize,
    complete: bool,
    failed: bool,
}

impl ToolsRefresh {
    pub fn push_page(&mut self, bytes: &[u8]) -> Result<Option<String>, DomainError> {
        if self.failed || self.complete {
            self.failed = true;
            return Err(invalid("catalog refresh retired"));
        }
        let result = self.push_inner(bytes);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn push_inner(&mut self, bytes: &[u8]) -> Result<Option<String>, DomainError> {
        if self.pages >= MAX_PAGES || bytes.len() > MAX_CATALOG_BYTES.saturating_sub(self.bytes) {
            return Err(DomainError::LimitExceeded("MCP tool catalog".into()));
        }
        let mut page = decode_json(bytes, Limits::default())?;
        let rows = page
            .get_mut("tools")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| invalid("tools/list result"))?;
        if rows.len() > MAX_TOOLS.saturating_sub(self.tools.len()) {
            return Err(DomainError::LimitExceeded("MCP catalog tools".into()));
        }
        for row in std::mem::take(rows) {
            let tool = parse_tool(row, &self.binding)?;
            let name = tool.original["name"]
                .as_str()
                .ok_or_else(|| invalid("tool name"))?
                .to_owned();
            if self.tools.insert(name, tool).is_some() {
                return Err(invalid("duplicate tool name"));
            }
        }
        let cursor = match page.get("nextCursor") {
            None => None,
            Some(Value::String(s)) if !s.is_empty() && s.len() <= 1024 => Some(s.clone()),
            _ => return Err(invalid("catalog cursor")),
        };
        if let Some(cursor) = &cursor {
            if !self.cursors.insert(cursor.clone()) {
                return Err(invalid("repeated catalog cursor"));
            }
            if self.pages + 1 == MAX_PAGES {
                return Err(DomainError::LimitExceeded("MCP catalog pages".into()));
            }
        }
        self.pages += 1;
        self.bytes += bytes.len();
        self.complete = cursor.is_none();
        Ok(cursor)
    }
}
