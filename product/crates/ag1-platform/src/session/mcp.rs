//! Native-owned, immutable selected MCP capabilities for one session epoch.
//! Catalog scope is supplied by the native host, never tool arguments or results.
use super::{adapters::AdapterContext, tool_journal::ToolJournal};
use crate::mcp::{
    catalog::CatalogScope,
    manager::{Manager, PreparedOperation, RuntimeCheck, SelectedCapability, Selection},
};
use ag1_core::{
    DomainError,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, ExecutionArtifacts},
        registry::{Registry, Scope, ValidatedCall},
    },
};
use std::{sync::Arc, time::Instant};

pub mod discovery;

pub struct SessionCapabilities {
    generation: Option<(Arc<std::sync::atomic::AtomicU64>, u64)>,
    manager: Arc<Manager>,
    scope: Scope,
    selection: Selection,
    policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
}
impl SessionCapabilities {
    /// Host selection only; imported metadata or a model cannot create this port.
    /// General chat must be explicitly bound to its native account-session owner.
    pub fn new(
        manager: Arc<Manager>,
        catalog_scope: CatalogScope,
        scope: Scope,
        registry: &mut Registry,
        selected: &[SelectedCapability],
        policy: Option<Arc<crate::client::approval_policy::ApprovalPolicy>>,
    ) -> Result<Self, DomainError> {
        if scope.workspace.is_empty() || scope.session.is_empty() || scope.epoch == 0 {
            return Err(DomainError::PermissionDenied);
        }
        if let CatalogScope::Workspace(workspace) = &catalog_scope
            && workspace != &scope.workspace
        {
            return Err(DomainError::PermissionDenied);
        }
        let selection = manager.select_capabilities(registry, catalog_scope, selected)?;
        Ok(Self {
            generation: None,
            manager,
            scope,
            selection,
            policy,
        })
    }
    pub fn wire_names(&self) -> Vec<String> {
        self.selection.wire_names()
    }
    pub fn schema_bytes(&self) -> usize {
        self.selection.schema_bytes()
    }
    pub fn validate(&self, scope: &Scope, registry: &Registry) -> Result<(), DomainError> {
        if scope.workspace != self.scope.workspace
            || scope.session != self.scope.session
            || scope.epoch != self.scope.epoch
        {
            return Err(DomainError::PermissionDenied);
        }
        self.check_generation()?;
        self.selection.rebind_registry(&self.manager, registry)?;
        Ok(())
    }
    fn check_generation(&self) -> Result<(), DomainError> {
        if let Some((revision, expected)) = &self.generation
            && revision.load(std::sync::atomic::Ordering::Acquire) != *expected
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    fn check(&self, context: AdapterContext<'_>) -> Result<RuntimeCheck, DomainError> {
        self.validate(context.scope, context.registry)?;
        let journal: Arc<ToolJournal> = context.journal.cloned().ok_or_else(|| {
            DomainError::Unavailable("MCP requires an owned native session journal".into())
        })?;
        let scope = context.scope.clone();
        let deadline = context.deadline;
        let policy = self.policy.clone();
        let revision = context.policy_revision;
        let generation = self.generation.clone();
        let check: RuntimeCheck = Arc::new(move || {
            if let Some((revision, expected)) = &generation
                && revision.load(std::sync::atomic::Ordering::Acquire) != *expected
            {
                return Err(DomainError::Conflict);
            }
            if Instant::now() >= deadline {
                return Err(DomainError::TimedOut);
            }
            journal.check_mcp_scope(&scope)?;
            if let Some(policy) = &policy
                && policy.snapshot()?.revision != revision
            {
                return Err(DomainError::Conflict);
            }
            Ok(())
        });
        check()?;
        Ok(check)
    }
    pub(super) fn prepare(
        &self,
        call: &ValidatedCall,
        context: AdapterContext<'_>,
    ) -> Result<PreparedOperation, DomainError> {
        self.check(context)?;
        let selection = self
            .selection
            .rebind_registry(&self.manager, context.registry)?;
        self.manager.prepare_operation(&selection, context.registry, call).map(|plan|plan.allow_input(!matches!(context.policy_mode, ag1_core::tools::policy::Mode::Planning|ag1_core::tools::policy::Mode::Headless)))
    }
    pub(super) fn execute(
        &self,
        plan: PreparedOperation,
        ticket: AuthorizedCall,
        context: AdapterContext<'_>,
        artifacts: &mut impl ExecutionArtifacts,
    ) -> Result<AdapterOutcome, DomainError> {
        let check = match self.check(context) {
            Ok(check) => check,
            Err(error) => {
                return Ok(crate::mcp::operation::failure(
                    crate::mcp::operation::error_code(&error, false),
                    false,
                ));
            }
        };
        let target = plan.target_revision();
        plan.execute(
            ticket,
            context.registry,
            ExecutionFence {
                scope: context.scope,
                policy_revision: context.policy_revision,
                target_revision: &target,
                now: context.clock.now_ms(),
            },
            context.clock,
            artifacts,
            Some(check),
        )
    }
}
