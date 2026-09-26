//! Native session composition. Storage/context work uses the bounded executor;
//! providers use P06 transport and the host's shared endpoint scheduler. This
//! module is independent of desktop, terminal and the optional TypeScript host.
use crate::{
    executor::BlockingPool,
    provider::{
        generation::{GenerationRequest, request_generation},
        http::{CancelSource, ProviderHttp, cancellation},
    },
};
use ag1_core::{
    DomainError, OperationId, PortFuture,
    context::compiler::Compilation,
    inference::{GenerationPermit, Scheduler, Ticket},
    provider::{ProviderEvent, StopReason, assistant_message},
    runtime::{
        SessionRuntime,
        session::{AppendAction, Configuration, PendingCall},
    },
    tools::{executor::Outcome, registry::Scope},
};
use ag1_storage::Store;
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

pub mod adapters;
pub mod approvals;
mod callbacks;
pub mod commands;
pub mod compaction;
mod completion_evidence;
mod context;
mod controlled;
pub mod extensions;
pub mod files;
mod job_notices;
mod lifecycle;
pub mod planning;
mod preparation;
pub mod print;
mod prompt;
mod recovery;
pub mod registry;
#[cfg(target_os = "linux")]
pub mod mcp;
pub mod repetition;
pub mod restart;
pub mod runtime_commands;
pub mod subagent;
pub mod subagent_tool;
mod tool_inference;
pub mod tool_journal;
pub mod tree;
pub mod verification;
mod working_set;
mod xml;
static NEXT_TURN: AtomicU64 = AtomicU64::new(1);

pub struct ProviderInput<'a> {
    pub configuration: &'a Configuration,
    pub projection: &'a Compilation,
    pub operation: &'a str,
    /// Host-owned overrides for isolated requests, never renderer credentials.
    pub options: Option<&'a Value>,
}
/// Credential-bearing request, constructed only by a host resolver. The session
/// verifies that the resolved model is the one selected at this request boundary.
pub struct ResolvedRequest {
    pub model: Value,
    pub endpoint: String,
    pub generation: GenerationRequest,
    pub timestamp_ms: u64,
}
/// Resolves request configuration and credentials without dispatching inference
/// or accepting tool effects. Dropping this preparation future must be safe;
/// isolated summary deadlines also bound resolution and scheduler queueing.
pub trait ProviderResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        input: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest>;
}
/// Executes an admitted batch through P09, returning source-ordered durable
/// receipts. Implementations own policy/preflight and concurrency; they must
/// drain accepted effects even if cancellation is requested.
pub trait ToolRunner: Send + Sync {
    #[cfg(target_os = "linux")]
    fn client_mcp(&self) -> Option<Arc<crate::mcp::service::Session>> { None }
    /// Capture an immutable native MCP selection for this one request/batch.
    #[cfg(target_os = "linux")]
    fn mcp_request_runner(&self, _scope:&Scope) -> Result<Option<Arc<dyn ToolRunner>>,DomainError> { Ok(None) }
    #[cfg(target_os = "linux")]
    fn mcp_request_definitions(&self) -> Option<Vec<ag1_core::tools::mcp::SelectedDefinition>> { None }
    /// Request-local MCP schemas are host-owned and rechecked after async work.
    #[cfg(target_os = "linux")]
    fn prepare_mcp_request(&self, _scope:&Scope, tools:&[Value]) -> Result<Vec<Value>,DomainError> { Ok(tools.to_vec()) }
    #[cfg(target_os = "linux")]
    fn validate_mcp_request(&self, _scope:&Scope, _tools:&[Value]) -> Result<(),DomainError> { Ok(()) }

