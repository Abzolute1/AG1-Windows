//! Task-state mutations borrow the active native session's journal owner.
use crate::{file_tools::FileExecution, session::tool_journal::ToolJournal, workspace::Workspace};
use ag1_core::{
    DomainError,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Scope, ValidatedCall},
        task_state,
    },
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
fn rejected(error: DomainError) -> AdapterOutcome {
    AdapterOutcome {
        result: serde_json::json!({"content":[{"type":"text","text":format!("Task tool failed: {error:?}")}],"isError":true,"details":{"executionEffect":"none"}}),
        effect: Effect::None,
        artifact_page: false,
    }
}
pub struct TaskTools {
    workspace: Arc<Workspace>,
}
pub struct PreparedTask {
    call: ValidatedCall,
    target: String,
    journal: Arc<ToolJournal>,
}
impl PreparedTask {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    pub fn may_mutate(&self) -> bool {
        self.call.args()["action"] != "show"
    }
}
impl TaskTools {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
    fn scope(&self, scope: &Scope, call: &ValidatedCall) -> Result<(), DomainError> {
        if scope.workspace != self.workspace.id().as_str()
            || call.name() != "taskState"
            || call.origin() != Origin::BuiltIn
        {
            Err(DomainError::PermissionDenied)
        } else {
            Ok(())
        }
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        journal: Option<&Arc<ToolJournal>>,
    ) -> Result<PreparedTask, DomainError> {
        self.scope(scope, call)?;
        let journal = journal
            .ok_or_else(|| {
                DomainError::Unavailable("Task tool requires the session journal owner".into())
            })?
            .clone();
        journal.snapshot(scope)?;
        Ok(PreparedTask {
            call: call.clone(),
            target: format!(
                "task:{}:{}:{}:{}",
                scope.workspace, scope.session, scope.epoch, scope.revision
            ),
            journal,
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedTask,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<AdapterOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        let prepared = (|| {
            self.scope(scope, authorization.call())?;
            let check = || {
                if cancel.load(Ordering::Acquire) {
                    Err(DomainError::Cancelled)
                } else if Instant::now() >= deadline {
                    Err(DomainError::TimedOut)
                } else {
                    Ok(())
                }
            };
            check()?;
            let call = authorization.consume(
                registry,
                ExecutionFence {
                    scope,
                    policy_revision,
                    target_revision: &plan.target,
                    now: now(),
                },
            )?;
            if call.args_hash() != plan.call.args_hash()
                || call.operation() != plan.call.operation()
                || call.call_id() != plan.call.call_id()
            {
                return Err(DomainError::Conflict);
            }
            let branch = plan.journal.snapshot(scope)?;
            let id = format!(
                "ts_{}",
                &ag1_storage::digest(call.operation().as_bytes())[..8]
            );
            let output = task_state::prepare(&branch, call.args(), plan.journal.timestamp(), &id)?;
            check()?;
            Ok((call, output))
        })();
        let (call, output) = match prepared {
            Ok(output) => output,
            Err(error) => {
                return Ok(rejected(error));
            }
        };
        let effect = if let Some(next) = output.next {
            match plan
                .journal
                .commit_state(scope, &call, "task_state", next.value().clone())
            {
                Ok(()) => (),
                Err(super::session::tool_journal::JournalMutationError::BeforeEffect(error)) => {
                    return Ok(rejected(error));
                }
                Err(super::session::tool_journal::JournalMutationError::Persistence(error)) => {
                    return Err(error);
                }
            }
            Effect::Committed
        } else {
            Effect::None
        };
        Ok(AdapterOutcome {
            result: output.result,
            effect,
            artifact_page: false,
        })
    }
}
