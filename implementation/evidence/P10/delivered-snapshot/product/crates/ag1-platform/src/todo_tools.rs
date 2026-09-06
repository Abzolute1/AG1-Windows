//! Checklist mutations borrow the active native session's journal owner.
use crate::{file_tools::FileExecution, session::tool_journal::ToolJournal, workspace::Workspace};
use ag1_core::{
    DomainError,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Scope, ValidatedCall},
    },
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
fn rejected(error: DomainError) -> AdapterOutcome {
    AdapterOutcome {
        result: serde_json::json!({"content":[{"type":"text","text":format!("Todo tool failed: {error:?}")}],"isError":true,"details":{"executionEffect":"none"}}),
        effect: Effect::None,
        artifact_page: false,
    }
}
pub struct TodoTools {
    workspace: Arc<Workspace>,
}
pub struct PreparedTodo {
    call: ValidatedCall,
    target: String,
    journal: Arc<ToolJournal>,
}
impl PreparedTodo {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    pub fn may_mutate(&self) -> bool {
        self.call.args()["clear"] == true
            || self.call.args().get("todos").is_some()
            || self.call.args().get("planAction").is_some()
    }
}
impl TodoTools {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self { workspace }
    }
    fn scope(&self, scope: &Scope, call: &ValidatedCall) -> Result<(), DomainError> {
        if scope.workspace != self.workspace.id().as_str()
            || call.name() != "todo"
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
    ) -> Result<PreparedTodo, DomainError> {
        self.scope(scope, call)?;
        let journal = journal
            .ok_or_else(|| {
                DomainError::Unavailable("Todo tool requires the session journal owner".into())
            })?
            .clone();
        journal.snapshot(scope)?;
        Ok(PreparedTodo {
            call: call.clone(),
            target: format!(
                "todo:{}:{}:{}:{}",
                scope.workspace, scope.session, scope.epoch, scope.revision
            ),
            journal,
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedTodo,
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
            check()?;
            Ok(call)
        })();
        let call = match prepared {
            Ok(call) => call,
            Err(error) => return Ok(rejected(error)),
        };
        let (result, changed) = match plan.journal.apply_todo(scope, &call) {
            Ok(output) => output,
            Err(super::session::tool_journal::JournalMutationError::BeforeEffect(error)) => {
                return Ok(rejected(error));
            }
            Err(super::session::tool_journal::JournalMutationError::Persistence(error)) => {
                return Err(error);
            }
        };
        let effect = if changed {
            Effect::Committed
        } else {
            Effect::None
        };

        Ok(AdapterOutcome {
            result,
            effect,
            artifact_page: false,
        })
    }
}
