//! Scope-filtered native capability selection; catalog entries are not grants.
use super::{ConnectedCatalog, ConnectionHealth, Manager, selection::binding};
use crate::mcp::{catalog::CatalogScope, definitions::validate_scope};
use ag1_core::{
    DomainError,
    tools::mcp::{Identity, Kind, SelectedDefinition},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::atomic::Ordering};

#[derive(Clone, Debug)]
pub struct SelectedCapability {
    pub connection_id: String,
    pub kind: Kind,
    pub key: String,
}
#[derive(Clone, Debug)]
pub struct CapabilityMatch {
    pub connection_id: String,
    pub server_id: String,
    pub kind: Kind,
    pub key: String,
    pub name: String,
    pub description: String,
    pub available: bool,
}
pub struct CapabilityMatches {
    pub matches: Vec<CapabilityMatch>,
    pub searched_entries: usize,
    pub truncated: bool,
    pub encoded_bytes: usize,
}
impl ConnectedCatalog {
    pub(super) fn rows(&self, kind: Kind) -> Vec<&Value> {
        match kind {
            Kind::Tool => self.tools.iter().map(|t| t.original()).collect(),
            Kind::Resource => self.resources.iter().collect(),
            Kind::ResourceTemplate => self.templates.iter().collect(),
            Kind::Prompt => self.prompts.iter().collect(),
        }
    }
    pub(crate) fn definition(
        &self,
        kind: Kind,
        key: &str,
    ) -> Result<SelectedDefinition, DomainError> {
        let row = self
            .rows(kind)
            .into_iter()
            .find(|row| row[kind.key_field()] == key)
            .ok_or(DomainError::NotFound)?;
        SelectedDefinition::capability(binding(&self.binding), kind, row)
    }
    pub(super) fn matches(
        &self,
        identity: &Identity,
        owner: Option<&ag1_core::tools::mcp::ExecutionScope>,
    ) -> bool {
        if binding(&self.binding) != identity.binding || identity.execution_scope.as_ref() != owner
        {
            return false;
        }
        if identity.kind == Kind::Tool {
            return self.tools.iter().any(|t| {
                t.wire_name() == identity.wire_name() && t.digest() == identity.metadata_digest
            });
        }
        self.definition(identity.kind, &identity.original_name)
            .and_then(|d| d.for_execution_scope(owner))
            .is_ok_and(|d| d.identity() == identity)
    }
}
impl Manager {
    /// Search already connected metadata. Never starts/refreshes a connection,
    /// expands a template, reads a resource, fetches a URI, or obtains a prompt.
    pub fn search_capabilities(
        &self,
        scope: &CatalogScope,
        kind: Kind,
        query: &str,
        limit: usize,
    ) -> Result<CapabilityMatches, DomainError> {
        self.search_capabilities_for(scope, kind, query, limit, None)
    }
    pub(super) fn search_capabilities_for(
        &self,
        scope: &CatalogScope,
        kind: Kind,
        query: &str,
        limit: usize,
        lease: Option<&std::sync::Arc<super::lease::Token>>,
    ) -> Result<CapabilityMatches, DomainError> {
        validate_scope(scope)?;
        if query.len() > 256 || query.chars().any(char::is_control) || !(1..=20).contains(&limit) {
            return Err(DomainError::InvalidInput(
                "MCP capability search bounds".into(),
            ));
        }
        let query = query.to_lowercase();
        let state = self.state()?;
        let mut matches = BTreeMap::new();
        let mut found = 0;
        let mut searched_entries = 0;
        for (connection, worker) in &state.live {
            if !super::lease::same(worker.control.lease.as_ref(), lease) {
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
                || worker.control.cancelled.load(Ordering::Acquire)
            {
                continue;
            }
            let Some(catalog) = &view.catalog else {
                continue;
            };
            if catalog.binding.config_revision != record.configuration_revision() {
                continue;
            }
            for row in catalog.rows(kind) {
                searched_entries += 1;
                let key = row[kind.key_field()]
                    .as_str()
                    .ok_or(DomainError::Conflict)?;
                let name = row["name"].as_str().ok_or(DomainError::Conflict)?;
                let description = row["description"].as_str().unwrap_or_default();
                if !query.is_empty()
                    && !key.to_lowercase().contains(&query)
                    && !name.to_lowercase().contains(&query)
                    && !description.to_lowercase().contains(&query)
                {
                    continue;
                }
                found += 1;
                matches.insert(
                    (record.id.clone(), key.to_owned()),
                    CapabilityMatch {
                        connection_id: connection.clone(),
                        server_id: record.id.clone(),
                        kind,
                        key: key.into(),
                        name: name.into(),
                        description: description.chars().take(160).collect(),
                        available: SelectedDefinition::capability(
                            binding(&catalog.binding),
                            kind,
                            row,
                        )
                        .is_ok(),
                    },
                );
                if matches.len() > limit {
                    matches.pop_last();
                }
            }
        }
        let matches: Vec<_> = matches.into_values().collect();
        let encoded_bytes=json!(matches.iter().map(|m|json!({"connectionId":m.connection_id,"serverId":m.server_id,"kind":m.kind,"key":m.key,"name":m.name,"description":m.description,"available":m.available})).collect::<Vec<_>>()).to_string().len();
        if encoded_bytes > 32 * 1024 {
            return Err(DomainError::LimitExceeded(
                "MCP search summary bytes".into(),
            ));
        }
        Ok(CapabilityMatches {
            matches,
            searched_entries,
            truncated: found > limit,
            encoded_bytes,
        })
    }
}
