//! Workspace command owner across session changes. Prepared queues stay private
//! until the lifecycle outcome acknowledges; an accepted driver outlives its
//! caller. Incomplete records from another incarnation never authorize replay.
use super::{
    Listener,
    commands::{self, RunSettings, SessionCommands},
    registry::{
        PreparedRuntime, RuntimeFactory, RuntimeRegistry, Selection, SessionAction, SessionRequest,
        Target,
    },
};
use ag1_core::{Clock, DomainError, PortFuture, SessionId};
use ag1_protocol::wire as w;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

mod initialization;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionReceipt {
    pub version: u32,
    pub operation: w::OperationView,
    pub source: w::Scope,
    pub selected: w::Scope,
    pub journal_revision: u64,
    /// Refers to immutable source history; no large user text in a wire frame.
    pub selected_entry: Option<String>,
}
struct Ready {
    receipt: SessionReceipt,
    bytes: Vec<u8>,
    commands: Arc<SessionCommands>,
}
struct Pending {
    envelope: w::CommandEnvelope,
    key: String,
    request: SessionRequest,
    source: Arc<SessionCommands>,
    intent: Mutex<Option<Vec<u8>>>,
    candidate: Mutex<Option<Arc<SessionCommands>>>,
    ready: Mutex<Option<Ready>>,
    started: AtomicBool,
    session_start: Arc<Mutex<StartState>>,
    running: AtomicBool,
}
#[derive(Default)]
struct StartState {
    attempted: bool,
    result: Option<w::OperationStatus>,
}
struct Starting(Arc<Mutex<StartState>>);
impl Drop for Starting {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock()
            && state.result.is_none()
        {
            state.result = Some(w::OperationStatus::Failed);
        }
    }
}
struct Running(Arc<Pending>);
impl Drop for Running {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::Release);
    }
}
struct Book {
    current: Arc<SessionCommands>,
    pending: Option<Arc<Pending>>,
}
pub struct RuntimeCommands {
    registry: Arc<RuntimeRegistry>,
    factory: Arc<dyn RuntimeFactory>,
    book: Mutex<Book>,
    administrative: crate::executor::BlockingPool,
    store: Arc<ag1_storage::Store>,
    instance: w::InstanceId,
    clock: Arc<dyn Clock>,
    settings: RunSettings,
    handle: tokio::runtime::Handle,
    initialization: Arc<Mutex<StartState>>,
}
fn unavailable() -> w::ProtocolError {
    w::ProtocolError::Unavailable {
        family: "native_session_recovery".into(),
    }
}
fn key(scope: &w::Scope, id: &w::CommandId) -> String {
    format!(
        "native-session-{}",
        ag1_storage::digest(
            json!([scope.workspace, scope.session, id])
                .to_string()
                .as_bytes()
        )
    )
}
fn operation(key: &str) -> w::OperationId {
    key.to_owned().try_into().expect("bounded digest ID")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Resume {
    session: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fork {
    entry: String,
}
fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, w::ProtocolError> {
    serde_json::from_value(args.clone()).map_err(|_| w::ProtocolError::Malformed {
        detail: "session arguments".into(),
    })
}
impl RuntimeCommands {
    pub async fn reconcile_terminal(
        &self,
        id: w::CommandId,
    ) -> Result<w::OperationView, DomainError> {
        self.current(false)?.reconcile_terminal(id).await
    }
    pub fn last_failure(&self) -> Result<Option<String>, DomainError> {
        self.current(true)?.last_failure()
    }
    pub fn client_transition_pending(&self) -> Result<bool, DomainError> {
        Ok(self
            .book
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .pending
            .is_some())
    }
    /// Presentation-only access to the current approval projection.
    pub fn pending_approvals(&self) -> Result<Vec<commands::PendingApproval>, DomainError> {
        self.current(true)?.pending_approvals()
    }
    pub fn new(
        registry: Arc<RuntimeRegistry>,
        factory: Arc<dyn RuntimeFactory>,
        instance: w::InstanceId,
        clock: Arc<dyn Clock>,
        settings: RunSettings,
        handle: tokio::runtime::Handle,
    ) -> Result<Arc<Self>, DomainError> {
        if !registry.has_durable_selection() {
            return Err(DomainError::InvalidInput(
                "durable registry required".into(),
            ));
        }
        let selection = registry.selected()?;
        let store = selection.services.runtime().store.clone();
        let current =
            SessionCommands::new(selection, instance.clone(), clock.clone(), settings.clone())?;
        Ok(Arc::new(Self {
            registry,
            factory,
            book: Mutex::new(Book {
                current,
                pending: None,
            }),
            administrative: crate::executor::BlockingPool::new(1, w::MAX_ENDPOINT_WAITING)?,
            store,
            instance,
            clock,
            settings,
            handle,
            initialization: Arc::new(Mutex::new(StartState::default())),
        }))
    }
    fn current(&self, allow_pending: bool) -> Result<Arc<SessionCommands>, DomainError> {
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        if book.pending.is_some() && !allow_pending {
            return Err(DomainError::DurabilityUnknown);
        }
        if !allow_pending {
            self.check_initialization()?;
        }
        Ok(book.current.clone())
    }
    pub fn handshake(&self) -> Result<w::Handshake, DomainError> {
        let mut handshake = self.current(true)?.handshake();
        handshake.capabilities.extend([
            "session.new".into(),
            "session.resume".into(),
            "session.fork".into(),
        ]);
        Ok(handshake)
    }
    pub fn recovery_required(&self) -> Result<bool, DomainError> {
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        Ok(book.current.runtime().restart_required()
            || book
                .pending
                .as_ref()
                .is_some_and(|p| !p.running.load(Ordering::Acquire)))
    }
    pub fn replay(&self, cursor: &w::Cursor) -> Result<w::Delivery, DomainError> {
        self.current(true)?.replay(cursor)
    }
    pub fn subscribe(&self) -> Result<w::Subscription, w::ProtocolError> {
        self.current(false).map_err(commands::domain)?.subscribe()
    }
    pub fn poll(&self, id: &w::SubscriptionId) -> Result<w::Delivery, w::ProtocolError> {
        match self.current(true).map_err(commands::domain)?.poll(id) {
            Err(w::ProtocolError::NotFound) => Ok(w::Delivery::Gap {
                snapshot: self.snapshot().map_err(commands::domain)?,
            }),
            result => result,
        }
    }
    pub fn unsubscribe(&self, id: &w::SubscriptionId) -> Result<(), DomainError> {
        self.current(true)?.unsubscribe(id)
    }
    pub fn snapshot(&self) -> Result<w::Snapshot, DomainError> {
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let mut snapshot = book.current.snapshot()?;
        if let Some(pending) = &book.pending
            && pending
                .intent
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .is_some()
        {
            snapshot.queued.push(operation(&pending.key));
        }
        Ok(snapshot)
    }
    pub async fn run_next(
        self: &Arc<Self>,
        listener: &dyn Listener,
    ) -> Result<Option<w::OperationView>, DomainError> {
        self.current(false)?.run_next(listener).await
    }
    pub async fn dispatch(self: &Arc<Self>, envelope: w::CommandEnvelope) -> w::Reply {
        let before = self
            .snapshot()
            .ok()
            .filter(|s| s.scope == envelope.scope)
            .map(|s| s.watermark.get())
            .unwrap_or(0);
        let result = self
            .dispatch_result(&envelope)
            .await
            .unwrap_or_else(|error| w::CommandResult::Rejected { error });
        let watermark = self
            .snapshot()
            .ok()
            .filter(|s| s.scope == envelope.scope)
            .map(|s| s.watermark.get())
            .unwrap_or(before);
        w::Reply {
            id: envelope.id,
            scope: envelope.scope,
            watermark: w::Sequence::new(watermark),
            result,
        }
    }
    async fn prior(
        &self,
        envelope: &w::CommandEnvelope,
        id: &w::CommandId,
        validate: bool,
    ) -> Result<Option<ag1_storage::CommandResult>, w::ProtocolError> {
        let current = self.current(true).map_err(commands::domain)?;
        if envelope.scope.workspace != current.scope().workspace
            || envelope.scope.instance != self.instance
        {
            return Err(w::ProtocolError::StaleContext);
        }
        let store = self.store.clone();
        let key = key(&envelope.scope, id);
        let envelope = envelope.clone();
        self.administrative
            .submit(move || {
                Ok((|| {
                    let record = store.command_result(&key)?;
                    if record.is_some() && validate {
                        store.admit_session_command(
                            envelope.scope.session.as_str(),
                            &key,
                            envelope.expected_revision.get(),
                            &serde_json::to_value(&envelope)?,
                        )?;
                    }
                    Ok::<_, ag1_storage::StorageError>(record)
                })())
            })
            .await
            .map_err(commands::domain)?
            .map_err(commands::storage)
    }
    async fn dispatch_result(
        self: &Arc<Self>,
        envelope: &w::CommandEnvelope,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        envelope.version.negotiate()?;
        w::encode(&w::WireMessage::Command(envelope.clone()))?;
        if let w::Command::CommandResult { command } = &envelope.command {
            if let Some(record) = self.prior(envelope, command, false).await? {
                return self.receipt_result(
                    &envelope.scope,
                    &key(&envelope.scope, command),
                    &record,
                );
            }
            let current = self.current(true).map_err(commands::domain)?;
            if envelope.scope == *current.scope() {
                return Ok(current.dispatch(envelope.clone()).await.result);
            }
            // Retired queues do not need to remain in memory to query their
            // immutable terminal records. Incomplete historical work is never
            // admitted to the new queue.
            let store = self.store.clone();
            let scope = envelope.scope.clone();
            let command = command.clone();
            return self
                .administrative
                .submit(move || {
                    let key = format!(
                        "native-command-{}",
                        ag1_storage::digest(
                            json!([scope.workspace, scope.session, command])
                                .to_string()
                                .as_bytes()
                        )
                    );
                    let result = (|| {
                        let record = store
                            .command_result(&key)
                            .map_err(commands::storage)?
                            .ok_or(w::ProtocolError::NotFound)?;
                        let intent: Value =
                            serde_json::from_slice(&record.intent).map_err(|_| unavailable())?;
                        let admitted: w::CommandEnvelope =
                            serde_json::from_value(intent["payload"].clone())
                                .map_err(|_| unavailable())?;
                        if admitted.scope != scope || admitted.id != command {
                            return Err(w::ProtocolError::NotFound);
                        }
                        if record.state != "completed" {
                            return Err(unavailable());
                        }
                        Ok(w::CommandResult::Settled {
                            operation: SessionCommands::historical_terminal(
                                &store, &scope, &key, &record,
                            )?,
                        })
                    })();
                    Ok(result)
                })
                .await
                .map_err(commands::domain)?;
        }
        let w::Command::Session { action, args } = &envelope.command else {
            let control = matches!(
                envelope.command,
                w::Command::Cancel { .. }
                    | w::Command::Approve { .. }
                    | w::Command::ArtifactRead { .. }
            );
            return Ok(self
                .current(control)
                .map_err(commands::domain)?
                .dispatch(envelope.clone())
                .await
                .result);
        };
        if !matches!(action.as_str(), "new" | "resume" | "fork") {
            return Ok(self
                .current(false)
                .map_err(commands::domain)?
                .dispatch(envelope.clone())
                .await
                .result);
        }
        if let Some(record) = self.prior(envelope, &envelope.id, true).await? {
            return self.receipt_result(
                &envelope.scope,
                &key(&envelope.scope, &envelope.id),
                &record,
            );
        }
        let key = key(&envelope.scope, &envelope.id);
        let generated = SessionId::new(format!("session-{}", ag1_storage::digest(key.as_bytes())))
            .map_err(commands::domain)?;
        let (action, target) = match action.as_str() {
            "new" => {
                let _: Empty = parse(args)?;
                (SessionAction::New, generated)
            }
            "resume" => {
                let args: Resume = parse(args)?;
                (
                    SessionAction::Resume,
                    SessionId::new(args.session).map_err(commands::domain)?,
                )
            }
            _ => {
                let args: Fork = parse(args)?;
                (SessionAction::Fork { entry: args.entry }, generated)
            }
        };
        let timestamp = i64::try_from(self.clock.now_ms())
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .ok_or_else(unavailable)?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let (pending, activity) = {
            let mut book = self.book.lock().map_err(|_| unavailable())?;
            self.check_initialization().map_err(commands::domain)?;
            if let Some(pending) = &book.pending {
                return if pending.envelope == *envelope
                    && pending.intent.lock().map_err(|_| unavailable())?.is_some()
                {
                    Ok(w::CommandResult::Accepted {
                        operation: operation(&pending.key),
                    })
                } else {
                    Err(unavailable())
                };
            }
            if envelope.scope != *book.current.scope() {
                return Err(w::ProtocolError::StaleContext);
            }
            let activity = self
                .registry
                .selected()
                .map_err(commands::domain)?
                .services
                .runtime()
                .enter_owned()
                .map_err(commands::domain)?;
            let pending = Arc::new(Pending {
                envelope: envelope.clone(),
                key: key.clone(),
                request: SessionRequest {
                    command: format!("bootstrap-{}", ag1_storage::digest(key.as_bytes())),
                    target,
                    expected_revision: envelope.expected_revision.get(),
                    timestamp,
                    action,
                },
                source: book.current.clone(),
                intent: Mutex::new(None),
                candidate: Mutex::new(None),
                ready: Mutex::new(None),
                started: AtomicBool::new(false),
                session_start: Arc::new(Mutex::new(StartState::default())),
                running: AtomicBool::new(true),
            });
            book.pending = Some(pending.clone());
            (pending, activity)
        };
        let (send, receive) = tokio::sync::oneshot::channel();
        let owner = self.clone();
        self.handle.spawn(async move {
            let _running = Running(pending.clone());
            let store = owner.store.clone();
            let admitted_pending = pending.clone();
            let admitted = owner
                .administrative
                .submit(move || {
                    let _activity = activity;
                    let envelope = &admitted_pending.envelope;
                    let result = store.admit_session_command(
                        envelope.scope.session.as_str(),
                        &admitted_pending.key,
                        envelope.expected_revision.get(),
                        &serde_json::to_value(envelope).map_err(|_| DomainError::Conflict)?,
                    )?;
                    if !result.inserted {
                        return Err(DomainError::DurabilityUnknown);
                    }
                    *admitted_pending
                        .intent
                        .lock()
                        .map_err(|_| DomainError::Conflict)? = Some(result.record.intent);
                    admitted_pending
                        .source
                        .publish_lifecycle(w::OperationView {
                            operation: operation(&admitted_pending.key),
                            status: w::OperationStatus::Queued,
                        })?;
                    Ok(())
                })
                .await;
            let _ = send.send(
                admitted
                    .clone()
                    .map(|_| w::CommandResult::Accepted {
                        operation: operation(&pending.key),
                    })
                    .map_err(commands::domain),
            );
            if admitted.is_ok() {
                pending.started.store(true, Ordering::Release);
                let _ = owner.execute(&pending).await;
            }
        });
        receive.await.map_err(|_| unavailable())?
    }
    fn receipt_result(
        &self,
        scope: &w::Scope,
        key: &str,
        record: &ag1_storage::CommandResult,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        if record.state == "completed" {
            let receipt: SessionReceipt =
                serde_json::from_slice(record.result.as_deref().ok_or_else(unavailable)?)
                    .map_err(|_| unavailable())?;
            if receipt.version != 1
                || receipt.source != *scope
                || receipt.operation.operation != operation(key)
                || receipt.operation.status == w::OperationStatus::Queued
            {
                return Err(unavailable());
            }
            return Ok(w::CommandResult::Settled {
                operation: receipt.operation,
            });
        }
        let book = self.book.lock().map_err(|_| unavailable())?;
        if book.pending.as_ref().is_some_and(|p| {
            p.key == key && p.envelope.scope == *scope && p.running.load(Ordering::Acquire)
        }) {
            Ok(w::CommandResult::Accepted {
                operation: operation(key),
            })
        } else {
            Err(unavailable())
        }
    }
    async fn execute(self: &Arc<Self>, pending: &Arc<Pending>) -> Result<(), DomainError> {
        let factory = CommandFactory {
            owner: self.clone(),
            pending: pending.clone(),
        };
        let result = self
            .registry
            .change_session(
                pending.envelope.scope.epoch.get(),
                pending.request.clone(),
                &factory,
            )
            .await;
        if result.is_err() && self.registry.pending_selection()?.is_some() {
            return Err(DomainError::DurabilityUnknown);
        }
        let (commands, status) = if result.is_ok() {
            let commands = pending
                .candidate
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .clone()
                .ok_or(DomainError::Conflict)?;
            let status = self
                .start_session(pending, self.registry.selected()?)
                .await?;
            (commands, status)
        } else {
            (
                pending.source.clone(),
                if matches!(result, Err(DomainError::Cancelled)) {
                    w::OperationStatus::Cancelled
                } else {
                    w::OperationStatus::Failed
                },
            )
        };
        self.ready(pending, commands, status)?;
        self.publish(pending).await
    }
    /// Modes bind session_start after runtime replacement, as in the source RPC
    /// and print rebindSession paths. The prepared queue remains private until
    /// callback completion and terminal custody; callback work is never replayed.
    async fn start_session(
        &self,
        pending: &Arc<Pending>,
        selection: Selection,
    ) -> Result<w::OperationStatus, DomainError> {
        let runtime = selection.services.runtime().clone();
        let tools = selection.services.tools().clone();
        if !tools.handles_session_event("session_start")
            && !tools.handles_session_event("resources_discover")
        {
            return Ok(w::OperationStatus::Completed);
        }
        let frozen = runtime.freeze()?;
        if !frozen.idle()? {
            return Err(DomainError::Conflict);
        }
        {
            let mut state = pending
                .session_start
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            if let Some(status) = &state.result {
                return Ok(status.clone());
            }
            if state.attempted {
                return Err(DomainError::Conflict);
            }
            state.attempted = true;
        }
        let starting = Starting(pending.session_start.clone());
        let reason = match pending.request.action {
            SessionAction::New => "new",
            SessionAction::Resume => "resume",
            SessionAction::Fork { .. } => "fork",
        };
        let event = json!({"type":"session_start","reason":reason,"previousSessionId":pending.envelope.scope.session});
        let command = format!("resources-{}", ag1_storage::digest(pending.key.as_bytes()));
        let timestamp = pending.request.timestamp.clone();
        let pool = runtime.pool.clone();
        pool.submit(move || {
            let _frozen = frozen;
            let status = if runtime
                .bind_client_sync(tools.as_ref(), &event, &command, &timestamp)
                .is_ok()
            {
                w::OperationStatus::Completed
            } else {
                w::OperationStatus::Failed
            };
            starting.0.lock().map_err(|_| DomainError::Conflict)?.result = Some(status.clone());
            Ok(status)
        })
        .await
    }
    fn ready(
        &self,
        pending: &Pending,
        commands: Arc<SessionCommands>,
        status: w::OperationStatus,
    ) -> Result<(), DomainError> {
        let receipt = SessionReceipt {
            version: 1,
            operation: w::OperationView {
                operation: operation(&pending.key),
                status: status.clone(),
            },
            source: pending.envelope.scope.clone(),
            selected: commands.scope().clone(),
            journal_revision: commands.snapshot()?.revision.get(),
            selected_entry: if status == w::OperationStatus::Completed {
                match &pending.request.action {
                    SessionAction::Fork { entry } => Some(entry.clone()),
                    _ => None,
                }
            } else {
                None
            },
        };
        let bytes = serde_json::to_vec(&receipt).map_err(|_| DomainError::Conflict)?;
        *pending.ready.lock().map_err(|_| DomainError::Conflict)? = Some(Ready {
            receipt,
            bytes,
            commands,
        });
        Ok(())
    }
    async fn publish(&self, pending: &Arc<Pending>) -> Result<(), DomainError> {
        let store = self.store.clone();
        let key = pending.key.clone();
        let intent = pending
            .intent
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clone()
            .ok_or(DomainError::DurabilityUnknown)?;
        let bytes = pending
            .ready
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
            .ok_or(DomainError::DurabilityUnknown)?
            .bytes
            .clone();
        self.administrative
            .submit(move || {
                store.settle_intent(&key, &intent, &bytes)?;
                Ok(())
            })
            .await?;
        let mut book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        if book
            .pending
            .as_ref()
            .is_none_or(|p| !Arc::ptr_eq(p, pending))
        {
            return Err(DomainError::Conflict);
        }
        let mut ready = pending.ready.lock().map_err(|_| DomainError::Conflict)?;
        let next = ready.as_ref().ok_or(DomainError::Conflict)?;
        next.commands
            .publish_lifecycle(next.receipt.operation.clone())?;
        let ready = ready.take().ok_or(DomainError::Conflict)?;
        book.current = ready.commands;
        book.pending = None;
        Ok(())
    }
    /// Explicit live-owner recovery only. A known terminal receipt or uncertain
    /// selection can be reconciled; an orphan admission never starts execution.
    pub async fn reconcile(self: &Arc<Self>) -> Result<(), DomainError> {
        let pending = self
            .book
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .pending
            .clone()
            .ok_or(DomainError::NotFound)?;
        pending
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| DomainError::Conflict)?;
        let _running = Running(pending.clone());
        if !pending.started.load(Ordering::Acquire)
            && pending
                .ready
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .is_none()
        {
            // This retained live driver never entered execution. A missing
            // admission can be released; a committed but unacknowledged intent
            // is cancelled with custody. Neither case executes an orphan.
            let store = self.store.clone();
            let key = pending.key.clone();
            let record = self
                .administrative
                .submit(move || Ok(store.command_result(&key)?))
                .await?;
            if let Some(record) = record {
                if record.state != "intent" {
                    return Err(DomainError::DurabilityUnknown);
                }
                let expected = json!({"version":1,"kind":"native-session-command","session":pending.envelope.scope.session,
                    "expectedRevision":pending.envelope.expected_revision,"payload":pending.envelope});
                let actual: Value =
                    serde_json::from_slice(&record.intent).map_err(|_| DomainError::Conflict)?;
                if actual != expected {
                    return Err(DomainError::Conflict);
                }
                *pending.intent.lock().map_err(|_| DomainError::Conflict)? = Some(record.intent);
                self.ready(
                    &pending,
                    pending.source.clone(),
                    w::OperationStatus::Cancelled,
                )?;
            } else {
                let mut book = self.book.lock().map_err(|_| DomainError::Conflict)?;
                if book
                    .pending
                    .as_ref()
                    .is_none_or(|p| !Arc::ptr_eq(p, &pending))
                {
                    return Err(DomainError::Conflict);
                }
                book.pending = None;
                return Ok(());
            }
        }
        if pending
            .ready
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_none()
        {
            let selected = if self.registry.pending_selection()?.is_some() {
                self.registry.reconcile_selection().await?
            } else {
                self.registry.selected()?
            };
            let commands = pending
                .candidate
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .clone()
                .ok_or(DomainError::DurabilityUnknown)?;
            if selected.epoch != commands.scope().epoch.get()
                || selected
                    .services
                    .runtime()
                    .state()?
                    .stamp()
                    .session()
                    .as_str()
                    != commands.scope().session.as_str()
            {
                return Err(DomainError::Conflict);
            }
            let status = self.start_session(&pending, selected).await?;
            self.ready(&pending, commands, status)?;
        }
        self.publish(&pending).await
    }
}
struct CommandFactory {
    owner: Arc<RuntimeCommands>,
    pending: Arc<Pending>,
}
impl RuntimeFactory for CommandFactory {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            let prepared = self.owner.factory.prepare(target, epoch).await?;
            let shared = Arc::new(PreparedRuntime::from_shared(
                prepared.runtime().clone(),
                prepared.provider().clone(),
                prepared.tools().clone(),
            ));
            let commands = SessionCommands::new(
                Selection {
                    epoch,
                    services: shared,
                },
                self.owner.instance.clone(),
                self.owner.clock.clone(),
                self.owner.settings.clone(),
            )?;
            *self
                .pending
                .candidate
                .lock()
                .map_err(|_| DomainError::Conflict)? = Some(commands);
            Ok(prepared)
        })
    }
}
