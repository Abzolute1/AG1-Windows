//! A tool batch borrows the existing session owner; it never creates a second
//! writer. Blocking tool workers execute its opaque append actions directly,
//! avoiding nested submission/deadlock when the host has a one-worker pool.
use super::{NativeSession, SessionEvent};
use ag1_core::{
    DomainError,
    runtime::{
        boundary::Stamp,
        session::{AppendAction, SessionRuntime},
    },
    tools::{
        executor::Outcome,
        registry::{Origin, Scope, ValidatedCall},
    },
};
use ag1_storage::Store;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
struct Book {
    stamp: Stamp,
    revision: u64,
    valid: bool,
    published: Vec<(String, String)>,
}
type ControlResult = Result<(Stamp, u64, u64), DomainError>;
struct Control {
    receipt: super::CommitWait,
    action: AppendAction,
    reply: tokio::sync::oneshot::Sender<ControlResult>,
}
#[derive(Default)]
struct Controls {
    closed: bool,
    pending: std::collections::VecDeque<Control>,
}
/// Closing never performs I/O on the async host, including queue rejection or
/// worker panic. Accepted but unacknowledged commands report unknown durability.
pub(super) struct ControlGuard(pub Arc<ToolJournal>);
impl Drop for ControlGuard {
    fn drop(&mut self) {
        if let Ok(mut controls) = self.0.controls.lock() {
            controls.closed = true;
            for control in controls.pending.drain(..) {
                let _ = control.reply.send(Err(DomainError::DurabilityUnknown));
            }
        }
    }
}
#[derive(Debug)]
pub enum JournalMutationError {
    BeforeEffect(DomainError),
    Persistence(DomainError),
}
impl From<DomainError> for JournalMutationError {
    fn from(error: DomainError) -> Self {
        Self::BeforeEffect(error)
    }
}
impl From<JournalMutationError> for DomainError {
    fn from(error: JournalMutationError) -> Self {
        match error {
            JournalMutationError::BeforeEffect(error)
            | JournalMutationError::Persistence(error) => error,
        }
    }
}
pub struct ToolJournal {
    state: Arc<Mutex<SessionRuntime>>,
    store: Arc<Store>,
    revision: Arc<AtomicU64>,
    changed: Arc<tokio::sync::Notify>,
    workspace: String,
    timestamp: String,
    fallback_ms: u64,
    result_prefix: String,
    book: Mutex<Book>,
    serial: Mutex<()>,
    events: tokio::sync::mpsc::UnboundedSender<SessionEvent>,
    delegation_progress_pending: AtomicBool,
    pub(super) endpoint: String,
    pub(super) isolated: tokio::sync::mpsc::UnboundedSender<super::tool_inference::Request>,
    controls: Mutex<Controls>,
    cancelled: Arc<AtomicBool>,
    pub(super) repetition: Arc<super::repetition::Guard>,
    pub(super) failures: Arc<Mutex<ag1_core::tools::recovery::FailureTracker>>,
    pub(super) controlled: Option<super::controlled::Shared>,
    pub(super) serialize_controlled: bool,
    pub(super) completion: Option<super::completion_evidence::Shared>,
    commands: Option<std::sync::Weak<super::commands::SessionCommands>>,
}
pub(super) type JournalBatch = (
    Arc<ToolJournal>,
    tokio::sync::mpsc::UnboundedReceiver<SessionEvent>,
    tokio::sync::mpsc::UnboundedReceiver<super::tool_inference::Request>,
);
pub(super) struct RuntimePolicies {
    pub repetition: Arc<super::repetition::Guard>,
    pub controlled: Option<super::controlled::Shared>,
    pub completion: Option<super::completion_evidence::Shared>,
    pub commands: Option<std::sync::Weak<super::commands::SessionCommands>>,
}
impl ToolJournal {
    /// Advisory Task updates are coalesced by their producer and may be dropped
    /// under listener backpressure. Durable result publication is separate.
    pub(super) fn delegation_progress(
        &self,
        call: &ValidatedCall,
        progress: &super::subagent::Progress,
    ) {
        if progress.model.is_empty()
            || self
                .delegation_progress_pending
                .swap(true, Ordering::AcqRel)
        {
            return;
        }
        let line = ag1_core::runtime::subagent::tool::progress_line(
            progress.turn,
            progress.tool_calls,
            progress.tokens,
            progress.elapsed_s,
            &progress.last_action,
        );
        let _ = self.events.send(SessionEvent::Recovery(json!({
            "type":"tool_execution_update", "toolName":call.name(), "toolCallId":call.call_id(),
            "partialResult":{"content":[{"type":"text","text":line}],
                "details":{"description":call.args()["description"],"type":call.args()["type"],
                    "state":"running","model":progress.model,"progress":progress}}
        })));
    }
    pub(super) fn event_dequeued(&self, event: &SessionEvent) {
        if matches!(event, SessionEvent::Recovery(value)
            if value["type"] == "tool_execution_update" && matches!(value["toolName"].as_str(), Some("task" | "explore")))
        {
            self.delegation_progress_pending
                .store(false, Ordering::Release);
        }
    }
    pub(super) fn repetition_events(&self, events: Vec<ag1_core::runtime::repetition::Event>) {
        for event in events {
            let mut event = serde_json::to_value(event).expect("bounded repetition event");
            event["type"] = json!("repetition_guard");
            let _ = self.events.send(SessionEvent::Recovery(event));
        }
    }
    pub(super) fn pending_messages(&self) -> Result<bool, DomainError> {
        self.commands
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .map_or(Ok(false), |commands| commands.has_pending_messages())
    }
    pub fn timestamp(&self) -> &str {
        &self.timestamp
    }
    pub(super) fn new(
        host: &NativeSession,
        result_prefix: String,
        timestamp: String,
        fallback_ms: u64,
        endpoint: String,
        cancelled: Arc<AtomicBool>,
        policies: RuntimePolicies,
    ) -> Result<JournalBatch, DomainError> {
        let RuntimePolicies {
            repetition,
            controlled,
            completion,
            commands,
        } = policies;
        let state = host.state()?;
        let book = Book {
            stamp: state.stamp().clone(),
            revision: state.journal_revision(),
            valid: true,
            published: vec![],
        };
        let (events, receiver) = tokio::sync::mpsc::unbounded_channel();
        let (isolated, requests) = tokio::sync::mpsc::unbounded_channel();
        Ok((
            Arc::new(Self {
                state: host.state.clone(),
                store: host.store.clone(),
                revision: host.revision.clone(),
                changed: host.changed.clone(),
                workspace: host.workspace.clone(),
                timestamp,
                fallback_ms,
                result_prefix,
                book: Mutex::new(book),
                serial: Mutex::new(()),
                events,
                delegation_progress_pending: AtomicBool::new(false),
                endpoint,
                isolated,
                controls: Mutex::new(Controls::default()),
                cancelled,
                repetition,
                failures: host.failures.clone(),
                serialize_controlled: controlled.is_some()
                    && super::controlled::serialize(&state.configuration().model),
                controlled,
                completion,
                commands,
            }),
            receiver,
            requests,
        ))
    }
    pub fn revision(&self) -> Result<u64, DomainError> {
        Ok(self
            .book
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .revision)
    }
    pub(super) fn enqueue_control(
        &self,
        action: AppendAction,
    ) -> Result<tokio::sync::oneshot::Receiver<ControlResult>, Box<AppendAction>> {
        let Ok(mut controls) = self.controls.lock() else {
            return Err(Box::new(action));
        };
        if controls.closed {
            return Err(Box::new(action));
        }
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let receipt = super::CommitWait {
            state: self.state.clone(),
            changed: self.changed.clone(),
            expected_revision: action.write().expected,
            received: false,
        };
        controls.pending.push_back(Control {
            receipt,
            action,
            reply,
        });
        // Release cancellable approval/I/O waits so the worker can acknowledge
        // the control command. This cancels only this batch, not the parent turn.
        self.cancel_batch();
        Ok(receiver)
    }
    pub(super) fn cancel_batch(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    /// Called only on a blocking tool worker, including while an isolated model
    /// request is pending. Control appends therefore need no second pool worker.
    pub(super) fn service_controls(&self) -> Result<(), DomainError> {
        self.drain_controls(false)
    }
    pub(super) fn finish_controls(&self) -> Result<(), DomainError> {
        self.drain_controls(true)
    }
    fn drain_controls(&self, close: bool) -> Result<(), DomainError> {
        let _serial = self.serial.lock().map_err(|_| DomainError::Conflict)?;
        let pending = {
            let mut controls = self.controls.lock().map_err(|_| DomainError::Conflict)?;
            controls.closed |= close;
            std::mem::take(&mut controls.pending)
        };
        for mut control in pending {
            let mut book = self.book.lock().map_err(|_| DomainError::Conflict)?;
            book.valid = false;
            let output = self
                .acknowledge_inner(control.action, &mut book, false)
                .and_then(|()| {
                    let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
                    Ok((
                        state.stamp().clone(),
                        state.journal_revision(),
                        state.committed_sequence(),
                    ))
                });
            let failed = output.is_err();
            control.receipt.received = !failed;
            let _ = control.reply.send(output);
            if failed {
                return Err(DomainError::DurabilityUnknown);
            }
        }
        Ok(())
    }
    /// Direct send-time check for an external capability. No journal read/copy or
    /// mutation; the exact batch cancellation and live owner/epoch are authority.
    #[cfg(target_os = "linux")]
    pub(crate) fn check_mcp_scope(&self, scope:&Scope) -> Result<(),DomainError> {
        if self.cancelled.load(Ordering::Acquire) { return Err(DomainError::Cancelled); }
        let book=self.book.lock().map_err(|_|DomainError::Conflict)?;
        let state=self.state.lock().map_err(|_|DomainError::Conflict)?;
        self.validate(scope,&book,&state)
    }
    pub fn valid(&self) -> bool {
        self.book.lock().is_ok_and(|book| book.valid)
    }
    fn validate(
        &self,
        scope: &Scope,
        book: &Book,
        state: &SessionRuntime,
    ) -> Result<(), DomainError> {
        if !book.valid
            || self.workspace != scope.workspace
            || scope.session != state.stamp().session().as_str()
            || scope.epoch != state.stamp().epoch()
            || scope.revision != book.revision
            || state.journal_revision() != book.revision
            || state.stamp() != &book.stamp
        {
            Err(DomainError::Conflict)
        } else {
            Ok(())
        }
    }
    pub fn snapshot(&self, scope: &Scope) -> Result<Arc<Vec<Value>>, DomainError> {
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        self.validate(scope, &book, &state)?;
        Ok(state.branch_snapshot())
    }
    pub fn stored_snapshot(
        &self,
        scope: &Scope,
    ) -> Result<ag1_core::storage::StoredJournal, DomainError> {
        use ag1_core::storage::StoragePort;
        let branch = self.snapshot(scope)?;
        let journal = self
            .store
            .journal_snapshot(&ag1_core::SessionId::new(&scope.session)?)?;
        if journal.revision != scope.revision || ag1_core::task::branch_values(&journal)? != *branch
        {
            return Err(DomainError::Conflict);
        }
        self.snapshot(scope)?;
        Ok(journal)
    }
    pub fn model(
        &self,
        scope: &Scope,
    ) -> Result<
        (
            ag1_core::runtime::session::Configuration,
            ag1_core::model::ModelScope,
        ),
        DomainError,
    > {
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        self.validate(scope, &book, &state)?;
        let configuration = state.configuration().clone();
        let model = &configuration.model;
        let scope = ag1_core::model::ModelScope {
            provider: model["provider"].as_str().unwrap_or("").into(),
            model: model["id"].as_str().unwrap_or("").into(),
            endpoint: self.endpoint.clone(),
            runtime_fingerprint: ag1_storage::digest(model.to_string().as_bytes()),
        };
        Ok((configuration, scope))
    }
    pub(super) fn validate_draft(
        &self,
        scope: &Scope,
        call: &ValidatedCall,
    ) -> Result<(), DomainError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        self.validate(scope, &book, &state)?;
        let pending = state.pending_tools().front().ok_or(DomainError::Conflict)?.effective(&state.branch_snapshot())?;
        if call.origin() != Origin::BuiltIn
            || call.name() != "episode"
            || call.args()["action"] != "draft"
            || pending.id != call.call_id()
            || pending.name != call.name()
            || pending.block["arguments"] != *call.args()
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    fn acknowledge(&self, action: AppendAction, book: &mut Book) -> Result<(), DomainError> {
        self.acknowledge_inner(action, book, true)
    }
    fn acknowledge_inner(
        &self,
        action: AppendAction,
        book: &mut Book,
        emit: bool,
    ) -> Result<(), DomainError> {
        let receipt = action.execute(self.store.as_ref());
        let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let result = state.acknowledge(receipt);
        self.changed.notify_waiters();
        if let Err(error) = result {
            book.valid = false;
            return Err(error);
        }
        book.revision = state.journal_revision();
        book.stamp = state.stamp().clone();
        self.revision.store(book.revision, Ordering::Release);
        let event = SessionEvent::Committed {
            revision: book.revision,
            sequence: state.committed_sequence(),
        };
        drop(state);
        if emit {
            let _ = self.events.send(event);
        }
        Ok(())
    }
    /// Validated adapters may publish supported custom records only for the
    /// current pending call, after its P09 intent has acknowledged.
    pub fn commit_state(
        &self,
        scope: &Scope,
        call: &ValidatedCall,
        custom_type: &str,
        data: Value,
    ) -> Result<(), JournalMutationError> {
        let _serial = self.serial.lock().map_err(|_| DomainError::Conflict)?;
        let mut book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let action = {
            let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
            self.validate(scope, &book, &state)?;
            let pending = state.pending_tools().front().ok_or(DomainError::Conflict)?.effective(&state.branch_snapshot())?;
            if call.origin() != Origin::BuiltIn
                || pending.id != call.call_id()
                || pending.name != call.name()
                || pending.block["arguments"] != *call.args()
            {
                return Err(DomainError::PermissionDenied.into());
            }
            let id = format!("{}-state", call.operation());
            state.append_tool_state(&id, &id, &self.timestamp, call.call_id(), custom_type, data)?
        };
        self.acknowledge(action, &mut book)
            .map_err(JournalMutationError::Persistence)
    }
    pub fn apply_todo(
        &self,
        scope: &Scope,
        call: &ValidatedCall,
    ) -> Result<(Value, bool), JournalMutationError> {
        self.apply_derived(scope, call, "todo")
    }
    pub fn apply_observation(
        &self,
        scope: &Scope,
        call: &ValidatedCall,
    ) -> Result<(Value, bool), JournalMutationError> {
        self.apply_derived(scope, call, "runtimeObservation")
    }
    fn apply_derived(
        &self,
        scope: &Scope,
        call: &ValidatedCall,
        name: &str,
    ) -> Result<(Value, bool), JournalMutationError> {
        let _serial = self.serial.lock().map_err(|_| DomainError::Conflict)?;
        let mut book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let (action, result) = {
            let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
            self.validate(scope, &book, &state)?;
            let pending = state.pending_tools().front().ok_or(DomainError::Conflict)?.effective(&state.branch_snapshot())?;
            if call.origin() != Origin::BuiltIn
                || pending.id != call.call_id()
                || call.name() != name
                || pending.name != call.name()
                || pending.block["arguments"] != *call.args()
            {
                return Err(DomainError::PermissionDenied.into());
            }
            let id = format!("{}-state", call.operation());
            state.append_journal_tool(&id, &id, &self.timestamp, call.call_id())?
        };
        let changed = action.is_some();
        if let Some(action) = action {
            self.acknowledge(action, &mut book)
                .map_err(JournalMutationError::Persistence)?;
        }
        Ok((result, changed))
    }
    pub(super) fn publish_result(&self, outcome: &Outcome) -> Result<(), DomainError> {
        self.service_controls()?;
        let _serial = self.serial.lock().map_err(|_| DomainError::Conflict)?;
        let mut book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        let id = format!("{}-{}", self.result_prefix, book.published.len());
        let mut message = json!({"role":"toolResult","toolCallId":outcome.record.call_id,"toolName":outcome.record.tool,"content":outcome.result["content"],"details":outcome.result["details"],"isError":outcome.result["isError"].as_bool().unwrap_or(false),"timestamp":outcome.record.finished_ms.unwrap_or(self.fallback_ms)});
        let action = {
            let mut state = self.state.lock().map_err(|_| DomainError::Conflict)?;
            if !state.stamp().same_owner_epoch(&book.stamp) {
                return Err(DomainError::Conflict);
            }
            // Changed configuration invalidates further execution, while known
            // results still drain through the same current mutation owner.
            if state.stamp() != &book.stamp {
                book.valid = false;
            }
            let pending = state.pending_tools().front().ok_or(DomainError::Conflict)?;
            if pending.id != outcome.record.call_id { return Err(DomainError::Conflict); }
            // Canonical receipts drive verification; providers receive the
            // original advertised name paired with their original call ID.
            message["toolName"] = json!(pending.name);
            if pending.name != outcome.record.tool {
                if !message["details"].is_object() { message["details"] = json!({}); }
                message["details"]["nativeTool"] = json!(outcome.record.tool);
            }
            state.append_message(&id, &id, &self.timestamp, message)?
        };
        self.acknowledge(action, &mut book)?;
        let evidence_id = format!("{id}-inspection");
        let action = self
            .state
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .capture_inspection(&evidence_id, &evidence_id, &self.timestamp, &id)?;
        if let Some(action) = action {
            self.acknowledge(action, &mut book)?;
        }
        book.published.push((
            outcome.record.call_id.clone(),
            ag1_storage::digest(outcome.result.to_string().as_bytes()),
        ));
        Ok(())
    }
    pub(super) fn published(&self, outcome: &Outcome) -> Result<bool, DomainError> {
        let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
        if let Some((_, digest)) = book
            .published
            .iter()
            .find(|(id, _)| id == &outcome.record.call_id)
        {
            if *digest != ag1_storage::digest(outcome.result.to_string().as_bytes()) {
                return Err(DomainError::Conflict);
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
