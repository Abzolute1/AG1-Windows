//! Isolated delegated sessions reuse native persistence, tool execution and the
//! parent's endpoint scheduler. Caller drop cancels; accepted work stays owned
//! until its checkpoint/result has been retained and its resources have drained.
use super::*;
use ag1_core::{
    SessionId,
    runtime::subagent::{
        self as domain, Profile,
        checkpoint::{Checkpoint, Manager, Owner, State},
        tracking::Tracking,
    },
    storage::StoragePort,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Instant;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub operation: String,
    pub description: String,
    #[serde(rename = "type")]
    pub profile: Profile,
    pub prompt: String,
    pub model: Option<String>,
    pub max_turns: Option<u64>,
    pub timeout: Option<f64>,
    pub token_budget: Option<u64>,
    pub resume_from: Option<String>,
}
pub struct Services {
    pub tools: Arc<files::NativeToolRunner>,
    pub provider: Arc<dyn ProviderResolver>,
    /// Host-resolved and credential-configured candidates only.
    pub models: Vec<Value>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultView {
    pub state: State,
    #[serde(rename = "type")]
    pub profile: Profile,
    pub description: String,
    pub model: String,
    pub summary: String,
    pub partial: bool,
    pub evidence: Vec<domain::checkpoint::Evidence>,
    pub usage: Value,
    pub turns: u64,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub checkpoint_id: String,
    /// Durable original transcript/evidence custody; never copied into parent context.
    pub child_session: String,
    pub unavailable_tools: Vec<String>,
    pub settled: bool,
    pub result_set: Value,
    /// Revision references from the child's recorded read results, not summary prose.
    pub source_references: Vec<String>,
}
pub struct Delegation {
    cancel: Arc<Control>,
    task: Option<tokio::task::JoinHandle<Result<ResultView, DomainError>>>,
}
/// Coalesced advisory progress. It never acknowledges a parent tool outcome or
/// substitutes for checkpoint/result persistence.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub turn: u64,
    pub tool_calls: u64,
    pub tokens: u64,
    pub elapsed_s: u64,
    pub last_action: String,
    pub model: String,
}
#[derive(Clone)]
pub struct Cancellation(Arc<Control>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.cancel.store(true, Ordering::Release);
    }
    pub fn progress(&self) -> Result<Progress, DomainError> {
        let mut progress = self
            .0
            .progress
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clone();
        progress.elapsed_s = self.0.started.elapsed().as_secs();
        Ok(progress)
    }
}
impl Delegation {
    pub fn cancellation(&self) -> Cancellation {
        Cancellation(self.cancel.clone())
    }
    pub fn cancel(&self) {
        self.cancel.cancel.store(true, Ordering::Release);
    }
    pub async fn finish(mut self) -> Result<ResultView, DomainError> {
        self.task
            .take()
            .ok_or(DomainError::Conflict)?
            .await
            .map_err(|_| DomainError::Unavailable("delegated task owner failed".into()))?
    }
}
impl Drop for Delegation {
    fn drop(&mut self) {
        self.cancel();
    }
}
pub(super) struct Control {
    pub cancel: AtomicBool,
    pub done: AtomicBool,
    progress: Mutex<Progress>,
    started: Instant,
    _slot: tokio::sync::OwnedSemaphorePermit,
}
pub(super) struct Retained {
    runtime: Arc<NativeSession>,
    _control: Arc<Control>,
    tools: Arc<GuardedTools>,
}
struct Completion(Arc<Control>);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.done.store(true, Ordering::Release);
    }
}
struct Checkpoints {
    manager: Manager,
    tracking: Tracking,
    budget: u64,
    at: String,
    control: Arc<Control>,
}
impl Checkpoints {
    fn capture(
        &mut self,
        child: &NativeSession,
        final_state: Option<State>,
    ) -> Result<(), DomainError> {
        let branch = child.state()?.branch().to_vec();
        if branch.len() == self.tracking.seen && final_state.is_none() {
            return Ok(());
        }
        let mut next = self.manager.acknowledged().clone();
        let mut tracking = self.tracking.clone();
        tracking.capture(&mut next, &branch, self.budget, &self.at)?;
        next.final_state = final_state;
        self.manager.commit(&*child.store, next)?;
        self.tracking = tracking;
        let mut progress = self
            .control
            .progress
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        progress.turn = self.manager.acknowledged().last_turn;
        progress.tokens = self.manager.acknowledged().usage.total_tokens;
        Ok(())
    }
    fn exhausted(&self) -> bool {
        self.manager.acknowledged().usage.total_tokens > self.budget
    }
}
struct GuardedResolver {
    child: Arc<NativeSession>,
    checkpoints: Arc<Mutex<Checkpoints>>,
    provider: Arc<dyn ProviderResolver>,
    loop_stopped: Arc<AtomicBool>,
}
impl ProviderResolver for GuardedResolver {
    fn resolve<'a>(
        &'a self,
        input: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest> {
        Box::pin(async move {
            if self.loop_stopped.load(Ordering::Acquire) {
                return Err(DomainError::LimitExceeded(
                    "subagent repetition budget".into(),
                ));
            }
            let checkpoints = self.checkpoints.clone();
            let child = self.child.clone();
            self.child
                .pool
                .submit(move || {
                    let mut checkpoints = checkpoints.lock().map_err(|_| DomainError::Conflict)?;
                    checkpoints.capture(&child, None)?;
                    if checkpoints.exhausted() {
                        return Err(DomainError::LimitExceeded("subagent token budget".into()));
                    }
                    Ok(())
                })
                .await?;
            self.provider.resolve(input, cancel).await
        })
    }
}
struct GuardedTools {
    child: Arc<NativeSession>,
    checkpoints: Arc<Mutex<Checkpoints>>,
    native: Arc<files::NativeToolRunner>,
    loop_stopped: Arc<AtomicBool>,
}
impl ToolRunner for GuardedTools {
    fn execute(
        &self,
        scope: &Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ToolSelection,
        operation: &str,
        cancel: &AtomicBool,
        revision: &AtomicU64,
    ) -> Result<Vec<ToolOutcome>, DomainError> {
        let mut outcomes = vec![];
        for (index, call) in calls.iter().enumerate() {
            {
                let mut checkpoints = self.checkpoints.lock().map_err(|_| DomainError::Conflict)?;
                checkpoints.capture(&self.child, None)?;
                if checkpoints.exhausted() {
                    cancel.store(true, Ordering::Release);
                }
                if checkpoints
                    .tracking
                    .repeats(&call.name, &call.block["arguments"])
                {
                    self.loop_stopped.store(true, Ordering::Release);
                    cancel.store(true, Ordering::Release);
                }
                let mut progress = checkpoints
                    .control
                    .progress
                    .lock()
                    .map_err(|_| DomainError::Conflict)?;
                progress.tool_calls = progress
                    .tool_calls
                    .checked_add(1)
                    .ok_or(DomainError::Conflict)?;
                progress.last_action = domain::tool::action(&call.name, &call.block["arguments"]);
            }
            let mut scope = scope.clone();
            scope.revision = revision.load(Ordering::Acquire);
            outcomes.extend(self.native.execute(
                &scope,
                std::slice::from_ref(call),
                selection,
                &format!("{operation}-child-{index}"),
                cancel,
                revision,
            )?);
            if outcomes.iter().any(|outcome| !outcome.execution_settled) {
                cancel.store(true, Ordering::Release);
            }
        }
        self.checkpoints
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .capture(&self.child, None)?;
        Ok(outcomes)
    }
    fn quiesce(&self, scope: &Scope, operation: &str) -> Result<(), DomainError> {
        self.native.quiesce(scope, operation)
    }
}
impl NativeSession {
    /// Retry resource drainage for this live child owner only. This does not
    /// repair its journal, settle unknown effects or replay a delegated task.
    pub async fn drain_subagent(&self, operation: &str) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        OperationId::new(operation)?;
        let (child, tools) = {
            let retained = self
                .retained_subagents
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            let child = retained.get(operation).ok_or(DomainError::Conflict)?;
            if !child._control.done.load(Ordering::Acquire) {
                return Err(DomainError::Unavailable(
                    "delegated execution is still draining".into(),
                ));
            }
            (child.runtime.clone(), child.tools.clone())
        };
        let scope = {
            let state = child.state()?;
            Scope {
                workspace: self.workspace.clone(),
                session: state.stamp().session().as_str().into(),
                epoch: state.stamp().epoch(),
                revision: state.journal_revision(),
            }
        };
        let operation = format!("{operation}-drain");
        let activity = self.retain_work()?;
        child
            .pool
            .submit(move || {
                let _activity = activity;
                tools.quiesce(&scope, &operation)
            })
            .await
    }
    pub(super) fn delegated_work_settled(&self) -> Result<bool, DomainError> {
        Ok(self
            .retained_subagents
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_empty()
            && !self
                .delegations
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .iter()
                .filter_map(std::sync::Weak::upgrade)
                .any(|child| !child.done.load(Ordering::Acquire)))
    }
    /// Scoped durable observation. "intent" requires inspection and never
    /// authorizes automatic restart; a completed result can be read after loss
    /// of the original caller or completion acknowledgement.
    pub async fn recorded_subagent(&self, operation: &str) -> Result<Value, DomainError> {
        OperationId::new(operation)?;
        let owner = Owner {
            workspace: self.workspace.clone(),
            session: self.state()?.stamp().session().as_str().into(),
        };
        let key = format!(
            "delegate-{:x}",
            Sha256::digest(json!([owner, operation]).to_string().as_bytes())
        );
        let store = self.store.clone();
        let operation = operation.to_owned();
        self.pool
            .submit(move || {
                let document = store
                    .load_metadata("subagent_run", &key)?
                    .ok_or(DomainError::NotFound)?;
                if document.diagnostic.is_some() {
                    return Err(DomainError::DurabilityUnknown);
                }
                let record: Value = serde_json::from_slice(&document.bytes)
                    .map_err(|_| DomainError::DurabilityUnknown)?;
                if record["version"] != 1
                    || record["owner"] != json!(owner)
                    || record["request"]["operation"] != operation
                {
                    return Err(DomainError::PermissionDenied);
                }
                Ok(record)
            })
            .await
    }
    /// Reconcile only a retained, healthy child with an already committed
    /// completion receipt. This never re-executes a tool or settles transport.
    pub async fn reconcile_subagent(&self, operation: &str) -> Result<(), DomainError> {
        let record = self.recorded_subagent(operation).await?;
        if record["state"] != "completed" || record["result"]["settled"] != true {
            return Err(DomainError::DurabilityUnknown);
        }
        let mut retained = self
            .retained_subagents
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if let Some(child) = retained.get(operation) {
            let child = &child.runtime;
            let session = child.state()?.stamp().session().as_str().to_owned();
            if session != record["childSession"]
                || !child.persistence_healthy()?
                || !child.settled()?
            {
                return Err(DomainError::DurabilityUnknown);
            }
            retained.remove(operation);
        }
        Ok(())
    }
    pub fn start_subagent(
        self: &Arc<Self>,
        request: Request,
        services: Services,
    ) -> Result<Delegation, DomainError> {
        if self.delegated {
            return Err(DomainError::PermissionDenied);
        }
        OperationId::new(&request.operation)?;
        if request.description.len() > 16 * 1024
            || request.prompt.len() > 4 * 1024 * 1024
            || request
                .timeout
                .is_some_and(|n| !n.is_finite() || n <= 0. || n > 86400.)
            || request.token_budget == Some(0)
        {
            return Err(DomainError::InvalidInput("subagent request bounds".into()));
        }
        if let Some(id) = &request.resume_from {
            domain::checkpoint_id(id)?;
        }
        let activity = self.enter_owned()?;
        // Serialize child publication with parent turn admission. Task may
        // delegate during its admitted turn; later turns must observe that work.
        let _admission = self.active.lock().map_err(|_| DomainError::Conflict)?;
        if self
            .retained_subagents
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .len()
            >= 4
        {
            return Err(DomainError::LimitExceeded("retained delegated work".into()));
        }
        let cancel = Arc::new(Control {
            cancel: AtomicBool::new(false),
            done: AtomicBool::new(false),
            progress: Mutex::new(Progress {
                last_action: "starting".into(),
                ..Default::default()
            }),
            started: Instant::now(),
            _slot: self
                .delegation_slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| {
                    DomainError::LimitExceeded("delegated work awaiting settlement".into())
                })?,
        });
        {
            let mut active = self.delegations.lock().map_err(|_| DomainError::Conflict)?;
            active.retain(|child| {
                child
                    .upgrade()
                    .is_some_and(|child| !child.done.load(Ordering::Acquire))
            });
            if active.len() >= 4 {
                return Err(DomainError::LimitExceeded("concurrent delegations".into()));
            }
            active.push(Arc::downgrade(&cancel));
        }
        let parent = self.clone();
        let task_cancel = cancel.clone();
        let task = tokio::runtime::Handle::try_current()
            .map_err(|_| DomainError::Unavailable("subagent async host".into()))?
            .spawn(async move {
                let _activity = activity;
                let _completion = Completion(task_cancel.clone());
                parent.run_subagent(request, services, task_cancel).await
            });
        Ok(Delegation {
            cancel,
            task: Some(task),
        })
    }
    async fn run_subagent(
        self: &Arc<Self>,
        request: Request,
        services: Services,
        cancel: Arc<Control>,
    ) -> Result<ResultView, DomainError> {
        let start = Instant::now();
        // Parent Task retains its tool worker. This fixed child lane avoids
        // nested pool deadlock; all inference shares the parent's scheduler.
        let work_pool = self
            .delegation_pool
            .get_or_init(|| BlockingPool::new(2, 32).map(Arc::new))
            .clone()?;
        let timeout = Duration::from_secs_f64(
            request
                .timeout
                .unwrap_or(domain::DEFAULT_TIMEOUT_SECONDS as f64)
                .max(1.),
        );
        let deadline = start.checked_add(timeout).ok_or(DomainError::Conflict)?;
        if cancel.cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let mut configuration = self.configuration()?;
        configuration.model = domain::routing::select(
            request.profile,
            request.model.as_deref(),
            &configuration.model,
            &services.models,
        )?;
        if configuration.model["reasoning"] != true {
            configuration.thinking = "off".into();
        }
        let owner = Owner {
            workspace: self.workspace.clone(),
            session: self.state()?.stamp().session().as_str().into(),
        };
        let hash = format!(
            "{:x}",
            Sha256::digest(json!([owner, request.operation]).to_string().as_bytes())
        );
        let session = format!("subagent-{hash}");
        let checkpoint_id = format!("ckpt_{hash}");
        let timestamp_ms = services.tools.clock.now_ms();
        let timestamp = chrono::DateTime::from_timestamp_millis(
            i64::try_from(timestamp_ms)
                .map_err(|_| DomainError::InvalidInput("subagent clock".into()))?,
        )
        .ok_or_else(|| DomainError::InvalidInput("subagent clock".into()))?
        .to_rfc3339();
        let mut adapters = services.tools.adapters.clone();
        // Parent MCP leases belong to its session epoch. Child profiles do not
        // inherit them, and child cleanup must never close a parent's lease.
        #[cfg(target_os = "linux")]
        { adapters.mcp=None; adapters.mcp_discovery=None; adapters.mcp_management=None; }
        adapters.delegation = None;
        adapters.extensions = None;
        // Share host launch configuration, with an isolated durable job book
        // and fresh process ownership. Readonly profiles never receive jobs.
        adapters.jobs = if request.profile.tools().contains(&"job") {
            adapters
                .jobs
                .as_ref()
                .map(|jobs| {
                    jobs.isolated(
                        Scope {
                            workspace: self.workspace.clone(),
                            session: session.clone(),
                            epoch: 1,
                            revision: 0,
                        },
                        &format!("child-jobs-{}-{hash}", self.incarnation),
                    )
                })
                .transpose()?
                .map(Arc::new)
        } else {
            None
        };
        if request.profile.readonly() {
            adapters.shell = adapters
                .shell
                .as_ref()
                .map(|shell| shell.readonly())
                .transpose()?
                .map(Arc::new);
        }
        let available = adapters
            .available_names()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let names = request
            .profile
            .tools()
            .into_iter()
            .filter(|name| available.contains(*name))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let unavailable_tools = request
            .profile
            .tools()
            .into_iter()
            .filter(|name| !available.contains(*name))
            .map(str::to_owned)
            .collect();
        let mut registry = ag1_core::tools::registry::Registry::builtins()?;
        registry.activate(&names, &available)?;
        configuration.tools = registry
            .native_tools()
            .into_iter()
            .filter(|tool| names.iter().any(|name| tool["name"] == *name))
            .collect();
        configuration.prompt = None;
        configuration.system_prompt = registry.build_prompt(&ag1_core::context::prompt::PromptInput {
            date:timestamp[..10].into(),cwd:self.root.root().to_str().ok_or(DomainError::PermissionDenied)?.into(),
            custom_prompt:Some(format!("A parent agent delegated a self-contained task to you. It sees only your final result, so include everything needed to assess your findings. MCP connections and private server capabilities belong to the parent session and are unavailable here. If the task needs them, report the required operation to the parent; do not recreate connections or seek credentials. {}",request.profile.instructions())),
            selected_tools:Some(names),model_provider:configuration.model["provider"].as_str().map(str::to_owned),context_window:configuration.model["contextWindow"].as_u64(),..Default::default()
        })?;
        configuration.validate()?;
        cancel
            .progress
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .model = format!(
            "{}/{}",
            configuration.model["provider"]
                .as_str()
                .ok_or(DomainError::Conflict)?,
            configuration.model["id"]
                .as_str()
                .ok_or(DomainError::Conflict)?
        );
        let store = self.store.clone();
        let root = self.root.clone();
        let pool = work_pool.clone();
        let scheduler = self.scheduler.clone();
        let http = self.http.clone();
        let req = request.clone();
        let timestamp_clone = timestamp.clone();
        let session_clone = session.clone();
        let operation_key = format!("delegate-{hash}");
        let checkpoint_control = cancel.clone();
        let (child,checkpoints,prompt,intent_revision) = work_pool.submit(move || {
            let timestamp = timestamp_clone;
            let session = session_clone;
            let prior = req.resume_from.as_ref().map(|id| Manager::load(&*store,owner.clone(),id)).transpose()?;
            let prompt = prior.map(|prior| prior.acknowledged().resume_prompt(&req.prompt)).transpose()?.unwrap_or(req.prompt.clone());
            // A repeated request identity cannot start a second child after a
            // crash or lost acknowledgement. Reconciliation reads this record.
            let intent_revision = store.commit_metadata("subagent_run",&operation_key,None,&json!({"version":1,"owner":owner,"request":req,"childSession":session,"checkpointId":checkpoint_id,"model":configuration.model,"state":"intent"}).to_string().into_bytes())?;
            let mut key = [0u8;32];
            std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut key)).map_err(crate::io_error)?;
            let header = json!({"type":"session","version":3,"id":session,"cwd":root.root(),"timestamp":timestamp}).to_string().into_bytes();
            store.bootstrap_session(ag1_storage::SessionBootstrap { command:&operation_key,session:&session,origin:&session,cursor_key:&key,seed:ag1_storage::SessionSeed::Fresh { header:&header,records:&[] } }).map_err(DomainError::from)?;
            let journal = store.journal_snapshot(&SessionId::new(&session)?)?;
            let state = ag1_core::runtime::SessionRuntime::resume(SessionId::new(&session)?,1,&journal,configuration)?;
            let manager = Manager::create(&*store,owner,Checkpoint::new(checkpoint_id,req.profile,req.description.clone(),timestamp.clone())?)?;
            let mut child = NativeSession::new(state,store,session,root,pool,scheduler,http)?;
            child.delegated = true;
            Ok((Arc::new(child),Arc::new(Mutex::new(Checkpoints { manager,tracking:Default::default(),budget:req.token_budget.unwrap_or(domain::DEFAULT_TOKEN_BUDGET),at:timestamp,control:checkpoint_control })),prompt,intent_revision))
        }).await?;
        let native = Arc::new(files::NativeToolRunner {
            files: services.tools.files.clone(),
            adapters,
            registry: Arc::new(registry),
            policy: request.profile.policy(&services.tools.policy),
            policy_revision: services.tools.policy_revision,
            approvals: services.tools.approvals.clone(),
            store: self.store.clone(),
            origin: session.clone(),
            timestamp: timestamp.clone(),
            clock: services.tools.clock.clone(),
            tool_timeout: services.tools.tool_timeout.min(timeout),
        });
        let guarded = Arc::new(GuardedTools {
            child: child.clone(),
            checkpoints: checkpoints.clone(),
            native,
            loop_stopped: Arc::new(AtomicBool::new(false)),
        });
        self.retained_subagents
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .insert(
                request.operation.clone(),
                Retained {
                    runtime: child.clone(),
                    _control: cancel.clone(),
                    tools: guarded.clone(),
                },
            );
        let resolver = GuardedResolver {
            child: child.clone(),
            checkpoints: checkpoints.clone(),
            provider: services.provider,
            loop_stopped: guarded.loop_stopped.clone(),
        };
        let future = child.run(TurnInput { command:request.operation.clone(),message:json!({"role":"user","content":[{"type":"text","text":prompt}],"timestamp":timestamp_ms}),timestamp:timestamp.clone(),max_generations:Some(domain::max_turns(request.max_turns)),compaction:ag1_core::context::compaction::Settings { enabled:false,..Default::default() },compaction_timeout_ms:900000,retry:Default::default() },&resolver,guarded.clone(),&|_:&SessionEvent|{});
        tokio::pin!(future);
        let mut timed_out = Instant::now() >= deadline;
        let outcome = if cancel.cancel.load(Ordering::Acquire) || timed_out {
            Err(DomainError::Cancelled)
        } else {
            loop {
                tokio::select! {
                    result = &mut future => break result,
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        timed_out |= Instant::now()>=deadline;
                        if timed_out || cancel.cancel.load(Ordering::Acquire) { child.cancel()?; }
                    }
                }
            }
        };
        let final_capture = checkpoints.clone();
        let final_child = child.clone();
        let final_tools = guarded.clone();
        let quiesce_operation = format!("{}-quiesce", request.operation);
        work_pool
            .submit(move || {
                let scope = {
                    let state = final_child.state()?;
                    Scope {
                        workspace: final_child.workspace.clone(),
                        session: state.stamp().session().as_str().into(),
                        epoch: state.stamp().epoch(),
                        revision: state.journal_revision(),
                    }
                };
                // Drain resources even when the terminal journal/checkpoint
                // append failed. Retained owns these handles if custody fails.
                final_tools.quiesce(&scope, &quiesce_operation)?;
                final_capture
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .capture(&final_child, None)
            })
            .await?;
        let exhausted = checkpoints
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .exhausted();
        let state = if timed_out {
            State::Timeout
        } else if exhausted
            || guarded.loop_stopped.load(Ordering::Acquire)
            || matches!(&outcome,Err(DomainError::LimitExceeded(message)) if message == "caller generation budget" || message == "subagent token budget")
        {
            State::Budget
        } else if outcome.is_err() || cancel.cancel.load(Ordering::Acquire) {
            State::Failed
        } else {
            State::Completed
        };
        let error = outcome.as_ref().err().map(|error| format!("{error:?}"));
        let child_copy = child.clone();
        let checkpoints_copy = checkpoints.clone();
        let store = self.store.clone();
        let operation_key = format!("delegate-{hash}");
        let completed_operation = request.operation.clone();
        let result = work_pool.submit(move || {
            let mut checkpoints = checkpoints_copy.lock().map_err(|_| DomainError::Conflict)?;
            checkpoints.capture(&child_copy,Some(state))?;
            let checkpoint = checkpoints.manager.acknowledged();
            let mut source_references = Vec::new();
            let branch = ag1_core::task::branch_values(&child_copy.store.journal_snapshot(&SessionId::new(&session)?)?)?;
            for entry in branch.iter().rev().take(128) {
                let message = &entry["message"];
                if message["role"] == "toolResult" && ag1_core::tools::surface::message_tool(message) == "read" && message["isError"] != true {
                    if let Some(reference) = message["details"]["sourceReference"].as_str() {
                        if ag1_core::evidence::SourceReference::parse(reference).is_ok() && !source_references.contains(&reference.to_owned()) { source_references.push(reference.to_owned()); }
                    }
                }
                if source_references.len() == 8 { break; }
            }
            let result = ResultView { source_references, result_set:domain::tool::result_set(checkpoint),state,profile:request.profile,description:request.description,model:format!("{}/{}",child_copy.configuration()?.model["provider"].as_str().unwrap(),child_copy.configuration()?.model["id"].as_str().unwrap()),summary:checkpoints.tracking.summary.clone(),partial:state != State::Completed && (!checkpoints.tracking.summary.is_empty() || !checkpoint.evidence.is_empty()),evidence:checkpoint.evidence.iter().rev().take(20).rev().map(|entry| entry.evidence.clone()).collect(),usage:json!({"input":checkpoint.usage.input,"output":checkpoint.usage.output,"totalTokens":checkpoint.usage.total_tokens,"cost":checkpoint.usage.cost,"cacheRead":checkpoints.tracking.cache_read,"cacheWrite":checkpoints.tracking.cache_write}),turns:checkpoint.last_turn,duration_ms:start.elapsed().as_millis().min(u64::MAX as u128) as u64,error,checkpoint_id:checkpoint.id.clone(),child_session:session,unavailable_tools,settled:child_copy.settled()? };
            let intent = store.load_metadata("subagent_run",&operation_key)?.ok_or(DomainError::DurabilityUnknown)?;
            let mut record:Value = serde_json::from_slice(&intent.bytes).map_err(|_| DomainError::DurabilityUnknown)?;
            record["state"] = json!("completed");
            record["result"] = json!(&result);
            store.commit_metadata("subagent_run",&operation_key,Some(&intent_revision),&record.to_string().into_bytes())?;
            Ok(result)
        }).await?;
        if result.settled {
            self.retained_subagents
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .remove(&completed_operation);
        }
        Ok(result)
    }
}
