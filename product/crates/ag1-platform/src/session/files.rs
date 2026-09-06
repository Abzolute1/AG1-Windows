//! P09 native tool execution for the session loop. Read-only runs execute in
//! bounded groups; mutations form ordered barriers and retain FileService's
//! per-file admission permit through approval and publication.
use super::{
    ToolOutcome, ToolRunner, ToolSelection,
    adapters::{AdapterContext, NativeAdapters, PreparedAdapter},
};
use crate::file_tools::FileService;
use ag1_core::{
    Clock, DomainError, SessionId,
    context::features::Features,
    runtime::session::PendingCall,
    tools::{
        approval::{
            ApprovalRequest, AuthorizedCall, Authorizer, ExecutionFence, Preflight,
            PreflightContext,
        },
        executor::{
            self, AdapterOutcome, Effect, Execution, ExecutionArtifacts, Hooks, Invocation,
            Rejection,
        },
        policy::Policy,
        registry::{Registry, Scope},
        result::ResultBudget,
    },
};
use ag1_storage::{Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub trait ApprovalHandler: Send + Sync {
    fn supports_commands(&self) -> bool {
        false
    }
    fn pending(&self) -> Result<Vec<ApprovalRequest>, DomainError> {
        Ok(Vec::new())
    }
    fn answer(&self, _: &ag1_core::tools::approval::Binding, _: bool) -> Result<(), DomainError> {
        Err(DomainError::Unavailable("approval commands".into()))
    }
    fn approve(&self, request: &ApprovalRequest, cancel: &AtomicBool) -> Result<bool, DomainError>;
}
pub struct RejectApproval;
impl ApprovalHandler for RejectApproval {
    fn approve(&self, _: &ApprovalRequest, _: &AtomicBool) -> Result<bool, DomainError> {
        Ok(false)
    }
}
#[derive(Clone)]
pub struct NativeToolRunner {
    pub files: Arc<FileService>,
    pub adapters: NativeAdapters,
    pub registry: Arc<Registry>,
    pub policy: Policy,
    pub policy_revision: u64,
    pub approvals: Arc<dyn ApprovalHandler>,
    pub store: Arc<Store>,
    pub origin: String,
    pub timestamp: String,
    pub clock: Arc<dyn Clock>,
    /// Explicit host tool deadline, independent of provider reasoning limits.
    pub tool_timeout: Duration,
}
pub type FileRunner = NativeToolRunner;

enum Prepared {
    Ready {
        plan: Box<PreparedAdapter>,
        authorization: Box<AuthorizedCall>,
        deadline: Instant,
    },
    Rejected(DomainError),
}
struct Work {
    call: PendingCall,
    operation: String,
    prepared: Prepared,
}
#[derive(Clone, Copy)]
struct Current<'a> {
    uncertain: bool,
    registry: &'a Registry,
    scope: &'a Scope,
    cancel: &'a AtomicBool,
    revision: &'a AtomicU64,
    selection: &'a ToolSelection,
}
impl Current<'_> {
    fn check(self) -> Result<(), DomainError> {
        if let Some(journal) = &self.selection.journal {
            journal.service_controls()?;
        }
        if self.uncertain {
            Err(DomainError::DurabilityUnknown)
        } else if self.cancel.load(Ordering::Acquire) {
            Err(DomainError::Cancelled)
        } else if self.revision.load(Ordering::Acquire) != self.scope.revision
            || self
                .selection
                .journal
                .as_ref()
                .is_some_and(|journal| !journal.valid())
        {
            Err(DomainError::Conflict)
        } else {
            Ok(())
        }
    }
}
impl NativeToolRunner {
    fn prepare(
        &self,
        current: Current<'_>,
        call: &PendingCall,
        allowed: bool,
        serialized: bool,
        operation: String,
        authorizer: &mut Authorizer,
    ) -> Work {
        let scope = current.scope;
        let cancel = current.cancel;
        let prepared = (|| {
            current.check()?;
            if !allowed {
                return Err(DomainError::PermissionDenied);
            }
            if serialized {
                return Err(DomainError::InvalidInput("Controlled runtime: multiple tool calls in one assistant turn are blocked. Wait for the previous tool result and continue with exactly one next action.".into()));
            }
            if cancel.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if let Some(controlled) = current
                .selection
                .journal
                .as_ref()
                .and_then(|j| j.controlled.as_ref())
            {
                let args = &call.block["arguments"];
                let path = super::controlled::path(args)
                    .map(|path| self.files.relative_path(path))
                    .transpose()?;
                let decision = controlled
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .evaluate(&call.name, args, path.as_deref())?;
                if let Some(reason) = decision.rejection() {
                    return Err(DomainError::InvalidInput(reason));
                }
            }
            let mut valid = current.registry.prepare(
                &operation,
                &call.id,
                &call.name,
                &call.block["arguments"],
                call.block["_argumentsTruncated"] != true,
            )?;
            let deadline = Instant::now()
                .checked_add(self.tool_timeout)
                .ok_or(DomainError::Conflict)?;
            if let Some(extensions) = &self.adapters.extensions
                && executor::recover(&*self.store, &scope.workspace, &scope.session, &operation)?.is_none() {
                let journal = current.selection.journal.as_deref().ok_or_else(|| DomainError::Unavailable("extension hooks require the native journal".into()))?;
                if valid.origin()==ag1_core::tools::registry::Origin::Extension {
                    let args=extensions.prepare_tool(journal,current.registry,scope,&valid,&crate::resource_fs::ScanControl{cancel,deadline})?;
                    valid=current.registry.prepare(&operation,&call.id,&call.name,&args,true)?;
                }
                let result = extensions.tool_event(journal, current.registry, scope,
                    json!({"type":"tool_call","toolName":valid.name(),"toolCallId":valid.call_id(),"input":valid.args()}),
                    &crate::resource_fs::ScanControl { cancel, deadline })?;
                if result.as_ref().is_some_and(|result| result["block"] == true) { return Err(DomainError::PermissionDenied); }
                current.check()?;
            }
            let plan = self.adapters.prepare(
                &self.files,
                &valid,
                AdapterContext {
                    journal: current.selection.journal.as_ref(),
                    registry: current.registry,
                    scope,
                    policy_revision: self.policy_revision,
                    clock: &*self.clock,
                    profile: &current.selection.research_profile,
                    loaded_memory: &current.selection.loaded_memory,
                    cancel,
                    deadline,
                },
            )?;
            let valid = plan.transformed_call().cloned().unwrap_or(valid);
            let target = plan.target_revision();
            let authorization = match authorizer.preflight(
                valid,
                PreflightContext {
                    scope,
                    policy: &self.policy,
                    policy_revision: self.policy_revision,
                    target_revision: &target,
                    now: self.clock.now_ms(),
                    ttl: self
                        .tool_timeout
                        .as_millis()
                        .min(300000)
                        .try_into()
                        .map_err(|_| DomainError::Conflict)?,
                    untrusted_extensions: true,
                },
            )? {
                Preflight::Ready(authorization) => authorization,
                Preflight::Rejected(reason) => return Err(DomainError::InvalidInput(reason)),
                Preflight::Approval(request) => {
                    let approved = self.approvals.approve(&request, cancel)?;
                    current.check()?;
                    if cancel.load(Ordering::Acquire) {
                        return Err(DomainError::Cancelled);
                    }
                    authorizer
                        .answer(
                            &request.binding,
                            approved,
                            current.registry,
                            ExecutionFence {
                                scope,
                                policy_revision: self.policy_revision,
                                target_revision: &target,
                                now: self.clock.now_ms(),
                            },
                        )?
                        .ok_or(DomainError::PermissionDenied)?
                }
            };
            if let Some(journal) = &current.selection.journal {
                let (reason, events) = journal.repetition.before(
                    &call.name,
                    &call.id,
                    authorization.call().args(),
                    self.clock.now_ms(),
                )?;
                journal.repetition_events(events);
                if let Some(reason) = reason {
                    return Err(DomainError::InvalidInput(reason));
                }
            }
            Ok(Prepared::Ready {
                plan: Box::new(plan),
                authorization: Box::new(authorization),
                deadline,
            })
        })()
        .unwrap_or_else(Prepared::Rejected);
        Work {
            call: call.clone(),
            operation,
            prepared,
        }
    }
    fn execute_one(
        &self,
        current: Current<'_>,
        mut work: Work,
    ) -> Result<ToolOutcome, DomainError> {
        let scope = current.scope;
        let cancel = current.cancel;
        if matches!(work.prepared, Prepared::Ready { .. })
            && let Err(error) = current.check()
        {
            work.prepared = Prepared::Rejected(error);
        }
        let mut artifacts = ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            &self.origin,
            &work.operation,
            &self.timestamp,
            Features::default(),
        )?;
        let shell = matches!(&work.prepared, Prepared::Ready {plan,..} if matches!(plan.as_ref(),PreparedAdapter::Shell {..}));
        let mut native_settled = false;
        let outcome = match work.prepared {
            Prepared::Rejected(reason) => executor::reject(
                &*self.store,
                &mut artifacts,
                Invocation {
                    operation: &work.operation,
                    call_id: &work.call.id,
                    name: &work.call.name,
                    arguments: &work.call.block["arguments"],
                    complete: work.call.block["_argumentsTruncated"] != true,
                },
                Rejection {
                    scope,
                    reason: &reason,
                    clock: &*self.clock,
                    budget: ResultBudget::default(),
                },
                |_| {},
            ),
            Prepared::Ready {
                plan,
                authorization,
                deadline,
            } => {
                let target = plan.target_revision().to_owned();
                executor::execute_with_artifacts(
                    &*self.store,
                    &mut artifacts,
                    *authorization,
                    Execution {
                        registry: current.registry,
                        fence: ExecutionFence {
                            scope,
                            policy_revision: self.policy_revision,
                            target_revision: &target,
                            now: self.clock.now_ms(),
                        },
                        budget: ResultBudget::default(),
                        clock: &*self.clock,
                    },
                    &mut RuntimeHooks {
                        registry: current.registry,
                        journal: current.selection.journal.as_deref(),
                        clock: &*self.clock,
                        extensions: self.adapters.extensions.as_deref(),
                        scope,
                        cancel,
                        deadline,
                    },
                    |authorization, artifacts| {
                        if let Err(error) = current.check() {
                            return Ok(AdapterOutcome {
                                effect: Effect::None,
                                result: json!({"content":[{"type":"text","text":format!("Tool cancelled before execution: {error:?}")}],"isError":true,"details":{"executionEffect":"none","rejectedBeforeExecution":true}}),
                                artifact_page: false,
                            });
                        }
                        let may_mutate = plan.may_mutate();
                        match (*plan).execute(
                            authorization,
                            AdapterContext {
                                journal: current.selection.journal.as_ref(),
                                registry: current.registry,
                                scope,
                                policy_revision: self.policy_revision,
                                clock: &*self.clock,
                                profile: &current.selection.research_profile,
                                loaded_memory: &current.selection.loaded_memory,
                                cancel,
                                deadline,
                            },
                            artifacts,
                        ) {
                            Ok(outcome) => {
                                native_settled = true;
                                Ok(outcome)
                            }
                            Err(error) => Ok(AdapterOutcome {
                                effect: if may_mutate {
                                    Effect::Unknown
                                } else {
                                    Effect::None
                                },
                                result: json!({"content":[{"type":"text","text":format!("Tool execution failed: {error:?}{}",if may_mutate {". Inspect the target before retrying."} else {""})}],"isError":true,"details":{"executionEffect":if may_mutate {"unknown"}else{"none"}}}),
                                artifact_page: false,
                            }),
                        }
                    },
                    |_| {},
                )
            }
        }?;
        let mut execution_settled = native_settled || outcome.record.effect != Effect::Unknown;
        if outcome.replayed && shell && !execution_settled {
            let native = artifacts.read_json(
                outcome
                    .record
                    .native_result
                    .as_ref()
                    .ok_or(DomainError::DurabilityUnknown)?,
            )?;
            execution_settled = matches!(
                native["details"]["termination"].as_str(),
                Some("exit" | "signal" | "cancelled" | "timeout" | "output_limit")
            ) && native["details"]["rawOutputArtifact"]["id"].is_string();
        }
        Ok(ToolOutcome {
            outcome,
            execution_settled,
        })
    }
}
impl ToolRunner for NativeToolRunner {
    fn supports_planning(&self) -> bool {
        true
    }
    fn planning_tools(
        &self,
        allow_inspection: bool,
    ) -> Result<(Arc<dyn ToolRunner>, Value), DomainError> {
        let mut runner = self.clone();
        runner.adapters.planning = true;
        runner.policy.mode = ag1_core::tools::policy::Mode::Planning;
        runner.policy.planning_read_only_tools =
            Some(if allow_inspection { "allow" } else { "block" }.into());
        if !runner
            .policy
            .trusted_tools
            .iter()
            .any(|s| s == "submit_plan")
        {
            runner.policy.trusted_tools.push("submit_plan".into());
        }
        runner.policy_revision = runner
            .policy_revision
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let submit_tool = runner
            .registry
            .public_tools()
            .into_iter()
            .find(|t| t["name"] == "submit_plan")
            .ok_or(DomainError::NotFound)?;
        Ok((Arc::new(runner), submit_tool))
    }

