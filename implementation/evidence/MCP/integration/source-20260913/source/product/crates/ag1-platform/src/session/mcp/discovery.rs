//! One native session's bounded selection. Each request captures an immutable
//! registry and generation witness; later selection cannot retarget that request.
use super::*;
use ag1_core::tools::{
    executor::Effect,
    mcp::{Kind, SelectedDefinition, discovery::NAME},
    registry::Origin,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

struct State {
    definitions: Vec<SelectedDefinition>,
    selection: Selection,
}
pub struct Discovery {
    lease: Option<Arc<crate::mcp::manager::SessionLease>>,
    manager: Arc<Manager>,
    catalog_scope: CatalogScope,
    scope: Scope,
    policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
    state: Mutex<State>,
    revision: Arc<AtomicU64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Entry {
    connection_id: String,
    kind: Kind,
    key: String,
}
impl From<Entry> for SelectedCapability {
    fn from(e: Entry) -> Self {
        Self {
            connection_id: e.connection_id,
            kind: e.kind,
            key: e.key,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
enum Action {
    Search {
        kind: Kind,
        query: String,
        limit: Option<usize>,
    },
    Inspect {
        entry: Entry,
    },
    Select {
        entries: Vec<Entry>,
    },
    Status,
    Clear,
}
pub(in crate::session) struct PreparedDiscovery {
    action: Action,
    revision: u64,
}
impl PreparedDiscovery {
    pub(in crate::session) fn target_revision(&self) -> String {
        format!("mcp-discovery-{}", self.revision)
    }
}
impl Discovery {
    pub fn new(
        manager: Arc<Manager>,
        catalog_scope: CatalogScope,
        scope: Scope,
        policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
    ) -> Result<Self, DomainError> {
        let mut registry = Registry::builtins()?;
        let empty = SessionCapabilities::new(
            manager.clone(),
            catalog_scope.clone(),
            scope.clone(),
            &mut registry,
            &[],
            policy.clone(),
        )?;
        Ok(Self {
            lease: None,
            manager,
            catalog_scope,
            scope,
            policy,
            state: Mutex::new(State {
                definitions: vec![],
                selection: empty.selection,
            }),
            revision: Arc::new(AtomicU64::new(1)),
        })
    }
    pub fn owned(
        lease: Arc<crate::mcp::manager::SessionLease>,
        scope: Scope,
        policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
    ) -> Result<Self, DomainError> {
        if scope.workspace.is_empty() || scope.session.is_empty() || scope.epoch == 0 {
            return Err(DomainError::PermissionDenied);
        }
        // The native folder resolver bound this lease to the canonical project.
        // Its directory identity may differ from the conversation journal owner.
        let selection = lease.select(&mut Registry::builtins()?, &[])?;
        Ok(Self {
            manager: lease.manager().clone(),
            catalog_scope: lease.scope().clone(),
            lease: Some(lease),
            scope,
            policy,
            state: Mutex::new(State {
                definitions: vec![],
                selection,
            }),
            revision: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn close(&self) -> Result<(), DomainError> {
        if let Some(lease) = &self.lease {
            lease.close()?;
        }
        Ok(())
    }
    fn capabilities(
        &self,
        registry: &mut Registry,
        selected: &[SelectedCapability],
    ) -> Result<Selection, DomainError> {
        if let Some(lease) = &self.lease {
            lease.select(registry, selected)
        } else {
            self.manager
                .select_capabilities(registry, self.catalog_scope.clone(), selected)
        }
    }
    pub fn register(&self, registry: &mut Registry) -> Result<(), DomainError> {
        registry.register_sdk(ag1_core::tools::mcp::discovery::metadata())
    }
    fn owner(&self, scope: &Scope) -> Result<(), DomainError> {
        if let Some(lease) = &self.lease {
            lease.check()?;
        }
        if scope.workspace != self.scope.workspace
            || scope.session != self.scope.session
            || scope.epoch != self.scope.epoch
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    /// Native host selection. The same operation is used by the admitted facade.
    pub fn select(
        &self,
        scope: &Scope,
        selected: &[SelectedCapability],
    ) -> Result<(), DomainError> {
        self.owner(scope)?;
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        self.replace(&mut state, selected)
    }
    fn replace(
        &self,
        state: &mut State,
        selected: &[SelectedCapability],
    ) -> Result<(), DomainError> {
        let revision = self
            .revision
            .load(Ordering::Acquire)
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let mut registry = Registry::builtins()?;
        let selection = self.capabilities(&mut registry, selected)?;
        let definitions = registry.mcp_selection();
        state.definitions = definitions;
        state.selection = selection;
        self.revision.store(revision, Ordering::Release);
        Ok(())
    }
    pub(in crate::session) fn capture(
        &self,
        scope: &Scope,
        registry: &mut Registry,
    ) -> Result<SessionCapabilities, DomainError> {
        self.owner(scope)?;
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        registry.replace_mcp_selection(&state.definitions)?;
        // Never silently adopt refreshed peer metadata under an old selection.
        // Revocation clears the entire small selection; the model can rediscover.
        if state
            .selection
            .rebind_registry(&self.manager, registry)
            .is_err()
        {
            self.replace(&mut state, &[])?;
            registry.replace_mcp_selection(&[])?;
        }
        let selection = state.selection.rebind_registry(&self.manager, registry)?;
        Ok(SessionCapabilities {
            manager: self.manager.clone(),
            scope: self.scope.clone(),
            selection,
            policy: self.policy.clone(),
            generation: Some((self.revision.clone(), self.revision.load(Ordering::Acquire))),
        })
    }
    pub(in crate::session) fn prepare(
        &self,
        call: &ValidatedCall,
        context: AdapterContext<'_>,
    ) -> Result<PreparedDiscovery, DomainError> {
        self.owner(context.scope)?;
        if call.origin() != Origin::TrustedHost || call.name() != NAME {
            return Err(DomainError::PermissionDenied);
        }
        if call.args().to_string().len() > 72 * 1024 {
            return Err(DomainError::LimitExceeded("MCP discovery arguments".into()));
        }
        let action = serde_json::from_value(call.args().clone())
            .map_err(|_| DomainError::InvalidInput("MCP discovery action".into()))?;
        Ok(PreparedDiscovery {
            action,
            revision: self.revision.load(Ordering::Acquire),
        })
    }
    pub(in crate::session) fn execute(
        &self,
        plan: PreparedDiscovery,
        ticket: AuthorizedCall,
        context: AdapterContext<'_>,
    ) -> Result<AdapterOutcome, DomainError> {
        self.owner(context.scope)?;
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        if self.revision.load(Ordering::Acquire) != plan.revision {
            return Err(DomainError::Conflict);
        }
        context
            .journal
            .ok_or(DomainError::PermissionDenied)?
            .check_mcp_scope(context.scope)?;
        if Instant::now() >= context.deadline {
            return Err(DomainError::TimedOut);
        }
        if let Some(policy) = &self.policy
            && policy.snapshot()?.revision != context.policy_revision
        {
            return Err(DomainError::Conflict);
        }
        ticket.consume(
            context.registry,
            ExecutionFence {
                scope: context.scope,
                policy_revision: context.policy_revision,
                target_revision: &plan.target_revision(),
                now: context.clock.now_ms(),
            },
        )?;
        let value = match plan.action {
            Action::Search { kind, query, limit } => {
                let found = if let Some(lease) = &self.lease {
                    lease.search(kind, &query, limit.unwrap_or(10))?
                } else {
                    self.manager.search_capabilities(
                        &self.catalog_scope,
                        kind,
                        &query,
                        limit.unwrap_or(10),
                    )?
                };
                let rows: Vec<_> = found.matches.into_iter().map(|r| json!({"connectionId":r.connection_id,"serverId":r.server_id,"kind":r.kind,"key":r.key,"name":r.name,"description":r.description,"available":r.available})).collect();
                json!({"matches":rows,"searchedEntries":found.searched_entries,"truncated":found.truncated,"source":"cached MCP metadata"})
            }
            Action::Inspect { entry } => {
                let mut registry = Registry::builtins()?;
                self.capabilities(&mut registry, &[entry.into()])?;
                json!({"definitions":registry.mcp_selection().iter().map(|d|json!({"identity":d.identity(),"metadata":d.metadata()})).collect::<Vec<_>>(),"selected":false})
            }
            Action::Select { entries } => {
                self.replace(
                    &mut state,
                    &entries.into_iter().map(Into::into).collect::<Vec<_>>(),
                )?;
                self.status(&state)
            }
            Action::Clear => {
                self.replace(&mut state, &[])?;
                self.status(&state)
            }
            Action::Status => self.status(&state),
        };
        Ok(AdapterOutcome {
            result: json!({"content":[{"type":"text","text":value.to_string()}],"details":{"mcpDiscovery":value}}),
            effect: Effect::None,
            artifact_page: false,
        })
    }
    fn status(&self, state: &State) -> Value {
        json!({"revision":self.revision.load(Ordering::Acquire),"availableOnNextRequest":true,"selected":state.definitions.iter().map(|d|json!({"name":d.identity().wire_name(),"identity":d.identity()})).collect::<Vec<_>>(),"schemaBytes":state.selection.schema_bytes()})
    }
}
