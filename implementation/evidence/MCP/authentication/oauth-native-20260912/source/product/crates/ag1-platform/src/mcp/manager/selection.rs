//! Bounded, scope-filtered catalog selection for ordinary registry preparation.
//! A Selection is a freshness witness, not authority to send an MCP operation.
use super::{ConnectionHealth, Manager, SelectedCapability, scope_admitted};
use crate::mcp::{
    catalog::{CatalogBinding, CatalogScope, SchemaSupport},
    invalid,
    wire::{Limits, decode_json},
};
use ag1_core::{
    DomainError,
    tools::{
        mcp::{self, Kind},
        registry::{Registry, ValidatedCall},
    },
};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug)]
pub struct ToolMatch {
    pub connection_id: String,
    pub server_id: String,
    pub original_name: String,
    pub wire_name: String,
    pub description: String,
    pub schema_support: SchemaSupport,
}
pub struct ToolMatches {
    pub matches: Vec<ToolMatch>,
    pub truncated: bool,
    pub searched_tools: usize,
    pub encoded_bytes: usize,
}
#[derive(Clone, Debug)]
pub struct SelectedTool {
    pub connection_id: String,
    pub original_name: String,
}
pub struct Selection {
    owner: Arc<()>,
    scope: CatalogScope,
    registry_revision: u64,
    pub(super) tools: BTreeMap<String, (String, mcp::Identity)>,
    schema_bytes: usize,
}
impl Selection {
    /// A native registry may change its non-MCP active set for a tool batch.
    /// Rebind the revision only after checking every original identity in place;
    /// never refresh/adopt peer metadata or replace a selected definition here.
    pub(crate) fn rebind_registry(
        &self,
        manager: &Manager,
        registry: &Registry,
    ) -> Result<Self, DomainError> {
        let next = Self {
            owner: self.owner.clone(),
            scope: self.scope.clone(),
            registry_revision: registry.revision(),
            tools: self.tools.clone(),
            schema_bytes: self.schema_bytes,
        };
        next.validate(manager, registry)?;
        Ok(next)
    }

