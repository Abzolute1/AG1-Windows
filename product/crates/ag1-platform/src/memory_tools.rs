//! Approved native memory tools share the P07 workspace namespace and CAS.
//! They never append session journal records behind the runtime's mutation owner.
use crate::{file_tools::FileExecution, source::SourceService, workspace::Workspace};
use ag1_core::{
    DomainError,
    evidence::{Freshness, memory_sources},
    memory::{MemoryService, MemoryStore},
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        memory,
        registry::{Origin, Scope, ValidatedCall},
    },
};
use ag1_storage::Store;
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};

pub struct MemoryTools {
    workspace: Arc<Workspace>,
    store: Arc<Store>,
}
pub struct PreparedMemory {
    call: ValidatedCall,
    expected: Option<String>,
    memory: MemoryStore,
    target: String,
}
impl PreparedMemory {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    pub fn may_mutate(&self) -> bool {
        !matches!(self.call.args()["action"].as_str(), Some("list" | "search"))
    }
}
impl MemoryTools {
    pub fn new(workspace: Arc<Workspace>, store: Arc<Store>) -> Self {
        Self { workspace, store }
    }
    fn scope(&self, call: &ValidatedCall, scope: &Scope) -> Result<(), DomainError> {
        if scope.workspace != self.workspace.id().as_str()
            || call.name() != "memory"
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
    ) -> Result<PreparedMemory, DomainError> {
        self.scope(call, scope)?;
        let (memory, expected) =
            MemoryService::load(self.store.as_ref(), self.workspace.id().as_str())?;
        let target = format!(
            "memory:{}:{}",
            self.workspace.id().as_str(),
            expected.as_deref().unwrap_or("absent")
        );
        Ok(PreparedMemory {
            call: call.clone(),
            memory,
            expected,
            target,
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedMemory,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
        loaded: &BTreeSet<String>,
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
            self.scope(authorization.call(), scope)?;
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
            let (_, current) =
                MemoryService::load(self.store.as_ref(), self.workspace.id().as_str())?;
            if current != plan.expected {
                return Err(DomainError::Conflict);
            }
            let sources = std::cell::RefCell::new(SourceService::new(self.workspace.clone()));
            let output = memory::prepare(
                &plan.memory,
                call.args(),
                loaded,
                now()
                    .try_into()
                    .map_err(|_| DomainError::InvalidInput("memory timestamp".into()))?,
                |entry| {
                    memory_sources(entry, |source| {
                        if source.starts_with("file:") {
                            sources.borrow_mut().validate(source).status
                        } else {
                            Freshness::Unverified
                        }
                    })
                },
            )?;
            check()?;
            Ok::<_, DomainError>(output)
        })();
        let output = match prepared {
            Ok(output) => output,
            Err(error) => {
                return Ok(AdapterOutcome {
                    effect: Effect::None,
                    result: serde_json::json!({"content":[{"type":"text","text":format!("Memory tool failed: {error:?}")}],"isError":true,"details":{"executionEffect":"none"}}),
                    artifact_page: false,
                });
            }
        };
        let effect = if let Some(next) = output.next {
            MemoryService::mutate(
                self.store.as_ref(),
                self.workspace.id().as_str(),
                plan.expected.as_deref(),
                |_| Ok(next),
            )?;
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
