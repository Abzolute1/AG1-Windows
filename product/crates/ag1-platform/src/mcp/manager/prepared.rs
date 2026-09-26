//! Reservation shared by the standalone service and native session executor.
use super::{Control, Manager, Selection, execution::Command};
use crate::mcp::{catalog::Tool, operation};
use ag1_core::{
    Clock, DomainError,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, ExecutionArtifacts},
        registry::{Registry, ValidatedCall},
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

pub(crate) type RuntimeCheck = Arc<dyn Fn() -> Result<(), DomainError> + Send + Sync>;
pub(crate) struct PreparedOperation {
    input_allowed: bool,
    control: Arc<Control>,
    sender: SyncSender<Command>,
    tool: Option<Tool>,
    call: ValidatedCall,
    wait: Duration,
}
impl Drop for PreparedOperation {
    fn drop(&mut self) {
        self.control.busy.store(false, Ordering::Release);
    }
}
impl Manager {
    pub(crate) fn prepare_operation(
        &self,
        selection: &Selection,
        registry: &Registry,
        call: &ValidatedCall,
    ) -> Result<PreparedOperation, DomainError> {
        selection.validate_call(self, registry, call)?;
        let bytes: usize = registry
            .definitions()
            .map(|d| d.metadata.to_string().len())
            .sum();
        if bytes > 4 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "MCP execution registry snapshot".into(),
            ));
        }
        let identity = call.mcp_identity().ok_or(DomainError::PermissionDenied)?;
        let state = self.state()?;
        let (connection, _) = selection
            .tools
            .get(call.name())
            .ok_or(DomainError::PermissionDenied)?;
        let worker = state.live.get(connection).ok_or(DomainError::Conflict)?;
        let view = worker
            .control
            .view
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        let tool = view
            .catalog
            .as_ref()
            .ok_or(DomainError::Conflict)?
            .tools
            .iter()
            .find(|t| t.wire_name() == call.name())
            .cloned();
        if identity.kind == ag1_core::tools::mcp::Kind::Tool && tool.is_none() {
            return Err(DomainError::Conflict);
        }
        worker
            .control
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DomainError::QueueFull)?;
        Ok(PreparedOperation {
            input_allowed: false,
            control: worker.control.clone(),
            sender: worker.sender.clone(),
            tool,
            call: call.clone(),
            wait: self.limits.timeout() * 2 + Duration::from_secs(1),
        })
    }
}
impl PreparedOperation {
    pub(crate) fn allow_input(mut self, allowed: bool) -> Self {
        self.input_allowed=allowed && self.control.input_entry.is_some();
        if self.input_allowed {self.wait=Duration::from_secs(super::super::input::MAX_SECONDS+2);}
        self
    }
    pub(crate) fn target_revision(&self) -> String {
        self.call
            .mcp_identity()
            .expect("MCP reservation")
            .target_revision()
    }
    /// Called only from the existing executor's adapter closure after intent ack.
    pub(crate) fn execute(
        self,
        ticket: AuthorizedCall,
        registry: &Registry,
        fence: ExecutionFence<'_>,
        clock: &dyn Clock,
        artifacts: &mut impl ExecutionArtifacts,
        check: Option<RuntimeCheck>,
    ) -> Result<AdapterOutcome, DomainError> {
        let identity = self.call.mcp_identity().expect("MCP reservation").clone();
        let mut outcome = self.execute_inner(ticket, registry, fence, clock, artifacts, check)?;
        outcome.result["details"]["mcp"]["identity"] = serde_json::json!(identity);
        Ok(outcome)
    }
    fn execute_inner(
        self,
        ticket: AuthorizedCall,
        registry: &Registry,
        fence: ExecutionFence<'_>,
        clock: &dyn Clock,
        artifacts: &mut impl ExecutionArtifacts,
        check: Option<RuntimeCheck>,
    ) -> Result<AdapterOutcome, DomainError> {
        ticket.check(registry, fence)?;
        let call = ticket.call();
        if call.operation() != self.call.operation()
            || call.call_id() != self.call.call_id()
            || call.name() != self.call.name()
            || call.args_hash() != self.call.args_hash()
            || call.mcp_identity() != self.call.mcp_identity()
            || call.mcp_reviewed_policy() != self.call.mcp_reviewed_policy()
        {
            return Err(DomainError::PermissionDenied);
        }
        if let Some(check) = &check
            && let Err(error) = check()
        {
            return Ok(operation::failure(
                operation::error_code(&error, false),
                false,
            ));
        }
        let (reply, receive) = mpsc::sync_channel(1);
        let sent = Arc::new(AtomicBool::new(false));
        let command = Command {
            input_allowed: self.input_allowed,
            ticket,
            registry: registry.clone(),
            scope: fence.scope.clone(),
            policy_revision: fence.policy_revision,
            now: clock.now_ms(),
            started: Instant::now(),
            check: check.clone(),
            sent: sent.clone(),
            reply,
        };
        if self.sender.try_send(command).is_err() {
            return Ok(operation::failure("not_sent", false));
        }
        let deadline = Instant::now() + self.wait;
        let mut runtime_error = None;
        let result = loop {
            if runtime_error.is_none()
                && let Some(check) = &check
                && let Err(error) = check()
            {
                runtime_error = Some(error);
                self.control.cancel();
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err(mpsc::RecvTimeoutError::Timeout);
            }
            match receive.recv_timeout(remaining.min(Duration::from_millis(10))) {
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                result => break result,
            }
        };
        let admitted = sent.load(Ordering::Acquire);
        // No private response is projected after its live runtime/scope is revoked.
        if let Some(check) = &check
            && let Err(error) = check()
        {
            runtime_error = Some(error);
        }
        if let Some(error) = runtime_error {
            return Ok(operation::failure(
                operation::error_code(&error, admitted),
                admitted,
            ));
        }
        if self.control.cancelled.load(Ordering::Acquire) {
            return Ok(operation::failure("cancelled", admitted));
        }
        if self.control.credential_check().is_err() {
            return Ok(operation::failure("permission_denied", admitted));
        }
        match result {
            Ok(Ok(envelope)) => {
                let outcome = operation::retain_result(
                    artifacts, self.call.mcp_identity().expect("MCP reservation"),
                    self.tool.as_ref(), self.call.call_id(), envelope,
                )?;
                // Bounded decoding/storage may outlast revocation. Historical
                // custody stays scoped, but the now-private projection is withheld.
                if self.control.cancelled.load(Ordering::Acquire) {
                    return Ok(operation::failure("cancelled", admitted));
                }
                if let Err(error) = self.control.credential_check().and_then(|_| check.as_ref().map_or(Ok(()), |check|check())) {
                    return Ok(operation::failure(operation::error_code(&error,admitted),admitted));
                }
                Ok(outcome)
            },
            Ok(Err(error)) => Ok(operation::failure(
                operation::error_code(&error, admitted),
                admitted,
            )),
            Err(_) => {
                self.control.cancel();
                Ok(operation::failure("worker_interrupted", admitted))
            }
        }
    }
}
