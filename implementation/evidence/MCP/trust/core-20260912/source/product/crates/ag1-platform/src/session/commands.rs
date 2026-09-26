//! Bounded P02 admission and delivery for one selected native runtime. Durable
//! intent is distinct from execution ownership: only an acknowledged insertion
//! enters this queue. Restarted or abandoned work requires explicit recovery.
use super::{Listener, NativeSession, TurnInput, registry::Selection};
use ag1_core::{Clock, DomainError, context::features::Features};
use ag1_protocol::wire as w;
use ag1_storage::{CommandResult as StoredCommand, StorageError, context::ContextStore};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

const EVENT_HISTORY: usize = 1024;
mod approval;
mod cancellation;
mod configuration;
mod input;
mod messages;
mod modes;
mod planning;
pub use approval::PendingApproval;
pub use messages::QueueMode;
mod delivery;
mod retirement;
pub const MAX_PROMPT_BYTES: u64 = 4 * 1024 * 1024;

/// Host settings, never accepted from an untrusted command payload.
#[derive(Clone)]
pub struct RunSettings {
    /// Host-resolved catalogs. Wire commands select identities, never arbitrary
    /// model configuration, executable tools or credentials from client JSON.
    pub models: Vec<serde_json::Value>,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub tools: Vec<serde_json::Value>,
    pub max_generations: Option<u64>,
    pub compaction: ag1_core::context::compaction::Settings,
    pub compaction_timeout_ms: u64,
    pub retry: ag1_core::runtime::retry::Settings,
    /// Trusted host policy for planning turns; wire input cannot enable it.
    pub planning_inspection: bool,
}
impl Default for RunSettings {
    fn default() -> Self {
        Self {
            models: vec![],
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            tools: vec![],
            max_generations: None,
            compaction: Default::default(),
            compaction_timeout_ms: 900_000,
            retry: Default::default(),
            planning_inspection: true,
        }
    }
}
#[derive(Clone)]
struct Queued {
    kind: messages::Kind,
    claimed: bool,
    retiring: bool,
    key: String,
    operation: w::OperationId,
    input: w::PromptInput,
    cancelled: Arc<AtomicBool>,
    intent: Vec<u8>,
    mode: Option<modes::Request>,
}
struct Active {
    client_control: bool,
    key: String,
    operation: w::OperationId,
    recovery: bool,
    cancelled: Arc<AtomicBool>,
}
#[derive(Default)]
struct Book {
    exclusive_control: bool,
    queue: VecDeque<Queued>,
    reserved: usize,
    active: Option<Active>,
    sequence: u64,
    events: VecDeque<w::Event>,
    last_terminal: Option<w::OperationView>,
    last_failure: Option<String>,
    delivery: delivery::State,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Terminal {
    version: u32,
    operation: w::OperationView,
    journal_revision: u64,
    context_epoch: u64,
    execution_settled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    generations: Option<u64>,
}
pub struct SessionCommands {
    selection: Selection,
    scope: w::Scope,
    book: Mutex<Book>,
    admission: Arc<tokio::sync::Mutex<()>>,
    administrative: crate::executor::BlockingPool,
    waiting: Arc<tokio::sync::Semaphore>,
    clock: Arc<dyn Clock>,
    settings: RunSettings,
}
struct Reservation(Arc<SessionCommands>);
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut book) = self.0.book.lock() {
            book.reserved -= 1;
        }
    }
}
/// Dropping a consumer does not return its prompt to the queue. Even a fully
/// drained native turn needs a terminal storage receipt before another claim.
struct Claim(Arc<SessionCommands>);
impl Drop for Claim {
    fn drop(&mut self) {
        if let Ok(mut book) = self.0.book.lock()
            && let Some(active) = &mut book.active
        {
            active.recovery = true;
        }
    }
}
fn unavailable(family: &str) -> w::ProtocolError {
    w::ProtocolError::Unavailable {
        family: family.into(),
    }
}
pub(super) fn domain(error: DomainError) -> w::ProtocolError {
    match error {
        DomainError::QueueFull => w::ProtocolError::QueueFull {
            queue: "native_worker".into(),
            capacity: w::MAX_ENDPOINT_WAITING as u32,
        },
        DomainError::Conflict => w::ProtocolError::StaleContext,
        DomainError::NotFound | DomainError::PermissionDenied => w::ProtocolError::NotFound,
        DomainError::InvalidInput(_) => w::ProtocolError::Malformed {
            detail: "invalid native command input".into(),
        },
        DomainError::LimitExceeded(_) => w::ProtocolError::Oversize {
            limit: MAX_PROMPT_BYTES as u32,
            use_artifact: true,
        },
        _ => unavailable("native_command_recovery"),
    }
}
pub(super) fn storage(error: StorageError) -> w::ProtocolError {
    match error {
        StorageError::RevisionConflict { .. } => w::ProtocolError::StaleContext,
        StorageError::Conflict(_) => w::ProtocolError::CommandConflict,
        error => domain(error.into()),
    }
}
impl SessionCommands {
    /// One command queue per native owner. Recreating a queue around the same
    /// live owner would discard execution claims, so it is explicitly rejected.
    pub fn new(
        selection: Selection,
        instance: w::InstanceId,
        clock: Arc<dyn Clock>,
        mut settings: RunSettings,
    ) -> Result<Arc<Self>, DomainError> {
        let runtime = selection.services.runtime();
        let _activity = runtime.enter_owned()?;
        if settings.max_generations == Some(0)
            || !runtime.settled()?
            || !runtime.persistence_healthy()?
        {
            return Err(DomainError::Conflict);
        }
        let scope = w::Scope {
            workspace: runtime
                .workspace
                .clone()
                .try_into()
                .map_err(DomainError::InvalidInput)?,
            session: runtime
                .state()?
                .stamp()
                .session()
                .as_str()
                .to_owned()
                .try_into()
                .map_err(DomainError::InvalidInput)?,
            instance,
            epoch: w::Sequence::new(selection.epoch),
        };
        if settings.tools.is_empty() {
            settings.tools = selection.services.tools().client_catalog();
        }
        configuration::catalogs(&runtime.configuration()?, &mut settings)?;
        // Control admission must not queue behind a tool waiting for approval.
        // One bounded worker belongs to this facade and is shared by all callers.
        let administrative = crate::executor::BlockingPool::new(1, w::MAX_ENDPOINT_WAITING)?;
        runtime
            .commands_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DomainError::Conflict)?;
        let owner = Arc::new(Self {
            selection,
            scope,
            administrative,
            book: Mutex::new(Book::default()),
            admission: Arc::new(tokio::sync::Mutex::new(())),
            waiting: Arc::new(tokio::sync::Semaphore::new(w::MAX_ENDPOINT_WAITING)),
            clock,
            settings,
        });
        owner
            .runtime()
            .command_owner
            .set(Arc::downgrade(&owner))
            .map_err(|_| DomainError::Conflict)?;
        Ok(owner)
    }
    /// Called after native admission freezes and accepted calls drain. Queued
    /// intent and abandoned claims require explicit terminal custody first.
    pub(super) fn replacement_ready(&self) -> Result<bool, DomainError> {
        let book = self.book()?;
        Ok(book.queue.is_empty() && book.active.is_none() && book.reserved == 0)
    }
    pub(super) fn runtime(&self) -> &Arc<NativeSession> {
        self.selection.services.runtime()
    }
    fn book(&self) -> Result<MutexGuard<'_, Book>, DomainError> {
        self.book.lock().map_err(|_| DomainError::Conflict)
    }
    pub(super) fn has_pending_messages(&self) -> Result<bool, DomainError> {
        Ok(!self.book()?.queue.is_empty())
    }
    pub fn scope(&self) -> &w::Scope {
        &self.scope
    }
    pub fn handshake(&self) -> w::Handshake {
        w::Handshake {
            version: Default::default(),
            scope: self.scope.clone(),
            fake: false,
            capabilities: [
                vec![
                    "prompt".into(),
                    "cancel".into(),
                    "model.select".into(),
                    "model.thinking".into(),
                    "model.context".into(),
                    "runtime.tools".into(),
                    "task".into(),
                    "plan.accept".into(),
                    "plan.reject".into(),
                    "plan.clear".into(),
                    "plan.answer".into(),
                    "plan.complete_step".into(),
                    "plan.reopen_step".into(),
                    "plan.execute".into(),
                    "session.steer".into(),
                    "session.follow_up".into(),
                    "session.compact".into(),
                    "session.print".into(),
                    "session.tree".into(),
                    "command_result".into(),
                    "artifact_read".into(),
                    "snapshot".into(),
                    "replay".into(),
                    "subscribe".into(),
                    "progress".into(),
                ],
                if self.selection.services.tools().approval_handler().is_some() {
                    vec!["approve".into()]
                } else {
                    vec![]
                },
                if self.selection.services.tools().supports_planning() {
                    vec!["plan.draft".into()]
                } else {
                    vec![]
                },
            ]
            .concat(),
        }
    }
    fn key(&self, command: &w::CommandId) -> String {
        let identity = json!([self.scope.workspace, self.scope.session, command]);
        format!(
            "native-command-{}",
            ag1_storage::digest(identity.to_string().as_bytes())
        )
    }
    fn operation(key: &str) -> w::OperationId {
        key.to_owned().try_into().expect("bounded digest ID")
    }
    fn snapshot_at(&self, book: &Book) -> Result<w::Snapshot, DomainError> {
        Ok(w::Snapshot {
            fake: false,
            scope: self.scope.clone(),
            branch: self
                .scope
                .session
                .as_str()
                .to_owned()
                .try_into()
                .map_err(DomainError::InvalidInput)?,
            revision: w::Sequence::new(self.runtime().journal_revision()?),
            watermark: w::Sequence::new(book.sequence),
            queued: book
                .active
                .iter()
                .map(|a| a.operation.clone())
                .chain(book.queue.iter().map(|q| q.operation.clone()))
                .collect(),
            last_terminal: book.last_terminal.clone(),
        })
    }
    pub fn last_failure(&self) -> Result<Option<String>, DomainError> {
        Ok(self.book()?.last_failure.clone())
    }
    pub fn snapshot(&self) -> Result<w::Snapshot, DomainError> {
        self.snapshot_at(&*self.book()?)
    }
    /// Host-local operation event sequence, independent of SQL global sequence
    /// and journal revision. Scope changes or bounded-history gaps require a
    /// snapshot; revision may stay equal on admission or jump on completion.
    pub fn replay(&self, cursor: &w::Cursor) -> Result<w::Delivery, DomainError> {
        let book = self.book()?;
        self.replay_at(&book, cursor)
    }
    fn replay_at(&self, book: &Book, cursor: &w::Cursor) -> Result<w::Delivery, DomainError> {
        let after = cursor.after.get();
        if cursor.scope != self.scope
            || after > book.sequence
            || book
                .events
                .front()
                .is_some_and(|e| after < e.sequence.get().saturating_sub(1))
        {
            return Ok(w::Delivery::Gap {
                snapshot: self.snapshot_at(book)?,
            });
        }
        let events: Vec<_> = book
            .events
            .iter()
            .filter(|e| e.sequence.get() > after)
            .take(32)
            .cloned()
            .collect();
        let through = events
            .last()
            .map(|e| e.sequence.clone())
            .unwrap_or_else(|| cursor.after.clone());
        Ok(w::Delivery::Events {
            events,
            more: through.get() < book.sequence,
            cursor: w::Cursor {
                scope: self.scope.clone(),
                after: through,
            },
        })
    }
    fn publish(&self, book: &mut Book, operation: w::OperationView) -> Result<(), DomainError> {
        let sequence = book.sequence.checked_add(1).ok_or(DomainError::Conflict)?;
        let revision = self.runtime().journal_revision()?;
        if operation.status != w::OperationStatus::Queued {
            delivery::terminal(book, &operation.operation);
        }
        book.events.push_back(w::Event {
            scope: self.scope.clone(),
            sequence: w::Sequence::new(sequence),
            revision: w::Sequence::new(revision),
            operation,
        });
        book.sequence = sequence;
        if book.events.len() > EVENT_HISTORY {
            book.events.pop_front();
        }
        Ok(())
    }
    /// Acknowledged workspace lifecycle events share this scope's delivery log.
    pub(super) fn publish_lifecycle(&self, operation: w::OperationView) -> Result<(), DomainError> {
        let mut book = self.book()?;
        self.publish(&mut book, operation.clone())?;
        if operation.status != w::OperationStatus::Queued {
            book.last_terminal = Some(operation);
        }
        Ok(())
    }
    /// Polling admission performs no filesystem work on the async executor.
    /// The accepted worker retains both admission serialization and lifecycle
    /// fencing even if the response future is dropped.
    pub async fn dispatch(self: &Arc<Self>, envelope: w::CommandEnvelope) -> w::Reply {
        let result = self
            .dispatch_result(&envelope)
            .await
            .unwrap_or_else(|error| w::CommandResult::Rejected { error });
        let watermark = self.book().map(|b| b.sequence).unwrap_or(0);
        w::Reply {
            id: envelope.id,
            scope: self.scope.clone(),
            watermark: w::Sequence::new(watermark),
            result,
        }
    }
    async fn dispatch_result(
        self: &Arc<Self>,
        envelope: &w::CommandEnvelope,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        envelope.version.negotiate()?;
        w::encode(&w::WireMessage::Command(envelope.clone()))?;
        if envelope.scope != self.scope {
            return Err(w::ProtocolError::StaleContext);
        }
        let activity = self.runtime().enter_owned().map_err(domain)?;
        let waiting =
            self.waiting
                .clone()
                .try_acquire_owned()
                .map_err(|_| w::ProtocolError::QueueFull {
                    queue: "native_admission".into(),
                    capacity: w::MAX_ENDPOINT_WAITING as u32,
                })?;
        let admission = self.admission.clone().lock_owned().await;
        let owner = self.clone();
        let envelope = envelope.clone();
        self.administrative
            .submit(move || {
                let (_activity, _waiting, _admission) = (activity, waiting, admission);
                Ok(owner.dispatch_worker(envelope))
            })
            .await
            .map_err(domain)?
    }
    fn artifacts(&self) -> Result<ContextStore<'_>, DomainError> {
        ContextStore::new(
            &self.runtime().store,
            ag1_core::SessionId::new(self.scope.session.as_str())?,
            &self.runtime().origin,
            "native-command-read",
            "command-read",
            Features::default(),
        )
    }
    fn dispatch_worker(
        self: &Arc<Self>,
        envelope: w::CommandEnvelope,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        match &envelope.command {
            w::Command::CommandResult { command } => return self.lookup_worker(command),
            w::Command::ArtifactRead {
                artifact,
                offset,
                limit,
            } => {
                if *limit == 0 || *limit > w::MAX_ARTIFACT_PAGE {
                    return Err(w::ProtocolError::Oversize {
                        limit: w::MAX_ARTIFACT_PAGE,
                        use_artifact: false,
                    });
                }
                let artifacts = self.artifacts().map_err(domain)?;
                let origin = artifacts
                    .origin_for(artifact.as_str())
                    .map_err(|_| w::ProtocolError::NotFound)?;
                let page = self
                    .runtime()
                    .store
                    .page(
                        self.scope.session.as_str(),
                        &origin,
                        artifact.as_str(),
                        offset.get(),
                        *limit,
                    )
                    .map_err(storage)?;
                return Ok(w::CommandResult::ArtifactPage {
                    artifact: artifact.clone(),
                    offset: w::Sequence::new(page.offset),
                    base64: STANDARD.encode(page.bytes),
                    next: page.next_offset.map(w::Sequence::new),
                });
            }
            w::Command::Prompt { .. }
            | w::Command::Cancel { .. }
            | w::Command::Approve { .. }
            | w::Command::Model { .. }
            | w::Command::Runtime { .. }
            | w::Command::Task { .. }
            | w::Command::Plan { .. }
            | w::Command::Session { .. } => (),
            _ => return Err(unavailable("native_command_family")),
        }
        let key = self.key(&envelope.id);
        if self.runtime().restart_required.load(Ordering::Acquire) {
            return Err(unavailable("native_restart_recovery"));
        }
        // A duplicate must survive queue-full, later revisions and even loss of
        // its input artifact. Atomic INSERT below remains the execution grant.
        if self
            .runtime()
            .store
            .command_result(&key)
            .map_err(storage)?
            .is_some()
        {
            // Reuse the storage layer's exact wrapper/hash check, including
            // version, session and revision; comparing only payload is weaker.
            let payload = serde_json::to_value(&envelope)
                .map_err(|_| unavailable("native_command_recovery"))?;
            self.runtime()
                .store
                .admit_session_command(
                    self.scope.session.as_str(),
                    &key,
                    envelope.expected_revision.get(),
                    &payload,
                )
                .map_err(storage)?;
            return Ok(w::CommandResult::Accepted {
                operation: match &envelope.command {
                    w::Command::Cancel { operation } => operation.clone(),
                    _ => Self::operation(&key),
                },
            });
        }
        if let w::Command::Cancel { operation } = &envelope.command {
            return self.cancel_worker(&envelope, &key, operation);
        }
        if let w::Command::Approve { binding, allow } = &envelope.command {
            return self.approve_worker(&envelope, &key, binding, *allow);
        }
        let plan_input = modes::queued(&envelope.command)?;
        if matches!(
            plan_input,
            Some((modes::Request::Planning(planning::Request::Draft { .. }), _))
        ) && !self.selection.services.tools().supports_planning()
        {
            return Err(unavailable("planning_tools"));
        }
        if let w::Command::Plan { .. } = &envelope.command
            && plan_input.is_none()
        {
            let _exclusive = self.exclusive_plan_control()?;
            return self.configuration_worker(&envelope, &key);
        }
        if matches!(
            envelope.command,
            w::Command::Model { .. } | w::Command::Runtime { .. } | w::Command::Task { .. }
        ) && plan_input.is_none()
        {
            return self.configuration_worker(&envelope, &key);
        }
        let (kind, input, mode) = if let Some((mode, input)) = plan_input {
            (messages::Kind::Prompt, input, Some(mode))
        } else {
            let (kind, input) = messages::input(&envelope.command)?;
            (kind, input, None)
        };
        if !self.runtime().persistence_healthy().map_err(domain)? {
            return Err(unavailable("native_command_recovery"));
        }
        if matches!(
            &input,
            w::PromptInput::Artifact { .. } | w::PromptInput::MessageArtifact { .. }
        ) {
            self.input_message(&input, self.clock.now_ms())
                .map_err(domain)?;
        }

        {
            let mut book = self.book().map_err(domain)?;
            let count = book.queue.len() + book.reserved + usize::from(book.active.is_some());
            if count >= w::MAX_SESSION_QUEUE {
                return Err(w::ProtocolError::QueueFull {
                    queue: "session".into(),
                    capacity: w::MAX_SESSION_QUEUE as u32,
                });
            }
            book.sequence.checked_add(2 * (count as u64 + 1)).ok_or(
                w::ProtocolError::Capacity {
                    resource: "event_sequence".into(),
                },
            )?;
            book.reserved += 1;
        }
        let _reservation = Reservation(self.clone());
        let payload =
            serde_json::to_value(&envelope).map_err(|_| unavailable("native_command_recovery"))?;
        let admitted = self
            .runtime()
            .store
            .admit_session_command(
                self.scope.session.as_str(),
                &key,
                envelope.expected_revision.get(),
                &payload,
            )
            .map_err(storage)?;
        let operation = Self::operation(&key);
        if admitted.inserted {
            let mut book = self.book().map_err(domain)?;
            let same_tree_leaf = match &mode {
                Some(modes::Request::Tree(options)) => self
                    .runtime()
                    .state()
                    .map_err(domain)?
                    .branch()
                    .last()
                    .is_some_and(|e| e["id"] == options.target),
                _ => false,
            };
            let preempt = mode
                .as_ref()
                .is_some_and(|mode| mode.compact() && !same_tree_leaf)
                .then(|| {
                    book.active
                        .as_ref()
                        .filter(|active| !active.recovery)
                        .map(|active| active.cancelled.clone())
                })
                .flatten();
            self.publish(
                &mut book,
                w::OperationView {
                    operation: operation.clone(),
                    status: w::OperationStatus::Queued,
                },
            )
            .map_err(domain)?;
            book.queue.push_back(Queued {
                kind,
                claimed: false,
                retiring: false,
                key,
                operation: operation.clone(),
                input: input.clone(),
                cancelled: Arc::new(AtomicBool::new(false)),
                intent: admitted.record.intent,
                mode,
            });
            if let Some(cancel) = &preempt {
                cancel.store(true, Ordering::Release);
            }
            drop(book);
            if let Some(cancel) = preempt {
                self.runtime().cancel_command(&cancel).map_err(domain)?;
            }
        }
        Ok(w::CommandResult::Accepted { operation })
    }
    fn lookup_worker(&self, command: &w::CommandId) -> Result<w::CommandResult, w::ProtocolError> {
        let key = self.key(command);
        let old = self
            .runtime()
            .store
            .command_result(&key)
            .map_err(storage)?
            .ok_or(w::ProtocolError::NotFound)?;
        if let Some(result) = self.cancel_lookup(&old)? {
            return Ok(result);
        }
        if old.state == "completed" {
            return Ok(w::CommandResult::Settled {
                operation: Self::terminal(&key, &old)?,
            });
        }
        let book = self.book().map_err(domain)?;
        if book.queue.iter().any(|q| {
            q.key == key && (!q.claimed || book.active.as_ref().is_some_and(|a| !a.recovery))
        }) || book
            .active
            .as_ref()
            .is_some_and(|a| a.key == key && !a.recovery)
        {
            Ok(w::CommandResult::Accepted {
                operation: Self::operation(&key),
            })
        } else {
            Err(unavailable("native_command_recovery"))
        }
    }
    pub(super) fn terminal(
        key: &str,
        old: &StoredCommand,
    ) -> Result<w::OperationView, w::ProtocolError> {
        let terminal: Terminal = serde_json::from_slice(
            old.result
                .as_deref()
                .ok_or_else(|| unavailable("native_command_recovery"))?,
        )
        .map_err(|_| unavailable("native_command_recovery"))?;
        if terminal.version != 1
            || !terminal.execution_settled
            || terminal.operation.operation != Self::operation(key)
            || terminal.operation.status == w::OperationStatus::Queued
        {
            return Err(unavailable("native_command_recovery"));
        }
        Ok(terminal.operation)
    }
    /// Recover publication after a lost terminal acknowledgement. This only
    /// consumes an already committed, validated terminal record; pending intent
    /// or uncertain native effects grant no replay or cancellation authority.
    pub async fn reconcile_terminal(
        self: &Arc<Self>,
        command: w::CommandId,
    ) -> Result<w::OperationView, DomainError> {
        let activity = self.runtime().enter_owned()?;
        let owner = self.clone();
        self.runtime()
            .pool
            .submit(move || {
                let _activity = activity;
                let key = owner.key(&command);
                let old = owner
                    .runtime()
                    .store
                    .command_result(&key)?
                    .ok_or(DomainError::NotFound)?;
                if old.state != "completed" {
                    return Err(DomainError::DurabilityUnknown);
                }
                let operation =
                    Self::terminal(&key, &old).map_err(|_| DomainError::DurabilityUnknown)?;
                let mut book = owner.book()?;
                if book.queue.iter().any(|q| q.key == key && q.claimed) {
                    if book.active.is_some()
                        || !owner.runtime().settled()?
                        || !owner.runtime().persistence_healthy()?
                    {
                        return Err(DomainError::Conflict);
                    }
                    owner.publish(&mut book, operation.clone())?;
                    book.last_terminal = Some(operation.clone());
                    book.queue.retain(|q| q.key != key);
                    return Ok(operation);
                }

                if let Some(active) = &book.active {
                    if active.key != key
                        || !active.recovery
                        || !owner.runtime().settled()?
                        || !owner.runtime().persistence_healthy()?
                    {
                        return Err(DomainError::Conflict);
                    }
                    owner.publish(&mut book, operation.clone())?;
                    book.last_terminal = Some(operation.clone());
                    book.active = None;
                }
                Ok(operation)
            })
            .await
    }
    /// Host consumer runs one admitted prompt against the captured real P06–P10
    /// services. Queued prompts compile the latest branch when their turn starts.
    /// The host must keep polling through cancellation to retain settlement.
    pub async fn run_next(
        self: &Arc<Self>,
        listener: &dyn Listener,
    ) -> Result<Option<w::OperationView>, DomainError> {
        let activity = self.runtime().enter_owned()?;
        let (queued, claim) = {
            let mut book = self.book()?;
            if book.exclusive_control
                || book.active.is_some()
                || book.queue.iter().any(|q| q.claimed)
            {
                return Err(DomainError::Conflict);
            }
            if !self.runtime().settled()? || !self.runtime().persistence_healthy()? {
                return Err(DomainError::DurabilityUnknown);
            }
            let index = book
                .queue
                .iter()
                .position(|q| q.mode.as_ref().is_some_and(modes::Request::compact))
                .or_else(|| {
                    book.queue
                        .iter()
                        .position(|q| q.kind == messages::Kind::Prompt)
                })
                .or_else(|| {
                    book.queue
                        .iter()
                        .position(|q| q.kind == messages::Kind::Steering)
                })
                .or_else(|| (!book.queue.is_empty()).then_some(0));
            let Some(index) = index else {
                return Ok(None);
            };
            let queued = book.queue.remove(index).ok_or(DomainError::Conflict)?;
            book.last_failure = None;
            book.active = Some(Active {
                client_control: matches!(
                    queued.mode,
                    Some(modes::Request::Extension { .. } | modes::Request::Resource { .. })
                ),
                key: queued.key.clone(),
                operation: queued.operation.clone(),
                recovery: false,
                cancelled: queued.cancelled.clone(),
            });
            (queued, Claim(self.clone()))
        };
        let now = self.clock.now_ms();
        let timestamp = i64::try_from(now)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .ok_or_else(|| DomainError::InvalidInput("command clock".into()))?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let message = if queued.cancelled.load(Ordering::Acquire) {
            Err(DomainError::Cancelled)
        } else {
            let owner = self.clone();
            let input = queued.input.clone();
            self.runtime()
                .pool
                .submit(move || owner.input_message(&input, now))
                .await
        };
        let relay = delivery::Relay::new(self, &queued.operation, listener);
        let result = match message {
            Ok(message) => {
                let input = TurnInput {
                    command: format!(
                        "native-prompt-{}",
                        ag1_storage::digest(queued.key.as_bytes())
                    ),
                    message,
                    timestamp,
                    max_generations: self.settings.max_generations,
                    compaction: self.settings.compaction.clone(),
                    compaction_timeout_ms: self.settings.compaction_timeout_ms,
                    retry: self.settings.retry.clone(),
                };
                let origin = super::RunOrigin::Command(queued.cancelled.clone(), self.clone());
                match &queued.mode {
                    Some(modes::Request::Resource { name, args }) => {
                        self.runtime()
                            .client_resource(
                                self.selection.services.tools().clone(),
                                queued.key.clone(),
                                name.clone(),
                                args.clone(),
                                queued.cancelled.clone(),
                            )
                            .await
                    }
                    Some(modes::Request::Extension { method, args }) => {
                        self.runtime()
                            .client_extension(
                                self.selection.services.tools().clone(),
                                method.clone(),
                                args.clone(),
                                queued.cancelled.clone(),
                            )
                            .await
                    }
                    Some(modes::Request::Print) => super::print::run_selected(
                        self.runtime(),
                        input,
                        self.selection.services.provider().as_ref(),
                        self.selection.services.tools().clone(),
                        &relay,
                        (Default::default(), origin),
                    )
                    .await
                    .and_then(|r| {
                        if r.exit_code == 0 {
                            Ok(r.turn)
                        } else {
                            Err(DomainError::Unavailable(
                                "print completion requirements were not met".into(),
                            ))
                        }
                    }),
                    Some(modes::Request::Planning(planning::Request::Draft {
                        previous,
                        feedback,
                    })) => super::planning::run_selected(
                        self.runtime(),
                        input,
                        self.selection.services.provider().as_ref(),
                        self.selection.services.tools().clone(),
                        &relay,
                        (
                            super::planning::Options {
                                previous: previous.clone(),
                                feedback: feedback.clone(),
                                allow_inspection: self.settings.planning_inspection,
                                ..Default::default()
                            },
                            origin,
                        ),
                    )
                    .await
                    .map(|r| r.turn),
                    Some(modes::Request::Planning(planning::Request::Execute {
                        plan,
                        step_index,
                    })) => super::planning::execute_selected(
                        self.runtime(),
                        input,
                        self.selection.services.provider().as_ref(),
                        self.selection.services.tools().clone(),
                        &relay,
                        (
                            super::planning::Execution {
                                plan: plan.clone(),
                                step_index: *step_index,
                            },
                            origin,
                        ),
                    )
                    .await
                    .map(|r| r.turn),
                    Some(modes::Request::Compact { focus }) => self
                        .runtime()
                        .compact_for_command(
                            super::compaction::CompactInput {
                                command: format!(
                                    "native-compact-{}",
                                    ag1_storage::digest(queued.key.as_bytes())
                                ),
                                timestamp: input.timestamp,
                                settings: self.settings.compaction.clone(),
                                focus: focus.clone(),
                                reason: super::compaction::Reason::Manual,
                                llm_timeout_ms: self.settings.compaction_timeout_ms,
                            },
                            self.selection.services.provider().as_ref(),
                            self.selection.services.tools().clone(),
                            queued.cancelled.clone(),
                            &relay,
                        )
                        .await
                        .and_then(|_| {
                            Ok(super::TurnResult {
                                generations: 0,
                                journal_revision: self.runtime().journal_revision()?,
                                assistant: serde_json::Value::Null,
                            })
                        }),
                    Some(modes::Request::Tree(options)) => self
                        .runtime()
                        .navigate_tree_for_command(
                            super::tree::TreeInput {
                                command: format!(
                                    "native-tree-{}",
                                    ag1_storage::digest(queued.key.as_bytes())
                                ),
                                timestamp: input.timestamp,
                                options: options.clone(),
                                reserve_tokens: 16384,
                                summary_timeout_ms: self.settings.compaction_timeout_ms,
                            },
                            self.selection.services.provider().as_ref(),
                            self.selection.services.tools().clone(),
                            queued.cancelled.clone(),
                            &relay,
                        )
                        .await
                        .map(|result| super::TurnResult {
                            generations: 0,
                            journal_revision: result.journal_revision,
                            assistant: serde_json::Value::Null,
                        }),
                    None => {
                        self.runtime()
                            .run_inner(
                                input,
                                self.selection.services.provider().as_ref(),
                                self.selection.services.tools().clone(),
                                &relay,
                                origin,
                                None,
                            )
                            .await
                    }
                }
            }
            Err(error) => Err(error),
        };
        if !self.runtime().settled()? || !self.runtime().persistence_healthy()? {
            return Err(DomainError::DurabilityUnknown);
        }
        let operation = w::OperationView {
            operation: queued.operation,
            status: match &result {
                Ok(_) => w::OperationStatus::Completed,
                Err(DomainError::Cancelled) => w::OperationStatus::Cancelled,
                Err(_) => w::OperationStatus::Failed,
            },
        };
        let evidence = {
            let state = self.runtime().state()?;
            serde_json::to_vec(&Terminal {
                version: 1,
                operation: operation.clone(),
                journal_revision: state.journal_revision(),
                context_epoch: state.stamp().epoch(),
                execution_settled: true,
                error: result.as_ref().err().map(|e| format!("{e:?}")),
                generations: result.as_ref().ok().map(|r| r.generations),
            })
            .map_err(|_| DomainError::Conflict)?
        };
        let failure = result.as_ref().err().map(|e| format!("{e:?}"));
        let owner = self.clone();
        self.runtime()
            .pool
            .submit(move || {
                let (_claim, _activity) = (claim, activity);
                owner
                    .runtime()
                    .store
                    .settle_intent(&queued.key, &queued.intent, &evidence)?;
                let mut book = owner.book()?;
                owner.publish(&mut book, operation.clone())?;
                book.last_terminal = Some(operation.clone());
                book.last_failure = failure;
                book.active = None;
                Ok(Some(operation))
            })
            .await
    }
}
