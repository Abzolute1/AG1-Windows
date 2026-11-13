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
    pub input_allowed: bool,
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
    pub fn input(
        &self,
        control: &Control,
        envelope: &Value,
        round: usize,
    ) -> Result<Value, DomainError> {
        if !self.input_allowed {
            return Err(DomainError::PermissionDenied);
        }
        let entry = control
            .input_entry
            .as_ref()
            .ok_or(DomainError::PermissionDenied)?;
        self.admit_continuation(control)?;
        let envelope = control.private_result(Ok(envelope.clone()))?;
        let round = crate::mcp::input::Round::parse(
            &envelope,
            self.identity(),
            self.ticket.call().call_id(),
            self.args(),
            round,
        )?;
        struct Waiting<'a>(&'a AtomicBool);
        impl Drop for Waiting<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        control.waiting_input.store(true, Ordering::Release);
        let _waiting = Waiting(&control.waiting_input);
        let answer = entry.enter(
            &round.form,
            &crate::resource_fs::ScanControl {
                cancel: &control.cancelled,
                deadline: self.started
                    + std::time::Duration::from_secs(crate::mcp::input::MAX_SECONDS),
            },
        )?;
        self.admit_continuation(control)?;
        round.respond(answer)
    }
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
        self.check_send(control, false)
    }
    pub fn admit_continuation(&self, control: &Control) -> Result<(), DomainError> {
        self.check_send(control, true)
    }
    fn check_send(&self, control: &Control, continuation: bool) -> Result<(), DomainError> {
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
        if if continuation {
            !self.sent.load(Ordering::Acquire)
        } else {
            self.sent.swap(true, Ordering::AcqRel)
        } {
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
        self.execute_tool_with_input(
            store,
            artifacts,
            selection,
            authorization,
            execution,
            hooks,
            on_event,
            false,
        )
    }
    /// Native host opt-in for an interactive caller. Session adapters derive this
    /// permission from the actual execution mode; headless/planning never opt in.
    #[allow(clippy::too_many_arguments)]
    pub fn execute_tool_with_input(
        &self,
        store: &impl StoragePort,
        artifacts: &mut impl ExecutionArtifacts,
        selection: &Selection,
        authorization: AuthorizedCall,
        execution: Execution<'_>,
        hooks: &mut impl Hooks,
        on_event: impl FnMut(ToolEvent),
        input_allowed: bool,
    ) -> Result<Outcome, DomainError> {
        let plan = self
            .prepare_operation(selection, execution.registry, authorization.call())?
            .allow_input(input_allowed);
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