    fn approval_handler(&self) -> Option<Arc<dyn ApprovalHandler>> {
        self.approvals
            .supports_commands()
            .then(|| self.approvals.clone())
    }
    fn supports_controlled_policy(&self) -> bool {
        true
    }
    fn job_notices(
        &self,
        scope: &Scope,
        operation: &str,
    ) -> Result<Vec<ag1_core::tools::jobs::Notice>, DomainError> {
        let Some(jobs) = &self.adapters.jobs else {
            return Ok(Vec::new());
        };
        let mut artifacts = ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            &self.origin,
            operation,
            &self.timestamp,
            Features::default(),
        )?;
        jobs.refresh_notices(scope, &mut artifacts)
    }
    fn acknowledge_job_notice(&self, scope: &Scope, id: &str) -> Result<(), DomainError> {
        self.adapters
            .jobs
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("background job notices".into()))?
            .acknowledge_scoped_notice(scope, id)
    }
    fn handles_session_event(&self, event: &str) -> bool {
        self.adapters
            .extensions
            .as_ref()
            .is_some_and(|extensions| extensions.handles(event))
    }
    fn discover_session_resources(
        &self,
        scope: &Scope,
        snapshot: &Value,
    ) -> Result<Option<ag1_core::runtime::prompt::PromptConfiguration>, DomainError> {
        let Some(extensions) = &self.adapters.extensions else {
            return Ok(None);
        };
        if self.tool_timeout.is_zero() || self.tool_timeout > Duration::from_secs(86400) {
            return Err(DomainError::InvalidInput(
                "extension callback deadline".into(),
            ));
        }
        let cancel = AtomicBool::new(false);
        let control = crate::resource_fs::ScanControl {
            cancel: &cancel,
            deadline: Instant::now()
                .checked_add(self.tool_timeout)
                .ok_or(DomainError::Conflict)?,
        };
        let mut snapshot = snapshot.clone();
        snapshot["allTools"] = json!(self.registry.public_tools());
        extensions.discover_resources(
            scope,
            &snapshot,
            (*self.registry).clone(),
            &self.timestamp,
            &control,
        )
    }
    fn session_event(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
    ) -> Result<Option<Value>, DomainError> {
        self.request_event(scope, snapshot, event, &AtomicBool::new(false))
    }
    fn request_event(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
        cancel: &AtomicBool,
    ) -> Result<Option<Value>, DomainError> {
        let Some(extensions) = &self.adapters.extensions else {
            return Ok(None);
        };
        if self.tool_timeout.is_zero() || self.tool_timeout > Duration::from_secs(86400) {
            return Err(DomainError::InvalidInput(
                "extension callback deadline".into(),
            ));
        }
        let deadline = Instant::now()
            .checked_add(self.tool_timeout)
            .ok_or(DomainError::Conflict)?;
        let mut snapshot = snapshot.clone();
        snapshot["allTools"] = json!(self.registry.public_tools());
        extensions.event_with_snapshot(
            scope,
            &snapshot,
            event,
            &crate::resource_fs::ScanControl { cancel, deadline },
            || Ok(()),
        )
    }
    fn quiesce(&self, scope: &Scope, operation: &str) -> Result<(), DomainError> {
        if let Some(jobs) = &self.adapters.jobs {
            let mut artifacts = ContextStore::new(
                &self.store,
                SessionId::new(&scope.session)?,
                &self.origin,
                operation,
                &self.timestamp,
                Features::default(),
            )?;
            jobs.quiesce(scope, &mut artifacts)?;
        }
        if let Some(extensions) = &self.adapters.extensions {
            extensions.stop()?;
        }
        Ok(())
    }
    fn execute(
        &self,
        scope: &Scope,
        calls: &[PendingCall],
        selection: &ToolSelection,
        operation: &str,
        cancel: &AtomicBool,
        current_revision: &AtomicU64,
    ) -> Result<Vec<ToolOutcome>, DomainError> {
        if calls.len() > 128
            || self.tool_timeout.is_zero()
            || self.tool_timeout > Duration::from_secs(86400)
        {
            return Err(DomainError::InvalidInput("native tool batch limits".into()));
        }
        let mut registry = (*self.registry).clone();
        let available: std::collections::BTreeSet<_> =
            self.adapters.available_names().into_iter().collect();
        let names = selection
            .names
            .iter()
            .filter(|name| {
                available.contains(*name)
                    && registry
                        .definitions()
                        .any(|definition| definition.metadata["name"] == **name)
            })
            .cloned()
            .collect::<Vec<_>>();
        registry.activate(&names, &available)?;
        let mut authorizer = Authorizer::new(operation)?;
        let mut outputs = vec![];
        let mut index = 0;
        let mut uncertain = false;
        while index < calls.len() {
            let mut current_scope = scope.clone();
            if let Some(journal) = &selection.journal {
                current_scope.revision = journal.revision()?;
            }
            let current = Current {
                uncertain,
                registry: &registry,
                scope: &current_scope,
                cancel,
                revision: current_revision,
                selection,
            };
            let eligible = |name: &str| matches!(name, "read" | "ls" | "grep" | "find" | "diff");
            let controlled = selection
                .journal
                .as_ref()
                .is_some_and(|j| j.controlled.is_some());
            let end = if !controlled
                && self.adapters.extensions.is_none()
                && eligible(&calls[index].name)
            {
                (index..calls.len())
                    .take(8)
                    .take_while(|i| eligible(&calls[*i].name))
                    .last()
                    .unwrap()
                    + 1
            } else {
                index + 1
            };
            // Policy/preflight runs in source order. No later mutation captures
            // a file before preceding work has drained and released its permit.
            let work = (index..end)
                .map(|i| {
                    self.prepare(
                        current,
                        &calls[i],
                        selection.names.contains(&calls[i].name),
                        i > 0
                            && selection
                                .journal
                                .as_ref()
                                .is_some_and(|j| j.serialize_controlled),
                        format!("{operation}-{i}"),
                        &mut authorizer,
                    )
                })
                .collect::<Vec<_>>();
            let completed = if work.len() == 1 {
                vec![self.execute_one(current, work.into_iter().next().unwrap())]
            } else {
                std::thread::scope(|threads| {
                    let handles = work
                        .into_iter()
                        .map(|work| threads.spawn(move || self.execute_one(current, work)))
                        .collect::<Vec<_>>();
                    // Always join EVERY accepted worker, including after an
                    // earlier error. Vec ordering follows calls, not completion.
                    handles
                        .into_iter()
                        .map(|handle| {
                            handle.join().unwrap_or_else(|_| {
                                Err(DomainError::Io("native tool worker panicked".into()))
                            })
                        })
                        .collect::<Vec<_>>()
                })
            };
            for result in completed {
                let result = result?;
                if let Some(journal) = &selection.journal {
                    journal.publish_result(&result.outcome)?;
                    if let Some(completion) = &journal.completion {
                        let artifacts = ContextStore::new(
                            &self.store,
                            SessionId::new(&scope.session)?,
                            &self.origin,
                            &result.outcome.record.operation,
                            &self.timestamp,
                            Features::default(),
                        )?;
                        super::completion_evidence::observe(
                            completion,
                            &artifacts,
                            &result.outcome,
                        )?;
                    }
                    if let Some(controlled) = &journal.controlled {
                        let artifacts = ContextStore::new(
                            &self.store,
                            SessionId::new(&scope.session)?,
                            &self.origin,
                            &result.outcome.record.operation,
                            &self.timestamp,
                            Features::default(),
                        )?;
                        super::controlled::observe(
                            controlled,
                            &self.files,
                            &artifacts,
                            &result.outcome,
                        )?;
                    }
                }
                uncertain |= !result.execution_settled;
                outputs.push(result);
            }
            index = end;
        }
        Ok(outputs)
    }
}