    pub fn wire_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
    /// Exact encoded definition bytes contributed by MCP. Native definitions and
    /// provider wrappers are additional and remain in existing compiler budgets.
    pub fn schema_bytes(&self) -> usize {
        self.schema_bytes
    }
    pub fn target_revision(&self, name: &str) -> Result<String, DomainError> {
        Ok(self
            .tools
            .get(name)
            .ok_or(DomainError::NotFound)?
            .1
            .target_revision())
    }
    pub fn validate(&self, manager: &Manager, registry: &Registry) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&self.owner, &manager.owner)
            || registry.revision() != self.registry_revision
        {
            return Err(DomainError::Conflict);
        }
        let state = manager.state()?;
        for (wire, (connection, identity)) in &self.tools {
            if !registry.definitions().any(|definition| {
                definition.metadata["name"] == *wire
                    && definition.origin == ag1_core::tools::registry::Origin::Mcp
                    && definition.mcp.as_ref() == Some(identity)
            }) {
                return Err(DomainError::Conflict);
            }
            let worker = state.live.get(connection).ok_or(DomainError::Conflict)?;
            let view = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            let record = state
                .records
                .servers
                .get(&view.status.server_id)
                .ok_or(DomainError::Conflict)?;
            if view.status.health != ConnectionHealth::Ready
                || worker.control.credential_check().is_err()
                || view.status.inspection_only
                || view.status.scope != self.scope
                || !record.availability.allows(&self.scope)
                || worker
                    .control
                    .cancelled
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err(DomainError::Conflict);
            }
            let catalog = view.catalog.as_ref().ok_or(DomainError::Conflict)?;
            if record.configuration_revision() != catalog.binding.config_revision
                || binding(&catalog.binding) != identity.binding
                || !catalog.matches(
                    identity,
                    worker
                        .control
                        .lease
                        .as_ref()
                        .and_then(|l| l.execution_scope.as_ref()),
                )
            {
                return Err(DomainError::Conflict);
            }
        }
        Ok(())
    }
    pub fn prepare(
        &self,
        manager: &Manager,
        registry: &Registry,
        operation: &str,
        call_id: &str,
        name: &str,
        arguments: &[u8],
    ) -> Result<ValidatedCall, DomainError> {
        self.validate(manager, registry)?;
        let args = decode_json(arguments, Limits::new(256 * 1024, 32, 20_000)?)?;
        let call = registry.prepare(operation, call_id, name, &args, true)?;
        self.validate_call(manager, registry, &call)?;
        Ok(call)
    }
    /// Recheck after approval or queued request preparation. execute_tool also
    /// refreshes the peer schema and rechecks under the native send admission gate.
    /// Credential freshness is checked alongside configuration and scope.
    pub fn validate_call(
        &self,
        manager: &Manager,
        registry: &Registry,
        call: &ValidatedCall,
    ) -> Result<(), DomainError> {
        self.validate(manager, registry)?;
        registry.validate_current(call)?;
        if self.tools.get(call.name()).map(|(_, identity)| identity) != call.mcp_identity()
            || call.mcp_identity().is_none()
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
}
pub(super) fn binding(native: &CatalogBinding) -> mcp::Binding {
    mcp::Binding {
        server_id: native.server_id.clone(),
        scope: match &native.scope {
            CatalogScope::GeneralChat => mcp::Scope::GeneralChat,
            CatalogScope::Workspace(id) => mcp::Scope::Workspace(id.clone()),
        },
        config_revision: native.config_revision,
        auth_profile: native.auth_profile.clone(),
        credential_generation: native.credential_generation,
        connection_generation: native.connection_generation,
        protocol: native.protocol.as_str().into(),
    }
}
impl Manager {
    /// Search connected approved metadata only. No process, network, resource read,
    /// credential resolution, implicit refresh or schema injection is performed.
    pub fn search_tools(
        &self,
        scope: &CatalogScope,
        query: &str,
        limit: usize,
    ) -> Result<ToolMatches, DomainError> {
        crate::mcp::definitions::validate_scope(scope)?;
        if query.len() > 256 || query.chars().any(char::is_control) || !(1..=20).contains(&limit) {
            return Err(invalid("tool search bounds"));
        }
        let query = query.to_lowercase();
        let state = self.state()?;
        let mut matches = BTreeMap::new();
        let mut found = 0;
        let mut searched_tools = 0;
        for (connection, worker) in &state.live {
            if worker.control.lease.is_some() {
                continue;
            }
            let view = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            let Some(record) = state.records.servers.get(&view.status.server_id) else {
                continue;
            };
            if view.status.health != ConnectionHealth::Ready
                || worker.control.credential_check().is_err()
                || view.status.inspection_only
                || &view.status.scope != scope
                || !record.availability.allows(scope)
                || worker
                    .control
                    .cancelled
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                continue;
            }
            let Some(catalog) = &view.catalog else {
                continue;
            };
            if catalog.binding.config_revision != record.configuration_revision() {
                continue;
            }
            for tool in &catalog.tools {
                searched_tools += 1;
                let original = tool.original();
                let name = original["name"].as_str().ok_or(DomainError::Conflict)?;
                let description = original["description"].as_str().unwrap_or_default();
                if !query.is_empty()
                    && !name.to_lowercase().contains(&query)
                    && !description.to_lowercase().contains(&query)
                {
                    continue;
                }
                found += 1;
                let row = ToolMatch {
                    connection_id: connection.clone(),
                    server_id: record.id.clone(),
                    original_name: name.into(),
                    wire_name: tool.wire_name().into(),
                    description: description.chars().take(160).collect(),
                    schema_support: tool.schema_support(),
                };
                matches.insert((record.id.clone(), name.to_owned()), row);
                if matches.len() > limit {
                    matches.pop_last();
                }
            }
        }
        let matches: Vec<_> = matches.into_values().collect();
        let encoded_bytes=json!(matches.iter().map(|m|json!({"connectionId":m.connection_id,"serverId":m.server_id,"name":m.original_name,"wireName":m.wire_name,
            "description":m.description,"schemaSupport":format!("{:?}",m.schema_support)})).collect::<Vec<_>>()).to_string().len();
        if encoded_bytes > 32 * 1024 {
            return Err(DomainError::LimitExceeded(
                "MCP search summary bytes".into(),
            ));
        }
        Ok(ToolMatches {
            matches,
            truncated: found > limit,
            searched_tools,
            encoded_bytes,
        })
    }
    /// One request scope and a small explicit selection. Registry publication is
    /// atomic and replaces only prior MCP selections, preserving other origins.
    pub fn select_tools(
        &self,
        registry: &mut Registry,
        scope: CatalogScope,
        selected: &[SelectedTool],
    ) -> Result<Selection, DomainError> {
        let selected: Vec<_> = selected
            .iter()
            .map(|s| SelectedCapability {
                connection_id: s.connection_id.clone(),
                kind: Kind::Tool,
                key: s.original_name.clone(),
            })
            .collect();
        self.select_capabilities(registry, scope, &selected)
    }
    /// Publish a small explicit mix of tools, resource reads, templates and prompts.
    /// A selected prompt is untrusted data retrieval, never a system instruction.
    pub fn select_capabilities(
        &self,
        registry: &mut Registry,
        scope: CatalogScope,
        selected: &[SelectedCapability],
    ) -> Result<Selection, DomainError> {
        self.select_capabilities_for(registry, scope, selected, None)
    }
    pub(super) fn select_capabilities_for(
        &self,
        registry: &mut Registry,
        scope: CatalogScope,
        selected: &[SelectedCapability],
        lease: Option<&Arc<super::lease::Token>>,
    ) -> Result<Selection, DomainError> {
        crate::mcp::definitions::validate_scope(&scope)?;
        if selected.len() > mcp::MAX_SELECTED_TOOLS {
            return Err(DomainError::LimitExceeded("MCP selected tool count".into()));
        }
        let state = self.state()?;
        let mut definitions = Vec::new();
        let mut tools = BTreeMap::new();
        let mut schema_bytes = 0usize;
        for request in selected {
            let worker = state
                .live
                .get(&request.connection_id)
                .ok_or(DomainError::NotFound)?;
            if !super::lease::same(worker.control.lease.as_ref(), lease) {
                return Err(DomainError::PermissionDenied);
            }
            let view = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            let record = state
                .records
                .servers
                .get(&view.status.server_id)
                .ok_or(DomainError::NotFound)?;
            if view.status.health != ConnectionHealth::Ready
                || worker.control.credential_check().is_err()
                || view.status.inspection_only
                || view.status.scope != scope
                || !scope_admitted(&record.availability, &scope, false)
                || worker
                    .control
                    .cancelled
                    .load(std::sync::atomic::Ordering::Acquire)
            {
                return Err(DomainError::PermissionDenied);
            }
            let catalog = view.catalog.as_ref().ok_or(DomainError::Conflict)?;
            if catalog.binding.config_revision != record.configuration_revision() {
                return Err(DomainError::Conflict);
            }
            let definition = catalog
                .definition(request.kind, &request.key)?
                .for_execution_scope(lease.and_then(|l| l.execution_scope.as_ref()))?;
            schema_bytes = schema_bytes
                .checked_add(definition.metadata().to_string().len())
                .ok_or(DomainError::Conflict)?;
            if schema_bytes > mcp::MAX_SELECTION_BYTES {
                return Err(DomainError::LimitExceeded(
                    "MCP selected schema bytes".into(),
                ));
            }
            if tools
                .insert(
                    definition.identity().wire_name(),
                    (request.connection_id.clone(), definition.identity().clone()),
                )
                .is_some()
            {
                return Err(DomainError::Conflict);
            }
            definitions.push(definition);
        }
        registry.replace_mcp_selection(&definitions)?;
        Ok(Selection {
            owner: self.owner.clone(),
            scope,
            registry_revision: registry.revision(),
            tools,
            schema_bytes,
        })
    }
}
