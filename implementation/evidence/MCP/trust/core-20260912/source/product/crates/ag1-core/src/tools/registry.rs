use super::{arguments, schema::Schema};
use crate::DomainError;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Responses retains both the provider call ID and output item ID for replay.
/// This is provider metadata, not the host operation/storage identity.
pub(super) fn validate_call_id(id: &str) -> Result<(), DomainError> {
    if id.len() > 128 {
        return Err(DomainError::InvalidInput("tool call identity".into()));
    }
    if let Some((call, item)) = id.split_once('|') {
        crate::OperationId::new(call)?;
        crate::OperationId::new(item)?;
    } else {
        crate::OperationId::new(id)?;
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    Extension,
    TrustedHost,
    Mcp,
}
#[derive(Clone, Debug)]
pub struct Definition {
    pub metadata: Value,
    pub schema: Schema,
    pub origin: Origin,
    pub mcp: Option<super::mcp::Identity>,
    mcp_reviewed_policy: Option<super::mcp::trust::ReviewedPolicy>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Scope {
    pub workspace: String,
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
}
/// Constructed only by registry validation; callers cannot substitute approved arguments.
#[derive(Clone, Debug)]
pub struct ValidatedCall {
    pub(crate) operation: String,
    pub(crate) call_id: String,
    pub(crate) name: String,
    pub(crate) args: Value,
    pub(crate) args_hash: String,
    pub(crate) registry_revision: u64,
    pub(crate) origin: Origin,
    pub(crate) mcp: Option<std::sync::Arc<super::mcp::Identity>>,
    mcp_reviewed_policy: Option<super::mcp::trust::ReviewedPolicy>,
}
impl ValidatedCall {
    pub fn operation(&self) -> &str {
        &self.operation
    }
    pub fn call_id(&self) -> &str {
        &self.call_id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn args(&self) -> &Value {
        &self.args
    }
    pub fn args_hash(&self) -> &str {
        &self.args_hash
    }
    pub fn mcp_identity(&self) -> Option<&super::mcp::Identity> {
        self.mcp.as_deref()
    }
    pub(crate) fn mcp_reviewed_policy(&self) -> Option<&super::mcp::trust::ReviewedPolicy> {
        self.mcp_reviewed_policy.as_ref()
    }
    pub fn origin(&self) -> Origin {
        self.origin
    }
}
#[derive(Clone, Debug)]
pub struct Registry {
    definitions: BTreeMap<String, Definition>,
    active: BTreeSet<String>,
    revision: u64,
}
impl Registry {
    pub fn builtins() -> Result<Self, DomainError> {
        let rows: Vec<Value> =
            serde_json::from_str(include_str!("../../resources/tool-catalog.json"))
                .map_err(|_| DomainError::InvalidInput("built-in catalog".into()))?;
        let mut registry = Self {
            definitions: BTreeMap::new(),
            active: BTreeSet::new(),
            revision: 0,
        };
        for row in rows {
            registry.register(row, Origin::BuiltIn)?;
        }
        super::surface::register(&mut registry)?;
        Ok(registry)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn definitions(&self) -> impl Iterator<Item = &Definition> {
        self.definitions.values()
    }
    pub fn active_names(&self) -> Vec<String> {
        self.active.iter().cloned().collect()
    }
    /// Public provenance is descriptive; Origin and native capabilities retain
    /// authority. Built-in provenance cannot be overridden by metadata fields.
    pub fn public_tools(&self) -> Vec<Value> {
        self.definitions.iter().map(|(name, definition)| {
            let visual = matches!(name.as_str(), "screenshot_url" | "interact_then_screenshot" | "verify_ui");
            let source = if visual { "visual" } else { "builtin" };
            let source_info = if definition.origin == Origin::BuiltIn {
                serde_json::json!({"path":format!("<{source}:{name}>"),"source":source,"scope":"temporary","origin":"top-level"})
            } else { definition.metadata.get("sourceInfo").cloned().unwrap_or_else(|| serde_json::json!({"path":format!("<host:{name}>"),"source":"host","scope":"temporary","origin":"top-level"})) };
            serde_json::json!({"name":name,"description":definition.metadata["description"],"parameters":definition.metadata["parameters"],"sourceInfo":source_info})
        }).collect()
    }
    /// SDK custom definitions are admitted by a native host and remain inactive
    /// until the host supplies their execution capabilities.
    pub fn register_sdk(&mut self, mut metadata: Value) -> Result<(), DomainError> {
        super::schema::bounded(&metadata, 256 * 1024, 32)?;
        let name = metadata["name"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("tool name".into()))?;
        metadata["sourceInfo"] = serde_json::json!({"path":format!("<sdk:{name}>"),"source":"sdk","scope":"temporary","origin":"top-level"});
        self.register(metadata, Origin::TrustedHost)
    }
    /// Seed SDK defaults only where a concrete native execution capability was
    /// supplied. Target visibility has already been filtered by its binding.
    pub fn activate_defaults(&mut self, available: &BTreeSet<String>) -> Result<(), DomainError> {
        const DEFAULTS: &[&str] = &[
            "read",
            "grep",
            "find",
            "ls",
            "bash",
            "edit",
            "write",
            "webSearch",
            "webFetch",
            "todo",
            "repo",
            "sourceContext",
            "memory",
            "artifact",
            "taskState",
            "runtimeObservation",
            "episode",
            "job",
            "check",
            "preview",
            "verify",
            "inspect",
            "ui",
            "work",
            "evidence",
            "retainEvidence",
            "releaseEvidence",
            "capability",
        ];
        let selected = self
            .definitions
            .iter()
            .filter(|(name, definition)| {
                available.contains(*name)
                    && (DEFAULTS.contains(&name.as_str())
                        || matches!(
                            name.as_str(),
                            "screenshot_url" | "interact_then_screenshot" | "verify_ui"
                        )
                        || definition.origin == Origin::Extension
                        || definition.metadata["sourceInfo"]["source"] == "sdk")
            })
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        self.activate(&selected, available)
    }
    /// Rebuild P07's prompt from current active definitions after dynamic tool
    /// changes. Base resource instructions remain host-owned and are not mutated.
    pub fn build_prompt(
        &self,
        base: &crate::context::prompt::PromptInput,
    ) -> Result<String, DomainError> {
        let mut input = base.clone();
        input.selected_tools = Some(self.active_names());
        input.tool_snippets.clear();
        for name in &self.active {
            let metadata = &self.definitions[name].metadata;
            let normalize = super::research::compact;
            let explicit = metadata["promptSnippet"]
                .as_str()
                .map(normalize)
                .filter(|s| !s.is_empty());
            let snippet = explicit.unwrap_or_else(|| {
                let text = normalize(metadata["description"].as_str().unwrap_or_default());
                if text.encode_utf16().count() <= 140 {
                    return text;
                }
                let mut units = 0;
                let prefix: String = text
                    .chars()
                    .take_while(|c| {
                        units += c.len_utf16();
                        units <= 137
                    })
                    .collect();
                format!("{}...", prefix.trim_end())
            });
            if !snippet.is_empty() {
                input.tool_snippets.insert(name.clone(), snippet);
            }
            if let Some(guidelines) = metadata["promptGuidelines"].as_array() {
                for guideline in guidelines
                    .iter()
                    .filter_map(Value::as_str)
                    .map(super::research::trim)
                    .filter(|s| !s.is_empty())
                {
                    if !input
                        .prompt_guidelines
                        .iter()
                        .any(|existing| existing == guideline)
                    {
                        input.prompt_guidelines.push(guideline.into());
                    }
                }
            }
        }
        crate::context::prompt::build(&input)
    }
    pub fn register(&mut self, metadata: Value, origin: Origin) -> Result<(), DomainError> {
        if origin == Origin::Mcp {
            return Err(DomainError::PermissionDenied);
        }
        self.insert(metadata, origin, None, None)
    }
    fn insert(
        &mut self,
        metadata: Value,
        origin: Origin,
        mcp: Option<super::mcp::Identity>,
        mcp_reviewed_policy: Option<super::mcp::trust::ReviewedPolicy>,
    ) -> Result<(), DomainError> {
        super::schema::bounded(&metadata, 256 * 1024, 32)?;
        let name = metadata["name"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("tool name".into()))?;
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
            || !metadata["description"].is_string()
        {
            return Err(DomainError::InvalidInput("tool definition".into()));
        }
        if self.definitions.contains_key(name) {
            return Err(DomainError::Conflict);
        }
        if self.definitions.len() >= 256 {
            return Err(DomainError::LimitExceeded("tool definitions".into()));
        }
        let schema = Schema::parse(&metadata["parameters"])?;
        self.revision = self.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        self.definitions.insert(
            name.into(),
            Definition {
                metadata,
                schema,
                origin,
                mcp,
                mcp_reviewed_policy,
            },
        );
        Ok(())
    }
    /// Copy native-validated MCP capabilities without reconstructing authority
    /// from serialized model metadata. Includes only this registry's MCP origin.
    pub fn mcp_selection(&self) -> Vec<super::mcp::SelectedDefinition> {
        self.definitions
            .values()
            .filter(|d| d.origin == Origin::Mcp)
            .map(|d| super::mcp::SelectedDefinition {
                identity: d.mcp.clone().expect("MCP definition identity"),
                metadata: d.metadata.clone(),
                reviewed_policy: d.mcp_reviewed_policy.clone(),
            })
            .collect()
    }
    /// Replace only MCP's selected definitions, atomically. Other origins and
    /// their active selection are preserved. Identical selections do not churn
    /// registry revisions; any bound identity/metadata change invalidates calls.
    pub fn replace_mcp_selection(
        &mut self,
        selected: &[super::mcp::SelectedDefinition],
    ) -> Result<(), DomainError> {
        if selected.len() > super::mcp::MAX_SELECTED_TOOLS {
            return Err(DomainError::LimitExceeded("MCP selected tool count".into()));
        }
        let mut bytes = 0usize;
        for definition in selected {
            bytes = bytes
                .checked_add(definition.metadata().to_string().len())
                .ok_or(DomainError::Conflict)?;
            if bytes > super::mcp::MAX_SELECTION_BYTES {
                return Err(DomainError::LimitExceeded(
                    "MCP selected schema bytes".into(),
                ));
            }
        }
        let mut next = self.clone();
        next.definitions.retain(|_, d| d.origin != Origin::Mcp);
        next.active.retain(|n| next.definitions.contains_key(n));
        for definition in selected {
            if let Some(review) = &definition.reviewed_policy {
                review.check(definition.identity())?;
            }
            let name = definition.identity().wire_name();
            next.insert(
                definition.metadata().clone(),
                Origin::Mcp,
                Some(definition.identity().clone()),
                definition.reviewed_policy.clone(),
            )?;
            next.active.insert(name);
        }
        let identical = self.active == next.active
            && self.definitions.len() == next.definitions.len()
            && self.definitions.iter().all(|(name, d)| {
                next.definitions.get(name).is_some_and(|n| {
                    d.origin == n.origin
                        && d.mcp == n.mcp
                        && d.metadata == n.metadata
                        && d.mcp_reviewed_policy == n.mcp_reviewed_policy
                })
            });
        if !identical {
            next.revision = self.revision.checked_add(1).ok_or(DomainError::Conflict)?;
            *self = next;
        }
        Ok(())
    }
    /// Validate the entire extension contribution before publishing any definition.
    /// A rejected schema or collision leaves both active selection and revision intact.
    pub fn register_extensions(&mut self, definitions: &[Value]) -> Result<(), DomainError> {
        if definitions.len() > 256 {
            return Err(DomainError::LimitExceeded(
                "extension tool definitions".into(),
            ));
        }
        let mut next = self.clone();
        for definition in definitions {
            next.register(definition.clone(), Origin::Extension)?;
        }
        *self = next;
        Ok(())
    }
    /// Refresh only the definitions owned by one admitted extension host. Publish
    /// schema changes and capability selection together, invalidating old approvals.
    pub fn replace_extension_group(
        &mut self,
        previous: &[String],
        definitions: &[Value],
        available: &BTreeSet<String>,
        activate_new: bool,
    ) -> Result<(), DomainError> {
        if previous.len() > 256 || definitions.len() > 256 {
            return Err(DomainError::LimitExceeded(
                "extension tool definitions".into(),
            ));
        }
        let mut next = self.clone();
        let old_active = self.active.clone();
        for name in previous {
            next.unregister_extension(name)?;
        }
        next.register_extensions(definitions)?;
        let mut active = old_active
            .into_iter()
            .filter(|name| next.definitions.contains_key(name) && available.contains(name))
            .collect::<BTreeSet<_>>();
        if activate_new {
            for definition in definitions {
                let name = definition["name"]
                    .as_str()
                    .ok_or_else(|| DomainError::InvalidInput("extension tool name".into()))?;
                if available.contains(name) {
                    active.insert(name.into());
                }
            }
        }
        next.activate(&active.into_iter().collect::<Vec<_>>(), available)?;
        *self = next;
        Ok(())
    }
    /// Removal is explicit; stale pending operations fail their registry revision fence.
    pub fn unregister_extension(&mut self, name: &str) -> Result<(), DomainError> {
        if self
            .definitions
            .get(name)
            .is_none_or(|d| d.origin != Origin::Extension)
        {
            return Err(DomainError::PermissionDenied);
        }
        self.revision = self.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        self.active.remove(name);
        self.definitions.remove(name);
        Ok(())
    }
    /// Host capabilities are supplied explicitly; unavailable definitions cannot become active.
    pub fn activate(
        &mut self,
        names: &[String],
        available: &BTreeSet<String>,
    ) -> Result<(), DomainError> {
        if names.len() > 256
            || names
                .iter()
                .any(|n| !self.definitions.contains_key(n) || !available.contains(n))
        {
            return Err(DomainError::Unavailable("selected tool capability".into()));
        }
        let next: BTreeSet<_> = names.iter().cloned().collect();
        if next != self.active {
            self.revision = self.revision.checked_add(1).ok_or(DomainError::Conflict)?;
            self.active = next;
        }
        Ok(())
    }
    pub fn prepare(
        &self,
        operation: &str,
        call_id: &str,
        name: &str,
        args: &Value,
        complete: bool,
    ) -> Result<ValidatedCall, DomainError> {
        crate::OperationId::new(operation)?;
        validate_call_id(call_id)?;
        if !complete {
            return Err(DomainError::InvalidInput("Tool call arguments were truncated or malformed (stream interrupted). The call was not executed. Retry the tool call with complete arguments.".into()));
        }
        let definition = self
            .definitions
            .get(name)
            .filter(|_| self.active.contains(name))
            .ok_or_else(|| DomainError::Unavailable(format!("Tool {name} not available")))?;
        if let Some(review) = &definition.mcp_reviewed_policy {
            review.check(definition.mcp.as_ref().ok_or(DomainError::Conflict)?)?;
        }
        super::schema::bounded(args, 8 * 1024 * 1024, 64)?;
        if name == "editPreview"
            && definition.origin == Origin::BuiltIn
            && !args["newText"].is_string()
        {
            return Err(DomainError::InvalidInput("editPreview requires newText as a string. Use an explicit empty string only for intentional deletion; null and omitted values are not replacements.".into()));
        }
        if args.to_string().len() > 8 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("tool arguments".into()));
        }
        let args = if definition.origin == Origin::Mcp {
            super::schema::bounded(args, 256 * 1024, 32)?;
            if args.to_string().len() > 256 * 1024 {
                return Err(DomainError::LimitExceeded("MCP argument bytes".into()));
            }
            definition.schema.validate(args)?;
            args.clone()
        } else {
            definition.schema.coerce(&arguments::prepare(name, args))?
        };
        if name == "check" || name == "artifact" && args["action"] == "lease" {
            super::verification::scope(&args)?;
        }
        let args_hash = format!("{:x}", Sha256::digest(args.to_string().as_bytes()));
        Ok(ValidatedCall {
            operation: operation.into(),
            call_id: call_id.into(),
            name: name.into(),
            args,
            args_hash,
            registry_revision: self.revision,
            origin: definition.origin,
            mcp_reviewed_policy: definition.mcp_reviewed_policy.clone(),
            mcp: definition
                .mcp
                .as_ref()
                .map(|identity| std::sync::Arc::new(identity.clone())),
        })
    }
    pub fn validate_current(&self, call: &ValidatedCall) -> Result<(), DomainError> {
        if let Some(review) = &call.mcp_reviewed_policy {
            review.check(call.mcp_identity().ok_or(DomainError::Conflict)?)?;
        }
        if self.revision != call.registry_revision
            || !self.active.contains(&call.name)
            || (call.origin == Origin::Mcp
                && self.definitions.get(&call.name).is_none_or(|definition| {
                    definition.origin != Origin::Mcp
                        || definition.mcp.as_ref() != call.mcp_identity()
                        || definition.mcp_reviewed_policy != call.mcp_reviewed_policy
                }))
        {
            Err(DomainError::Conflict)
        } else {
            Ok(())
        }
    }
}