struct RuntimeHooks<'a> {
    registry: &'a Registry,
    extensions: Option<&'a super::extensions::NativeExtensions>,
    scope: &'a Scope,
    cancel: &'a AtomicBool,
    deadline: Instant,
    journal: Option<&'a super::tool_journal::ToolJournal>,
    clock: &'a dyn Clock,
}
impl Hooks for RuntimeHooks<'_> {
    fn after(
        &mut self,
        call: &ag1_core::tools::registry::ValidatedCall,
        result: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        let mut extension_error = None;
        let mut projected = if let (Some(extensions), Some(journal)) =
            (self.extensions, self.journal)
        {
            let mut event = json!({"type":"tool_result","toolName":call.name(),"toolCallId":call.call_id(),"input":call.args(),"content":result["content"],"isError":result["isError"]==true});
            if let Some(details) = result.get("details") {
                event["details"] = details.clone();
            }
            let mut scope = self.scope.clone();
            scope.revision = journal.revision()?;
            extensions
                .tool_event(
                    journal,
                    self.registry,
                    &scope,
                    event,
                    &crate::resource_fs::ScanControl {
                        cancel: self.cancel,
                        deadline: self.deadline,
                    },
                )
                .unwrap_or_else(|error| {
                    extension_error = Some(error);
                    None
                })
                .map(|patch| {
                    let mut projection = result.clone();
                    for key in ["content", "details", "isError"] {
                        if let Some(value) = patch.get(key) {
                            projection[key] = value.clone();
                        }
                    }
                    projection
                })
        } else {
            None
        };
        if let Some(journal) = self.journal {
            journal.repetition.after(
                call.name(),
                call.call_id(),
                call.args(),
                result,
                self.clock.now_ms(),
            )?;
            let content = &result["content"];
            let text = match content.as_str() {
                Some(text) => std::borrow::Cow::Borrowed(text),
                None => std::borrow::Cow::Owned(
                    content
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|block| block["type"] == "text")
                        .filter_map(|block| block["text"].as_str())
                        .collect::<String>(),
                ),
            };
            let guidance = journal
                .failures
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .observe(call.name(), result["isError"] == true, &text)?;
            if let Some(guidance) = guidance {
                // P09 retained the native result before this hook. Only the
                // model-facing result gains guidance; evidence stays original.
                let projection = projected.get_or_insert_with(|| result.clone());
                let mut blocks = projection["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                blocks.push(json!({"type":"text","text":guidance}));
                projection["content"] = json!(blocks);
            }
        }
        if let Some(error) = extension_error {
            return Err(error);
        }
        Ok(projected)
    }
}
