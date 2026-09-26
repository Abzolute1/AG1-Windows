//! Blocking service API for native execution workers. Renderer/model dispatch
//! cannot submit raw requests; only the ordinary executor supplies a ticket.
use super::{ConnectionHealth, Control, Manager, Selection};

use ag1_core::{
    DomainError,
    storage::StoragePort,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{self, Execution, ExecutionArtifacts, Hooks, Outcome, ToolEvent},
        registry::{Registry, Scope},
    },
};
use serde_json::Value;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
    },
    time::Instant,
};

pub(super) struct Command {
    pub(super) ticket: AuthorizedCall,
    pub(super) registry: Registry,
    pub(super) scope: Scope,
    pub(super) policy_revision: u64,
    pub(super) now: u64,
    pub(super) started: Instant,
    pub check: Option<super::prepared::RuntimeCheck>,
    pub sent: Arc<AtomicBool>,
    pub reply: SyncSender<Result<Value, DomainError>>,
}
impl Command {
    pub fn identity(&self) -> &ag1_core::tools::mcp::Identity {
        self.ticket
            .call()
            .mcp_identity()
            .expect("admitted MCP identity")
    }
    pub fn args(&self) -> &Value {
        self.ticket.call().args()
    }
    pub fn admit_send(&self, control: &Control) -> Result<(), DomainError> {
        // Linearizes send admission against disable/edit/disconnect. Revocation
        // after this point cancels an in-flight call, whose effects are unknown.
        // Never hold this lock while waiting for network or process I/O.
        let _gate = control
            .send_gate
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if control.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        control.credential_check()?;
        let view = control.view.lock().map_err(|_| DomainError::Conflict)?;
        if view.status.health != ConnectionHealth::Ready || view.status.inspection_only {
            return Err(DomainError::PermissionDenied);
        }
        let catalog = view.catalog.as_ref().ok_or(DomainError::Conflict)?;
        let identity = self.identity();
        if !catalog.matches(
            identity,
            control
                .lease
                .as_ref()
                .and_then(|l| l.execution_scope.as_ref()),
        ) {
            return Err(DomainError::Conflict);
        }
        if let Some(check) = &self.check {
            check()?;
        }
        let now = self
            .now
            .saturating_add(self.started.elapsed().as_millis().min(u64::MAX as u128) as u64);
        self.ticket.check(
            &self.registry,
            ExecutionFence {
                scope: &self.scope,
                policy_revision: self.policy_revision,
                target_revision: self.ticket.target_revision(),
                now,
            },
        )?;
        if self.sent.swap(true, Ordering::AcqRel) {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}
impl Manager {
    /// Run off the UI/session owner thread. One call per connection; additional
    /// submissions fail before intent. This service entry and the session adapter
    /// share PreparedOperation; neither executes outside the ordinary executor.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_tool(
        &self,
        store: &impl StoragePort,
        artifacts: &mut impl ExecutionArtifacts,
        selection: &Selection,
        authorization: AuthorizedCall,
        execution: Execution<'_>,
        hooks: &mut impl Hooks,
        on_event: impl FnMut(ToolEvent),
    ) -> Result<Outcome, DomainError> {
        let plan = self.prepare_operation(selection, execution.registry, authorization.call())?;
        let registry = execution.registry;
        let fence = execution.fence;
        let clock = execution.clock;
        executor::execute_with_artifacts(
            store,
            artifacts,
            authorization,
            execution,
            hooks,
            |ticket, artifacts| plan.execute(ticket, registry, fence, clock, artifacts, None),
            on_event,
        )
    }
}