    fn folder_access(&self) -> Option<(Arc<crate::folder_access::FolderAccess>, u64)> { None }
    fn client_activity(&self) -> Result<Value, DomainError> {
        Ok(serde_json::json!({}))
    }
    fn client_resources(&self) -> Option<Arc<crate::client::resource_actions::Actions>> {
        None
    }
    fn client_extensions(&self) -> Option<Arc<extensions::NativeExtensions>> {
        None
    }
    fn client_catalog(&self) -> Vec<Value> {
        vec![]
    }
    fn discover_session_resources(
        &self,
        _scope: &Scope,
        _snapshot: &Value,
    ) -> Result<Option<ag1_core::runtime::prompt::PromptConfiguration>, DomainError> {
        Ok(None)
    }
    fn handles_session_event(&self, _event: &str) -> bool {
        false
    }
    /// Lifecycle callbacks run on the bounded host worker. Snapshot inspection
    /// and vetoes do not create a second journal mutation owner.
    fn session_event(
        &self,
        _scope: &Scope,
        _snapshot: &Value,
        _event: &Value,
    ) -> Result<Option<Value>, DomainError> {
        Ok(None)
    }
    /// Request callbacks share the parent cancellation signal. They run on the
    /// bounded worker and must drain before its retained ownership is released.
    fn request_event(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
        cancel: &AtomicBool,
    ) -> Result<Option<Value>, DomainError> {
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        self.session_event(scope, snapshot, event)
    }
    fn approval_handler(&self) -> Option<Arc<dyn files::ApprovalHandler>> {
        None
    }
    /// True only for runners enforcing the host-selected native controller.
    fn supports_controlled_policy(&self) -> bool {
        false
    }
    fn supports_planning(&self) -> bool {
        false
    }
    /// Return an isolated planning policy and the host's submit_plan schema.
    /// Implementations retain the same native tool/storage capabilities.
    fn planning_tools(
        &self,
        _allow_inspection: bool,
    ) -> Result<(Arc<dyn ToolRunner>, Value), DomainError> {
        Err(DomainError::Unavailable("planning tools".into()))
    }
    /// Refresh owned background resources and return still-unacknowledged notices.
    /// The host invokes this on its bounded worker before a new user prompt's
    /// first request; returning text never acknowledges delivery.
    fn job_notices(
        &self,
        _scope: &Scope,
        _operation: &str,
    ) -> Result<Vec<ag1_core::tools::jobs::Notice>, DomainError> {
        Ok(Vec::new())
    }
    fn acknowledge_job_notice(&self, _scope: &Scope, _id: &str) -> Result<(), DomainError> {
        Err(DomainError::Unavailable("background job notices".into()))
    }
    /// Called on the bounded worker after all admitted batches drain. Runners
    /// owning background work must stop it and retain durable terminal evidence
    /// before returning. Failure prevents runtime replacement; retries must be
    /// safe. The default is for runners with no persistent background resources.
    fn quiesce(&self, _scope: &Scope, _operation: &str) -> Result<(), DomainError> {
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
    ) -> Result<Vec<ToolOutcome>, DomainError>;
}
/// Current committed tool selection and context budget, captured together with
/// the batch scope. Adapters cannot supply stale model/context configuration.
pub struct ToolSelection {
    pub journal: Option<Arc<tool_journal::ToolJournal>>,
    pub names: Vec<String>,
    pub research_profile: ag1_core::tools::research::Profile,
    /// Identities included in the admitted provider request that produced this batch.
    pub loaded_memory: std::collections::BTreeSet<String>,
}
pub struct ToolOutcome {
    pub outcome: Outcome,
    /// Native execution drained with retained output. This is independent from
    /// whether a general shell command's filesystem effects can be enumerated.
    pub execution_settled: bool,
}
pub enum SessionEvent {
    Provider(ProviderEvent),
    Committed { revision: u64, sequence: u64 },
    Recovery(Value),
}
pub trait Listener: Send + Sync {
    fn on_event(&self, event: &SessionEvent);
}
impl<F: Fn(&SessionEvent) + Send + Sync> Listener for F {
    fn on_event(&self, event: &SessionEvent) {
        self(event);
    }
}
pub struct TurnResult {
    pub generations: u64,
    pub journal_revision: u64,
    pub assistant: Value,
}
pub struct TurnInput {
    pub command: String,
    pub message: Value,
    pub timestamp: String,
    /// Optional caller-imposed generation budget. None adds no parent turn cap.
    pub max_generations: Option<u64>,
    pub compaction: ag1_core::context::compaction::Settings,
    pub compaction_timeout_ms: u64,
    pub retry: ag1_core::runtime::retry::Settings,
}
#[derive(Clone)]
enum RunOrigin {
    User,
    Continuation,
    Command(Arc<AtomicBool>, Arc<commands::SessionCommands>),
    CommandContinuation(Arc<AtomicBool>, Arc<commands::SessionCommands>),
}
struct ActiveTurn {
    cancelled: Arc<AtomicBool>,
    source: Arc<CancelSource>,
}
pub struct NativeSession {
    state: Arc<Mutex<SessionRuntime>>,
    last_context: Mutex<Value>,
    working_set: Mutex<Option<working_set::WorkingSet>>,
    workspace_snapshot: Option<String>,
    store: Arc<Store>,
    origin: String,
    workspace: String,
    root: Arc<crate::workspace::Workspace>,
    pool: Arc<BlockingPool>,
    scheduler: Scheduler,
    http: ProviderHttp,
    active: Mutex<Option<ActiveTurn>>,
    incarnation: String,
    changed: Arc<tokio::sync::Notify>,
    revision: Arc<AtomicU64>,
    transport: Mutex<Option<Ticket>>,
    waiting_for_inference: AtomicBool,
    compacting: Mutex<Option<ActiveTurn>>,
    recovery: Mutex<ag1_core::runtime::recovery::Recovery>,
    tool_owner: Mutex<Option<std::sync::Weak<tool_journal::ToolJournal>>>,
    lifecycle: Arc<Mutex<lifecycle::Lifecycle>>,
    commands_claimed: AtomicBool,
    command_owner: std::sync::OnceLock<std::sync::Weak<commands::SessionCommands>>,
    /// Set only while constructing an isolated delegated session.
    delegated: bool,
    delegations: Mutex<Vec<std::sync::Weak<subagent::Control>>>,
    retained_subagents: Mutex<std::collections::BTreeMap<String, subagent::Retained>>,
    delegation_slots: Arc<tokio::sync::Semaphore>,
    delegation_pool: std::sync::OnceLock<Result<Arc<BlockingPool>, DomainError>>,
    failures: Arc<Mutex<ag1_core::tools::recovery::FailureTracker>>,
    repetition: Arc<repetition::Guard>,
    mode: Mutex<Option<Arc<AtomicBool>>>,
    restart_required: AtomicBool,
}
struct TurnLease<'a> {
    session: &'a NativeSession,
}
struct CommitWait {
    state: Arc<Mutex<SessionRuntime>>,
    changed: Arc<tokio::sync::Notify>,
    expected_revision: u64,
    received: bool,
}
impl Drop for CommitWait {
    fn drop(&mut self) {
        if !self.received {
            if let Ok(mut state) = self.state.lock()
                && state.journal_revision() <= self.expected_revision
            {
                state.fail_persistence();
            }
            self.changed.notify_waiters();
        }
    }
}
impl Drop for TurnLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.session.active.lock()
            && let Some(turn) = active.take()
        {
            turn.cancelled.store(true, Ordering::Release);
            turn.source.cancel();
        }
        self.session.changed.notify_waiters();
        if let Ok(owner) = self.session.tool_owner.lock()
            && let Some(owner) = owner.as_ref().and_then(std::sync::Weak::upgrade)
        {
            owner.cancel_batch();
        }
    }
}
struct QueuedTicket {
    scheduler: Scheduler,
    ticket: Ticket,
    started: bool,
}
impl Drop for QueuedTicket {
    fn drop(&mut self) {
        if !self.started {
            let _ = self.scheduler.cancel(&self.ticket);
        }
    }
}
impl NativeSession {
    /// A committed, immutable journal projection for client views. No mutable
    /// runtime handle or execution authority crosses this boundary.
    pub fn client_context(&self) -> Result<Value, DomainError> {
        self.last_context
            .lock()
            .map(|v| v.clone())
            .map_err(|_| DomainError::Conflict)
    }
    pub fn client_records(&self) -> Result<Arc<Vec<Value>>, DomainError> {
        Ok(self.state()?.branch_snapshot())
    }
    pub fn new(
        state: SessionRuntime,
        store: Arc<Store>,
        origin: String,
        workspace: Arc<crate::workspace::Workspace>,
        pool: Arc<BlockingPool>,
        scheduler: Scheduler,
        http: ProviderHttp,
    ) -> Result<Self, DomainError> {
        OperationId::new(&origin)?;
        OperationId::new(workspace.id().as_str())?;
        let mut nonce = [0u8; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut nonce))
            .map_err(crate::io_error)?;
        let incarnation = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
        let revision = Arc::new(AtomicU64::new(state.journal_revision()));
        let repetition = Arc::new(repetition::Guard::new(workspace.root().to_owned()));
        let restart_required = !store
            .interrupted_commands(
                workspace.id().as_str(),
                state.stamp().session().as_str(),
                None,
                1,
            )?
            .is_empty();
        let workspace_snapshot = match &state.configuration().prompt {
            Some(prompt) => prompt.workspace_snapshot().map(str::to_owned),
            None => prompt::workspace_snapshot(&workspace)?,
        };
        Ok(Self {
            workspace_snapshot,
            last_context: Mutex::new(Value::Null),
            working_set: Mutex::new(None),
            state: Arc::new(Mutex::new(state)),
            store,
            origin,
            workspace: workspace.id().as_str().into(),
            root: workspace,
            pool,
            scheduler,
            http,
            active: Mutex::new(None),
            incarnation,
            changed: Arc::new(tokio::sync::Notify::new()),
            revision,
            transport: Mutex::new(None),
            waiting_for_inference: AtomicBool::new(false),
            compacting: Mutex::new(None),
            recovery: Mutex::new(Default::default()),
            tool_owner: Mutex::new(None),
            lifecycle: Arc::new(Mutex::new(Default::default())),
            commands_claimed: AtomicBool::new(false),
            command_owner: std::sync::OnceLock::new(),
            delegated: false,
            delegations: Mutex::new(Vec::new()),
            retained_subagents: Mutex::new(Default::default()),
            delegation_slots: Arc::new(tokio::sync::Semaphore::new(4)),
            delegation_pool: std::sync::OnceLock::new(),
            failures: Arc::new(Mutex::new(Default::default())),
            repetition,
            mode: Mutex::new(None),
            restart_required: AtomicBool::new(restart_required),
        })
    }
    fn state(&self) -> Result<MutexGuard<'_, SessionRuntime>, DomainError> {
        self.state
            .lock()
            .map_err(|_| DomainError::Unavailable("session owner poisoned".into()))
    }
    pub fn journal_revision(&self) -> Result<u64, DomainError> {
        Ok(self.state()?.journal_revision())
    }
    pub fn context_epoch(&self) -> Result<u64, DomainError> {
        Ok(self.state()?.stamp().epoch())
    }
    pub fn persistence_healthy(&self) -> Result<bool, DomainError> {
        Ok(self.state()?.persistence_healthy())
    }
    /// Captured host configuration; renderer snapshots should project only
    /// public model/tool fields rather than serialize host service inputs.
    pub fn configuration(&self) -> Result<Configuration, DomainError> {
        Ok(self.state()?.configuration().clone())
    }
    pub fn settled(&self) -> Result<bool, DomainError> {
        let active = self
            .active
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_some();
        let core = self.state()?.settled();
        let compacting = self
            .compacting
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_some();
        let mode = self
            .mode
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_some();
        Ok(!active
            && core
            && !compacting
            && !mode
            && self
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
                .any(|child| !child.done.load(Ordering::Acquire))
            && self.background_work_settled()?
            && self.pending_transport()?.is_none())
    }
    /// Host supervisor only: reconcile after terminal/owned process evidence.
    /// Elapsed time and renderer acknowledgement cannot settle this ticket.
    pub fn pending_transport(&self) -> Result<Option<Ticket>, DomainError> {
        let transport = self.transport.lock().map_err(|_| DomainError::Conflict)?;
        match transport.as_ref() {
            Some(ticket) if !self.scheduler.ticket_settled(ticket)? => Ok(Some(ticket.clone())),
            _ => Ok(None),
        }
    }
    pub fn cancel(&self) -> Result<(), DomainError> {
        for child in self
            .delegations
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            child.cancel.store(true, Ordering::Release);
        }
        let mode = self.mode.lock().map_err(|_| DomainError::Conflict)?.clone();
        if let Some(mode) = mode {
            mode.store(true, Ordering::Release);
        }
        if let Some(work) = self
            .compacting
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
        {
            work.cancelled.store(true, Ordering::Release);
            work.source.cancel();
        }
        if let Some(turn) = self
            .active
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
        {
            turn.cancelled.store(true, Ordering::Release);
            turn.source.cancel();
        }
        self.state()?.cancel();
        if let Some(owner) = self
            .tool_owner
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            owner.cancel_batch();
        }
        self.recovery
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clear(None);
        self.changed.notify_waiters();
        Ok(())
    }
    /// The command queue owns this token before the turn starts. A late cancel
    /// cannot reach a different native turn, even after the target has drained.
    fn cancel_command(&self, target: &Arc<AtomicBool>) -> Result<(), DomainError> {
        target.store(true, Ordering::Release);
        let active = self.active.lock().map_err(|_| DomainError::Conflict)?;
        let turn = active
            .as_ref()
            .filter(|turn| Arc::ptr_eq(&turn.cancelled, target));
        let compacting = self.compacting.lock().map_err(|_| DomainError::Conflict)?;
        let own_compaction = compacting
            .as_ref()
            .is_some_and(|work| Arc::ptr_eq(&work.cancelled, target));
        if turn.is_none() && !own_compaction {
            return Ok(());
        }
        if let Some(turn) = turn {
            turn.source.cancel();
        }
        if let Some(work) = compacting.as_ref() {
            work.cancelled.store(true, Ordering::Release);
            work.source.cancel();
        }
        self.state()?.cancel();
        if let Some(owner) = self
            .tool_owner
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            owner.cancel_batch();
        }
        self.recovery
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clear(None);
        self.changed.notify_waiters();
        Ok(())
    }
    fn emit(listener: &dyn Listener, event: SessionEvent) {
        // Client failures are diagnostics for the embedding; they cannot abort
        // internal persistence or strand a scheduler permit mid-response.
        let _ =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| listener.on_event(&event)));
    }
    async fn commit(
        &self,
        action: AppendAction,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        self.commit_stamped(action, listener).await.map(|_| ())
    }
    async fn commit_stamped(
        &self,
        action: AppendAction,
        listener: &dyn Listener,
    ) -> Result<ag1_core::runtime::boundary::Stamp, DomainError> {
        let mut wait = CommitWait {
            state: self.state.clone(),
            changed: self.changed.clone(),
            expected_revision: action.write().expected,
            received: false,
        };
        let owner = self
            .tool_owner
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        let action = if let Some(owner) = owner {
            match owner.enqueue_control(action) {
                Ok(receipt) => {
                    // The accepted tool worker owns acknowledgement even when
                    // the caller drops its response future.
                    wait.received = true;
                    let output = receipt.await.unwrap_or(Err(DomainError::DurabilityUnknown));
                    match output {
                        Ok((stamp, revision, sequence)) => {
                            Self::emit(listener, SessionEvent::Committed { revision, sequence });
                            return Ok(stamp);
                        }
                        Err(error) => {
                            self.state()?.fail_persistence();
                            return Err(error);
                        }
                    }
                }
                Err(action) => *action,
            }
        } else {
            action
        };
        let store = self.store.clone();
        let receipt = self
            .pool
            .submit(move || Ok(action.execute(store.as_ref())))
            .await;
        let output = self.acknowledge_stamped(receipt, listener);
        wait.received = true;
        output
    }
    async fn commit_branch(
        &self,
        branch: ag1_core::runtime::session::BranchAction,
        listener: &dyn Listener,
    ) -> Result<ag1_core::runtime::boundary::Stamp, DomainError> {
        let mut wait = CommitWait {
            state: self.state.clone(),
            changed: self.changed.clone(),
            expected_revision: self.journal_revision()?,
            received: false,
        };
        let store = self.store.clone();
        let receipt = self
            .pool
            .submit(move || Ok(branch.execute(store.as_ref())))
            .await;
        let output = self.acknowledge_stamped(receipt, listener);
        wait.received = true;
        output
    }
    async fn wait_for_commit(&self) -> Result<(), DomainError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let state = self.state()?;
                if !state.persistence_healthy() {
                    return Err(DomainError::DurabilityUnknown);
                }
                if !state.pending_commit() {
                    return Ok(());
                }
            }
            changed.await;
        }
    }
    fn acknowledge_stamped(
        &self,
        receipt: Result<ag1_core::runtime::session::CommitReceipt, DomainError>,
        listener: &dyn Listener,
    ) -> Result<ag1_core::runtime::boundary::Stamp, DomainError> {
        let receipt = match receipt {
            Ok(receipt) => receipt,
            Err(error) => {
                self.state()?.fail_persistence();
                self.changed.notify_waiters();
                return Err(error);
            }
        };
        let mut state = self.state()?;
        let acknowledged = state.acknowledge(receipt);
        self.changed.notify_waiters();
        acknowledged?;
        self.revision
            .store(state.journal_revision(), Ordering::Release);
        let event = SessionEvent::Committed {
            revision: state.journal_revision(),
            sequence: state.committed_sequence(),
        };
        let stamp = state.stamp().clone();
        drop(state);
        Self::emit(listener, event);
        Ok(stamp)
    }
    pub fn reset_tool_failures(&self) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        self.failures
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .reset();
        Ok(())
    }
    pub async fn select_model(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        model: Value,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        let action = self.state()?.select_model(command, id, timestamp, model)?;
        self.commit(action, listener).await
    }
    pub async fn select_tools(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        tools: Vec<Value>,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        let action = self.state()?.select_tools(command, id, timestamp, tools)?;
        self.commit(action, listener).await
    }
    pub async fn select_thinking(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        thinking: String,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        let action = self
            .state()?
            .select_thinking(command, id, timestamp, thinking)?;
        self.commit(action, listener).await
    }
    pub async fn append_task_state(
        &self,
        command: &str,
        id: &str,
        timestamp: &str,
        task: ag1_core::task::TaskState,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        let action = self
            .state()?
            .append_task_state(command, id, timestamp, task)?;
        self.commit(action, listener).await
    }
    pub fn waiting_for_inference(&self) -> bool {
        self.waiting_for_inference.load(Ordering::Acquire)
    }
    async fn permit(
        &self,
        endpoint: &str,
        operation: &str,
        cancelled: &AtomicBool,
    ) -> Result<(Ticket, GenerationPermit), DomainError> {
        struct Waiting<'a>(&'a AtomicBool);
        impl Drop for Waiting<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _waiting = Waiting(&self.waiting_for_inference);
        let mut queue = QueuedTicket {
            scheduler: self.scheduler.clone(),
            ticket: self
                .scheduler
                .submit(endpoint, OperationId::new(operation)?)?,
            started: false,
        };
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if let Some(permit) = self.scheduler.start(&queue.ticket)? {
                queue.started = true;
                return Ok((queue.ticket.clone(), permit));
            }
            self.waiting_for_inference.store(true, Ordering::Release);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    /// One native loop. An embedding must retain/poll this future until it drains
    /// after cancel(); dropping it can leave unresolved effects requiring reload.
    pub async fn run(
        &self,
        input: TurnInput,
        resolver: &dyn ProviderResolver,
        tools: Arc<dyn ToolRunner>,
        listener: &dyn Listener,
    ) -> Result<TurnResult, DomainError> {
        self.run_inner(input, resolver, tools, listener, RunOrigin::User, None)
            .await
    }
    /// Host-owned recovery contribution to the existing user task. This does
    /// not reset repetition or replace task instruction, and cannot settle
    /// unresolved effects. P02 controller admission remains a separate layer.
    pub async fn continue_with(
        &self,
        input: TurnInput,
        resolver: &dyn ProviderResolver,
        tools: Arc<dyn ToolRunner>,
        listener: &dyn Listener,
    ) -> Result<TurnResult, DomainError> {
        self.run_inner(
            input,
            resolver,
            tools,
            listener,
            RunOrigin::Continuation,
            None,
        )
        .await
    }
    async fn run_inner(
        &self,
        mut input: TurnInput,
        resolver: &dyn ProviderResolver,
        tools: Arc<dyn ToolRunner>,
        listener: &dyn Listener,
        origin: RunOrigin,
        mode: Option<&print::ModeLease<'_>>,
    ) -> Result<TurnResult, DomainError> {
        let _activity = self.enter()?;
        if self.commands_claimed.load(Ordering::Acquire)
            && !matches!(
                origin,
                RunOrigin::Command(..) | RunOrigin::CommandContinuation(..)
            )
        {
            return Err(DomainError::Unavailable(
                "command facade owns prompt admission".into(),
            ));
        }
        if self.restart_required.load(Ordering::Acquire) {
            return Err(DomainError::Unavailable(
                "interrupted commands require host inspection".into(),
            ));
        }
        OperationId::new(&input.command)?;
        let continuation = matches!(
            origin,
            RunOrigin::Continuation | RunOrigin::CommandContinuation(..)
        );
        let valid_role = if continuation {
            input.message["role"] == "custom"
                && input.message["display"].is_boolean()
                && input.message["customType"]
                    .as_str()
                    .is_some_and(|kind| !kind.is_empty())
        } else {
            input.message["role"] == "user"
        };
        if !valid_role || input.max_generations == Some(0) {
            return Err(DomainError::InvalidInput("turn input".into()));
        }
        let (source, mut cancel) = cancellation();
        if self.pending_transport()?.is_some() {
            return Err(DomainError::Unavailable(
                "model transport requires reconciliation".into(),
            ));
        }
        // Turn cleanup cancels its own source even after a normal terminal.
        // The encompassing mode's cancellation flag must outlive that cleanup.
        let command_queue = match &origin {
            RunOrigin::Command(_, queue) | RunOrigin::CommandContinuation(_, queue) => {
                Some(queue.clone())
            }
            _ => None,
        };
        let cancelled = match origin {
            RunOrigin::Command(cancelled, _) | RunOrigin::CommandContinuation(cancelled, _) => {
                cancelled
            }
            _ => Arc::new(AtomicBool::new(false)),
        };
        {
            let mut active = self.active.lock().map_err(|_| DomainError::Conflict)?;
            if active.is_some() {
                return Err(DomainError::Conflict);
            }
            if !self.delegated_work_settled()? {
                return Err(DomainError::Unavailable(
                    "delegated work requires settlement before another turn".into(),
                ));
            }
            let current_mode = self.mode.lock().map_err(|_| DomainError::Conflict)?.clone();
            let authorized = match (&current_mode, mode) {
                (None, None) => true,
                (Some(current), Some(lease)) => Arc::ptr_eq(current, &lease.cancelled),
                _ => false,
            };
            if !authorized {
                return Err(DomainError::Unavailable(
                    "client mode owns the current prompt".into(),
                ));
            }
            if !self.background_work_settled()? {
                return Err(DomainError::Unavailable(
                    "accepted background work is still draining".into(),
                ));
            }
            if cancelled.load(Ordering::Acquire)
                || mode.is_some_and(|lease| lease.cancelled.load(Ordering::Acquire))
            {
                return Err(DomainError::Cancelled);
            }
            let state = self.state()?;
            if continuation {
                if !state
                    .branch()
                    .iter()
                    .any(|entry| entry["message"]["role"] == "user")
                {
                    return Err(DomainError::InvalidInput(
                        "continuation requires an existing user task".into(),
                    ));
                }
                if self.repetition.stop_reason()?.is_some() {
                    return Err(DomainError::Unavailable(
                        "repetition pause requires a new user instruction".into(),
                    ));
                }
            }
            drop(state);
            self.state()?.next_turn()?;
            if !continuation {
                self.repetition.reset()?;
            }
            *active = Some(ActiveTurn {
                cancelled: cancelled.clone(),
                source: Arc::new(source),
            });
        }
        let _lease = TurnLease { session: self };
        let serial = NEXT_TURN
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| DomainError::Conflict)?;
        let prefix = format!("turn-{}-{serial}", self.incarnation);
        if !continuation && !self.delegated && tools.handles_session_event("input") {
            let mut event = json!({"type":"input","text":ag1_core::context::hygiene::content(&input.message,"\n"),
                "source":if command_queue.is_some(){"rpc"}else{"interactive"}});
            let images = callbacks::images(&input.message);
            if !images.is_empty() {
                event["images"] = json!(images);
            }
            if let Some(result) = self
                .request_callback(
                    tools.clone(),
                    event,
                    cancelled.clone(),
                    &format!("{prefix}-input"),
                    &input.timestamp,
                )
                .await?
                && callbacks::transform_input(&mut input.message, &result)?
            {
                return Ok(TurnResult {
                    generations: 0,
                    journal_revision: self.journal_revision()?,
                    assistant: Value::Null,
                });
            }
        }
        let mut start = if !continuation && !self.delegated {
            self.before_agent_start(
                tools.clone(),
                &input,
                cancelled.clone(),
                &format!("{prefix}-before-agent"),
                mode.and_then(|lease| lease.planning.as_ref()),
            )
            .await?
        } else {
            callbacks::Start::default()
        };
        let instruction = ag1_core::context::hygiene::content(&input.message, "\n");
        let action = self.state()?.append_message(
            &input.command,
            &format!(
                "{prefix}-{}",
                if continuation { "continuation" } else { "user" }
            ),
            &input.timestamp,
            input.message,
        )?;
        self.commit(action, listener).await?;
        if !continuation && !self.delegated {
            let task = self.state()?.capture_instruction(
                &format!("{prefix}-task"),
                &format!("{prefix}-task"),
                &input.timestamp,
                &instruction,
            )?;
            if let Some(action) = task {
                self.commit(action, listener).await?;
            }
            let warning = loop {
                self.wait_for_commit().await?;
                let mut state = self.state()?;
                if state.pending_commit() {
                    continue;
                }
                let names = state
                    .configuration()
                    .tools
                    .iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_owned))
                    .collect::<Vec<_>>();
                let warning = self
                    .failures
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .prepare_prompt(&names)?;
                let Some(warning) = warning else {
                    break None;
                };
                let id = format!("{prefix}-tool-failure-warning");
                let timestamp = chrono::DateTime::parse_from_rfc3339(&input.timestamp)
                    .map_err(|_| DomainError::InvalidInput("turn timestamp".into()))?
                    .timestamp_millis();
                let message = json!({"role":"custom","customType":"tool_failure_warning","content":warning.content,"display":true,"details":warning.details,"timestamp":timestamp});
                let action = state.append_message(&id, &id, &input.timestamp, message)?;
                break Some((action, warning.warnings));
            };
            if let Some((action, warnings)) = warning {
                self.commit(action, listener).await?;
                for warning in warnings {
                    let mut event =
                        serde_json::to_value(warning).map_err(|_| DomainError::Conflict)?;
                    event["type"] = json!("tool_failure_warning");
                    Self::emit(listener, SessionEvent::Recovery(event));
                }
            }
        }
        if !continuation {
            self.deliver_job_notices(
                tools.clone(),
                &prefix,
                &input.timestamp,
                &cancelled,
                listener,
            )
            .await?;
        }
        for (index, message) in std::mem::take(&mut start.messages).into_iter().enumerate() {
            let id = format!("{prefix}-before-agent-message-{index}");
            let action = self
                .state()?
                .append_message(&id, &id, &input.timestamp, message)?;
            self.commit(action, listener).await?;
        }
        let mut generations = 0u64;
        let repetition = self.repetition.clone();
        let mut preparation_serial = 0u64;
        let mut attempted_compaction = None;
        let mut retry_attempt = 0u32;
        let mut retry_replay = None;
        let mut overflow_attempted = false;
        let mut retry_restore: Option<recovery::RetryContinuation> = None;
        let mut xml_fallback = xml::Fallback::default();
        let outcome = async {
        if let Some(queue) = &command_queue {
            queue.deliver_messages(false, &cancelled, listener).await?;
        }
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if retry_replay
                .as_ref()
                .is_some_and(|stamp| self.state().is_ok_and(|state| state.stamp() != stamp))
            {
                return Err(DomainError::Conflict);
            }
            if input.max_generations.is_some_and(|max| generations >= max) {
                return Err(DomainError::LimitExceeded(
                    "caller generation budget".into(),
                ));
            }
            if self.pending_transport()?.is_some() {
                return Err(DomainError::Unavailable(
                    "model transport requires reconciliation".into(),
                ));
            }
            preparation_serial = preparation_serial
                .checked_add(1)
                .ok_or(DomainError::Conflict)?;
            let operation = format!("{prefix}-request-{preparation_serial}");
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            Self::emit(listener, SessionEvent::Recovery(json!({"type":"request_phase","phase":"preparing"})));
            let prepared = self.state()?.prepare_request();
            let mut preparation = match prepared {
                Ok(action) => preparation::RequestPreparation::new(action,self.state.clone(),self.changed.clone()),
                Err(DomainError::Unavailable(_)) => {
                    changed.await;
                    continue;
                }
                Err(error) => return Err(error),
            };
            #[cfg(target_os = "linux")]
            let mcp_scope=Scope {workspace:self.workspace.clone(),session:preparation.stamp().session().as_str().into(),epoch:preparation.stamp().epoch(),revision:preparation.journal_revision()};
            #[cfg(target_os = "linux")]
            let request_tools=tools.mcp_request_runner(&mcp_scope)?.unwrap_or_else(||tools.clone());
            #[cfg(not(target_os = "linux"))]
            let request_tools=tools.clone();
            #[cfg(target_os = "linux")]
            {
                let view=request_tools.prepare_mcp_request(&mcp_scope,&preparation.configuration().tools)?;
                if let Some(definitions)=request_tools.mcp_request_definitions() { preparation.set_mcp_tools(&definitions,view)?; }
                else if view!=preparation.configuration().tools { preparation.set_tools(view)?; }
            }
            if let Some(planning) = mode.and_then(|lease| lease.planning.as_ref())
                && let Err(error) = planning.prepare(&mut preparation)
            {
                let _ = self.state()?.admit_request(preparation.into_action(),Err(error.clone()));
                return Err(error);
            }
            if let Err(error)=start.prepare(&mut preparation) {
                let _=self.state()?.admit_request(preparation.into_action(),Err(error.clone()));
                return Err(error);
            }
            let prepared_context = self.prepare_context(&preparation,&operation,&input.timestamp, mode.and_then(|lease| lease.verification_command.as_deref()), mode.and_then(|lease| lease.controlled.as_ref()).cloned(), mode.map(|lease| lease.completion.clone()),
                tools.handles_session_event("context").then(|| (tools.clone(), cancelled.clone()))).await
                .and_then(|context| {preparation.set_system_prompt(context.system_prompt.clone())?;Ok(context)});
            let context::PreparedContext {projection,witness,loaded_memory,working_set,..} = match prepared_context {
                Ok(context) => context,
                Err(error) => {
                    let _ = self.state()?.admit_request(preparation.into_action(), Err(error.clone()));
                    if error == DomainError::Conflict {
                        continue;
                    }
                    return Err(error);
                }
            };
            let estimated = preparation.configuration().fixed_tokens()
                + projection
                    .messages
                    .iter()
                    .map(ag1_core::context::estimate_tokens)
                    .sum::<u64>();
            let window = preparation.configuration().context_limit();
            if attempted_compaction != Some(preparation.journal_revision())
                && ag1_core::context::compaction::should_compact(
                    estimated.saturating_mul(11).div_ceil(10),
                    window,
                    &input.compaction,
                    true,
                )
            {
                attempted_compaction = Some(preparation.journal_revision());
                let result = self
                    .compact_with_callbacks(
                        compaction::CompactInput {
                            command: format!("{operation}-compact"),
                            timestamp: input.timestamp.clone(),
                            settings: input.compaction.clone(),
                            focus: None,
                            reason: compaction::Reason::Threshold,
                            llm_timeout_ms: input.compaction_timeout_ms,
                        },
                        resolver,
                        tools.clone(),
                        listener,
                    )
                    .await;
                match result {
                    Ok(Some(result)) => attempted_compaction = Some(result.journal_revision),
                    Ok(None) | Err(DomainError::Conflict) => {}
                    Err(error) => return Err(error),
                }
                continue;
            }
            let resolved = tokio::select! {
                biased;
                _ = cancel.wait() => Err(DomainError::Cancelled),
                output = resolver.resolve(
                    ProviderInput {
                        configuration: preparation.configuration(),
                        projection: &projection,
                        operation: &operation,
                        options: None,
                    },
                    &cancelled,
                ) => output,
            };
            let mut resolved = match resolved {
                Ok(resolved) if resolved.model == preparation.configuration().model => resolved,
                result => {
                    let error = result.err().unwrap_or(DomainError::Conflict);
                    let _ = self.state()?.admit_request(preparation.into_action(), Err(error.clone()));
                    return Err(error);
                }
            };
            if tools.handles_session_event("before_provider_request") {
                let body=std::mem::take(&mut resolved.generation.request.body);
                let transformed=self.payload_callback(&preparation,callbacks::PayloadInput {
                    tools:tools.clone(),cancel:cancelled.clone(),body,operation:format!("{operation}-payload"),timestamp:input.timestamp.clone(),
                }).await.and_then(|(body,extra)|{preparation.account_additional_input(extra)?;Ok(body)});
                match transformed {
                    Ok(body)=>resolved.generation.request.body=body,
                    Err(error)=>{
                        let _=self.state()?.admit_request(preparation.into_action(),Err(error.clone()));
                        if error==DomainError::Conflict && retry_replay.is_none(){continue;}
                        return Err(error);
                    }
                }
            }
            let (ticket, permit) = match self
                .permit(&resolved.endpoint, &operation, &cancelled)
                .await
            {
                Ok(permit) => permit,
                Err(error) => {
                    let _ = self.state()?.admit_request(preparation.into_action(), Err(error.clone()));
                    return Err(error);
                }
            };
            let admission = {
                #[cfg(target_os = "linux")]
                if let Err(error)=request_tools.validate_mcp_request(&mcp_scope,&preparation.configuration().tools) {
                    permit.terminal()?;
                    let _=self.state()?.admit_request(preparation.into_action(),Err(error.clone()));
                    if error==DomainError::Conflict && retry_replay.is_none() {continue;}
                    return Err(error);
                }
                let current=self.context_current(witness).await;
                if current.as_ref().is_err() || current == Ok(false) {
                    permit.terminal()?;
                    let error=current.err().unwrap_or(DomainError::Conflict);
                    let _=self.state()?.admit_request(preparation.into_action(),Err(error.clone()));
                    if error==DomainError::Conflict && retry_replay.is_none(){continue;}
                    return Err(error);
                }
                let mut state = self.state()?;
                let projection = if retry_replay
                    .as_ref()
                    .is_some_and(|stamp| stamp != state.stamp())
                {
                    Err(DomainError::Conflict)
                } else {
                    Ok(projection)
                };
                let mut cache = self.working_set.lock().map_err(|_| DomainError::Conflict)?;
                let admitted = state.admit_request(preparation.into_action(), projection);
                if admitted.is_ok() {
                    *cache = working_set;
                }
                admitted
            };
            let generation = match admission {
                Ok(generation) => generation,
                Err(DomainError::Conflict) => {
                    permit.terminal()?;
                    if retry_replay.is_some() {
                        return Err(DomainError::Conflict);
                    }
                    continue;
                }
                Err(error) => {
                    permit.terminal()?;
                    return Err(error);
                }
            };
            Self::emit(listener, SessionEvent::Recovery(json!({"type":"request_phase","phase":"waiting"})));
            retry_replay = None;
            generations = generations.checked_add(1).ok_or(DomainError::Conflict)?;
            *self.transport.lock().map_err(|_| DomainError::Conflict)? = Some(ticket);
            let outcome = request_generation(
                &self.http,
                resolved.generation,
                permit,
                &mut cancel,
                |event| {
                    Self::emit(listener, SessionEvent::Provider(event.clone()));
                    Ok(())
                },
            )
            .await;
            let (raw_snapshot, raw_error) = match &outcome {
                Ok(snapshot) => (Some(snapshot), None),
                Err(failure) => (failure.partial.as_ref(), Some(&failure.error)),
            };
            // Keep the returned provider data before error normalization or
            // journal validation, while the generation still owns its barrier.
            let retained = self.retain_provider_output(
                &operation, &input.timestamp, &generation.configuration().model,
                raw_snapshot, raw_error,
            ).await;
            let output_artifact = match retained {
                Ok(artifact) => artifact,
                Err(error) => {
                    let mut state = self.state()?;
                    state.fail_persistence();
                    state.generation_terminal(&generation)?;
                    return Err(error);
                }
            };
            let (snapshot, error) = match outcome {
                Ok(snapshot) => {
                    let error = match snapshot.stop_reason {
                        Some(StopReason::Error) => Some(DomainError::Unavailable(
                            snapshot
                                .error
                                .clone()
                                .unwrap_or_else(|| "provider generation failed".into()),
                        )),
                        Some(StopReason::Aborted) => Some(DomainError::Cancelled),
                        _ => None,
                    };
                    (snapshot, error)
                }
                Err(failure) => {
                    let Some(mut partial) = failure.partial else {
                        self.state()?.generation_terminal(&generation)?;
                        return Err(failure.error);
                    };
                    partial.stop_reason = Some(if failure.error == DomainError::Cancelled {
                        StopReason::Aborted
                    } else {
                        StopReason::Error
                    });
                    partial.error = Some(format!("{:?}", failure.error));
                    (partial, Some(failure.error))
                }
            };
            let message = assistant_message(
                &snapshot,
                &generation.configuration().model,
                resolved.timestamp_ms,
            );
            retry_restore = None;
            let mut message = match message {
                Ok(message) => message,
                Err(error) => {
                    self.state()?.fail_persistence();
                    self.state()?.generation_terminal(&generation)?;
                    return Err(error);
                }
            };
            message["nativeOutputArtifact"] = json!(output_artifact);
            // SQL may finish before the control caller consumes its receipt.
            // Keep raw output in custody, then reserve its append under the same
            // lock that checks the acknowledged configuration. A new control
            // between wakeup and lock acquisition simply makes us wait again.
            let assistant_id = format!("{prefix}-assistant-{generations}");
            let (append, xml, health) = loop {
                if let Err(error) = self.wait_for_commit().await {
                    self.state()?.generation_terminal(&generation)?;
                    return Err(error);
                }
                let mut state = self.state()?;
                if state.pending_commit() {
                    continue;
                }
                // This profile decision depends only on the declared context
                // window; locality, images and reasoning do not affect rails.
                let rails = ag1_core::model::infer_capabilities(
                    state.configuration().model["contextWindow"].as_u64(), false, false, false,
                ).volume_rails();
                let health = ag1_core::runtime::health::analyze(&message, rails);
                let xml = if error.is_none()
                    && !cancelled.load(Ordering::Acquire)
                    && (!input.compaction.enabled || !ag1_core::runtime::retry::overflow(&message, generation.configuration().context_limit()))
                {
                    let names = generation.configuration().tools.iter().filter_map(|tool| tool["name"].as_str().map(str::to_owned)).collect::<Vec<_>>();
                    match xml_fallback.prepare(&mut message, &names, &operation) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            state.generation_terminal(&generation)?;
                            return Err(error);
                        }
                    }
                } else { None };
                let append = state.append_message(
                    &assistant_id,
                    &assistant_id,
                    &input.timestamp,
                    message.clone(),
                );
                state.generation_terminal(&generation)?;
                match append {
                    Ok(append) => break (append, xml, health),
                    Err(error) => {
                        state.fail_persistence();
                        return Err(error);
                    }
                }
            };
            self.commit(append, listener).await?;
            if health.severity != "normal" {
                let mut event = serde_json::to_value(health).map_err(|_| DomainError::Conflict)?;
                event["type"] = json!("degraded_turn");
                Self::emit(listener, SessionEvent::Recovery(event));
            }
            let (scope, calls, mut selection) = {
                let state = self.state()?;
                (
                    Scope {
                        workspace: self.workspace.clone(),
                        session: state.stamp().session().as_str().into(),
                        epoch: state.stamp().epoch(),
                        revision: state.journal_revision(),
                    },
                    state.pending_tools().iter().cloned().collect::<Vec<_>>(),
                    ToolSelection {
                        journal: None,
                        names: generation.configuration().tools.iter()
                            .filter_map(|tool| tool["name"].as_str().map(str::to_owned)).collect(),
                        research_profile: context::research_profile(&state)?,
                        loaded_memory,
                    },
                )
            };
            if let Some(planning) = mode.and_then(|lease| lease.planning.as_ref()) {
                selection.names = planning.names(&selection.names);
            }
            let has_calls = !calls.is_empty();
            if has_calls {
                Self::emit(listener, SessionEvent::Recovery(json!({"type":"request_phase","phase":"executing"})));
                repetition.start_turn()?;
                let failed = error.is_some()
                    || matches!(snapshot.stop_reason, Some(StopReason::Error | StopReason::Aborted));
                let batch_cancel = Arc::new(AtomicBool::new(failed || cancelled.load(Ordering::Acquire)));
                let (journal, mut journal_events, mut isolated_requests) = tool_journal::ToolJournal::new(
                    self, format!("{prefix}-result-{generations}"), input.timestamp.clone(), resolved.timestamp_ms, resolved.endpoint.clone(), batch_cancel.clone(), tool_journal::RuntimePolicies { repetition: repetition.clone(), controlled: mode.and_then(|lease| lease.controlled.as_ref()).cloned(), completion: mode.map(|lease| lease.completion.clone()), commands: command_queue.as_ref().map(Arc::downgrade) },
                )?;
                selection.journal = Some(journal.clone());
                *self.tool_owner.lock().map_err(|_| DomainError::Conflict)? = Some(Arc::downgrade(&journal));
                if cancelled.load(Ordering::Acquire) { journal.cancel_batch(); }
                let control_guard = tool_journal::ControlGuard(journal.clone());
                let runner = request_tools.clone();
                let current_revision = self.revision.clone();
                let batch_operation = format!("{prefix}-tools-{generations}");
                let expected = calls
                    .iter()
                    .map(|call| (call.id.clone(), call.name.clone(), ag1_core::tools::surface::route(&call.name, &call.block["arguments"], &[]).ok().flatten().map(|(name,_)| name)))
                    .collect::<Vec<_>>();
                let execution = self
                    .pool
                    .submit(move || {
                        let output = runner.execute(
                            &scope,
                            &calls,
                            &selection,
                            &batch_operation,
                            &batch_cancel,
                            &current_revision,
                        );
                        control_guard.0.finish_controls()?;
                        drop(control_guard);
                        output
                    });
                tokio::pin!(execution);
                let outcomes = loop {
                    tokio::select! {
                        result = &mut execution => {
                            while let Ok(event) = journal_events.try_recv() { journal.event_dequeued(&event); Self::emit(listener,event); }
                            break result?;
                        }
                        Some(event) = journal_events.recv() => { journal.event_dequeued(&event); Self::emit(listener,event); },
                        Some(request) = isolated_requests.recv() => {
                            self.run_tool_inference(&journal, request, resolver, &cancelled).await;
                        }
                    }
                };
                if outcomes.len() != expected.len()
                    || outcomes.iter().zip(&expected).any(|(terminal, (id, name, canonical))| {
                        let outcome = &terminal.outcome;
                        outcome.record.call_id != *id || (outcome.record.tool != *name
                            && !(canonical.as_ref() == Some(&outcome.record.tool) && journal.published(outcome).unwrap_or(false)))
                    })
                {
                    return Err(DomainError::Conflict);
                }
                let uncertain = outcomes.iter().any(|outcome| !outcome.execution_settled);
                for (index, terminal) in outcomes.into_iter().enumerate() {
                    let outcome = terminal.outcome;
                    if journal.published(&outcome)? { continue; }
                    let id = format!("{prefix}-result-{generations}-{index}");
                    let result = json!({"role":"toolResult","toolCallId":outcome.record.call_id,"toolName":outcome.record.tool,"content":outcome.result["content"],"details":outcome.result["details"],"isError":outcome.result["isError"].as_bool().unwrap_or(false),"timestamp":outcome.record.finished_ms.unwrap_or(resolved.timestamp_ms)});
                    let append =
                        self.state()?
                            .append_message(&id, &id, &input.timestamp, result)?;
                    self.commit(append, listener).await?;
                    let evidence_id = format!("{id}-inspection");
                    let evidence = self.state()?.capture_inspection(
                        &evidence_id, &evidence_id, &input.timestamp, &id,
                    )?;
                    if let Some(append) = evidence {
                        self.commit(append, listener).await?;
                    }
                    if let Some(mode) = mode {
                        let completion = mode.completion.clone();
                        let store = self.store.clone();
                        let origin = self.origin.clone();
                        let timestamp = input.timestamp.clone();
                        let session = self.state()?.stamp().session().clone();
                        if outcome.record.session != session.as_str() || outcome.record.workspace != self.workspace {
                            return Err(DomainError::Conflict);
                        }
                        let activity = self.retain_work()?;
                        self.pool.submit(move || {
                            let _activity = activity;
                            let artifacts = ag1_storage::context::ContextStore::new(
                                &store, session, &origin, &outcome.record.operation, &timestamp,
                                ag1_core::context::features::Features::default(),
                            )?;
                            completion_evidence::observe(&completion, &artifacts, &outcome)
                        }).await?;
                    }
                }
                if uncertain {
                    return Err(DomainError::DurabilityUnknown);
                }
            }
            if let Some(xml) = xml {
                Self::emit(listener, SessionEvent::Recovery(xml.event));
                if let Some(guidance) = xml.guidance
                    && !cancelled.load(Ordering::Acquire)
                {
                    let id = format!("{assistant_id}-xml-recovery");
                    let message = json!({"role":"custom","customType":"xml-tool-recovery","content":format!("<xml-tool-recovery>\n{guidance}\n</xml-tool-recovery>"),"display":false,"timestamp":resolved.timestamp_ms});
                    let action = self.state()?.append_message(&id, &id, &input.timestamp, message)?;
                    self.commit(action, listener).await?;
                }
            }
            let window = generation.configuration().context_limit();
            if input.compaction.enabled
                && (error.is_some() || !has_calls)
                && ag1_core::runtime::retry::overflow(&message, window)
                && self.pending_transport()?.is_none()
                && !cancelled.load(Ordering::Acquire)
            {
                let overflow_error = error.clone().unwrap_or_else(|| DomainError::Unavailable(
                    "Provider input usage exceeds the selected model context window".into()));
                if overflow_attempted {
                    Self::emit(listener, SessionEvent::Recovery(json!({"type":"compaction_end","reason":"overflow","aborted":false,"willRetry":false,"errorMessage":"Context overflow recovery failed after one compact-and-retry attempt. Try reducing context or switching to a larger-context model."})));
                    return Err(overflow_error);
                }
                let branch = {
                    let mut state = self.state()?;
                    if state.configuration().model != generation.configuration().model {
                        return Err(DomainError::Conflict);
                    }
                    let stamp = state.stamp().clone();
                    state.branch_before_overflow(
                        &format!("{assistant_id}-overflow"),
                        &assistant_id,
                        &stamp,
                    )?
                };
                self.commit_branch(branch, listener).await?;
                overflow_attempted = true;
                let result = self
                    .compact_with_callbacks(
                        compaction::CompactInput {
                            command: format!("{assistant_id}-compact"),
                            timestamp: input.timestamp.clone(),
                            settings: input.compaction.clone(),
                            focus: None,
                            reason: compaction::Reason::Overflow,
                            llm_timeout_ms: input.compaction_timeout_ms,
                        },
                        resolver,
                        tools.clone(),
                        listener,
                    )
                    .await?;
                if let Some(result) = result {
                    attempted_compaction = Some(result.journal_revision);
                    retry_replay = Some(result.stamp);
                    continue;
                }
                return Err(overflow_error);
            }
            if let Some(error) = error {
                if input.retry.enabled
                    && ag1_core::runtime::retry::retryable(&message, window)
                    && self.pending_transport()?.is_none()
                    && !cancelled.load(Ordering::Acquire)
                {
                    if retry_attempt < input.retry.max_retries {
                        retry_attempt += 1;
                        let continuation = self.retry_failure(
                                recovery::RetryInput {
                                    command: format!("{assistant_id}-retry"),
                                    failed_id: &assistant_id,
                                    message: &message,
                                    attempt: retry_attempt,
                                    settings: &input.retry,
                                    cancelled: &cancelled,
                                },
                                listener,
                            )
                            .await?;
                        retry_replay=Some(continuation.stamp.clone());
                        retry_restore=Some(continuation);
                        continue;
                    }
                    Self::emit(
                        listener,
                        SessionEvent::Recovery(
                            json!({"type":"auto_retry_end","success":false,"attempt":retry_attempt,"finalError":message["errorMessage"]}),
                        ),
                    );
                }
                return Err(error);
            }
            if retry_attempt > 0 {
                Self::emit(
                    listener,
                    SessionEvent::Recovery(
                        json!({"type":"auto_retry_end","success":true,"attempt":retry_attempt}),
                    ),
                );
                retry_attempt = 0;
            }
            overflow_attempted = false;
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if let Some(queue) = &command_queue {
                let steering = queue.deliver_messages(false, &cancelled, listener).await?;
                if steering > 0 { continue; }
                if !has_calls && queue.deliver_messages(true, &cancelled, listener).await? > 0 { continue; }
            }
            if let Some(reason) = repetition.stop_reason()? {
                let id = format!("{assistant_id}-repetition-pause");
                let (action, terminal) = loop {
                    self.wait_for_commit().await?;
                    let mut state = self.state()?;
                    if state.pending_commit() { continue; }
                    let model = &state.configuration().model;
                    let terminal = json!({"role":"assistant","content":[{"type":"text","text":reason}],
                        "api":model["api"],"provider":model["provider"],"model":model["id"],
                        "usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
                            "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
                        "stopReason":"stop","timestamp":resolved.timestamp_ms,"nativePolicy":"repetition_guard"});
                    let action = state.append_message(&id, &id, &input.timestamp, terminal.clone())?;
                    break (action, terminal);
                };
                self.commit(action, listener).await?;
                return Ok(TurnResult { generations, journal_revision: self.journal_revision()?, assistant: terminal });
            }
            if !has_calls {
                return Ok(TurnResult {
                    generations,
                    journal_revision: self.journal_revision()?,
                    assistant: message,
                });
            }
        }
        }.await;
        let unfinished_retry = retry_restore.is_some();
        if outcome.is_err()
            && let Some(restore) = retry_restore
            && self.pending_transport()?.is_none()
        {
            let action =
                self.state()?
                    .restore_branch(&restore.command, &restore.checkpoint, &restore.stamp);
            match action {
                Ok(action) => {
                    self.commit_branch(action, listener).await?;
                }
                Err(DomainError::Conflict) => {}
                Err(error) => return Err(error),
            }
        }
        if let Err(error) = &outcome
            && unfinished_retry
        {
            Self::emit(
                listener,
                SessionEvent::Recovery(
                    json!({"type":"auto_retry_end","success":false,"attempt":retry_attempt,"finalError":format!("{error:?}")}),
                ),
            );
        }
        outcome
    }
}
