//! Admission fencing is separate from provider cancellation: accepted work
//! keeps its receipt path while a replacement prevents new public mutations.
use super::NativeSession;
use ag1_core::DomainError;
use std::sync::Arc;

impl NativeSession {
    /// Client binding owns a frozen, idle runtime. Resource publication uses its
    /// existing append/acknowledgement path directly on this blocking worker.
    pub(super) fn bind_client_sync(
        &self,
        tools: &dyn super::ToolRunner,
        event: &serde_json::Value,
        command: &str,
        timestamp: &str,
    ) -> Result<(), DomainError> {
        if self.accepts_commands()? || !self.settled()? {
            return Err(DomainError::Conflict);
        }
        self.lifecycle_event_sync(tools, event)?;
        if !tools.handles_session_event("resources_discover") {
            return Ok(());
        }
        let (scope, snapshot) = self.lifecycle_snapshot()?;
        let Some(prompt) = tools.discover_session_resources(&scope, &snapshot)? else {
            return Ok(());
        };
        let action = {
            let mut state = self.state()?;
            if state.journal_revision() != scope.revision || state.stamp().epoch() != scope.epoch {
                return Err(DomainError::Conflict);
            }
            state.install_prompt(command, command, timestamp, prompt)?
        };
        let mut wait = super::CommitWait {
            state: self.state.clone(),
            changed: self.changed.clone(),
            expected_revision: scope.revision,
            received: false,
        };
        let receipt = action.execute(self.store.as_ref());
        let result = self.acknowledge_stamped(Ok(receipt), &|_: &super::SessionEvent| {});
        wait.received = true;
        result.map(|_| ())
    }
    fn lifecycle_snapshot(
        &self,
    ) -> Result<(ag1_core::tools::registry::Scope, serde_json::Value), DomainError> {
        let idle = self
            .active
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_none();
        let pending = self
            .command_owner
            .get()
            .and_then(std::sync::Weak::upgrade)
            .map(|commands| commands.has_pending_messages())
            .transpose()?
            .unwrap_or(false);
        let state = self.state()?;
        let scope = ag1_core::tools::registry::Scope {
            workspace: self.workspace.clone(),
            session: state.stamp().session().as_str().into(),
            epoch: state.stamp().epoch(),
            revision: state.journal_revision(),
        };
        let branch = state.branch_snapshot();
        let configuration = state.configuration().clone();
        drop(state);
        use ag1_core::storage::StoragePort;
        let stored = self
            .store
            .journal_snapshot(&ag1_core::SessionId::new(&scope.session)?)?;
        if stored.revision != scope.revision {
            return Err(DomainError::Conflict);
        }
        let snapshot = super::extensions::snapshot::configuration(
            &configuration,
            super::extensions::snapshot::session(&stored, &branch)?,
            idle,
            pending,
        );
        Ok((scope, snapshot))
    }
    pub(super) async fn client_resource(
        self: &Arc<Self>,
        tools: Arc<dyn super::ToolRunner>,
        operation: String,
        name: String,
        args: serde_json::Value,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<super::TurnResult, DomainError> {
        let service = tools.client_resources().ok_or_else(|| {
            DomainError::Unavailable("Resource helpers are not configured".into())
        })?;
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        self.pool
            .submit(move || {
                let _activity = activity;
                let _mode =
                    super::print::ModeLease::begin_with_cancel(&runtime, None, cancel.clone())?;
                let (scope, _) = runtime.lifecycle_snapshot()?;
                let result = service.execute(&scope, &operation, &name, &args, &cancel)?;
                if result["record"]["status"] != "completed" || result["result"]["isError"] == true
                {
                    return Err(DomainError::Unavailable(format!(
                        "Resource action ended with {}",
                        result["record"]["status"]
                    )));
                }
                Ok(super::TurnResult {
                    generations: 0,
                    journal_revision: runtime.journal_revision()?,
                    assistant: result,
                })
            })
            .await
    }
    pub(super) async fn client_extension(
        self: &Arc<Self>,
        tools: Arc<dyn super::ToolRunner>,
        method: String,
        args: serde_json::Value,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<super::TurnResult, DomainError> {
        let extension = tools
            .client_extensions()
            .ok_or_else(|| DomainError::Unavailable("No admitted extension host".into()))?;
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        self.pool
            .submit(move || {
                let _activity = activity;
                let _mode =
                    super::print::ModeLease::begin_with_cancel(&runtime, None, cancel.clone())?;
                let (scope, snapshot) = runtime.lifecycle_snapshot()?;
                let result = extension.client_call(
                    &scope,
                    &snapshot,
                    &method,
                    &args,
                    &crate::resource_fs::ScanControl {
                        cancel: &cancel,
                        deadline: std::time::Instant::now() + std::time::Duration::from_secs(300),
                    },
                )?;
                if runtime.journal_revision()? != scope.revision
                    || runtime.context_epoch()? != scope.epoch
                {
                    return Err(DomainError::Conflict);
                }
                Ok(super::TurnResult {
                    generations: 0,
                    journal_revision: scope.revision,
                    assistant: result.unwrap_or(serde_json::Value::Null),
                })
            })
            .await
    }
    pub(super) fn lifecycle_event_sync(
        &self,
        tools: &dyn super::ToolRunner,
        event: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        if !tools.handles_session_event(event["type"].as_str().unwrap_or("")) {
            return Ok(None);
        }
        let (scope, snapshot) = self.lifecycle_snapshot()?;
        let result = tools.session_event(&scope, &snapshot, event)?;
        if self.journal_revision()? != scope.revision
            || self.state()?.stamp().epoch() != scope.epoch
        {
            return Err(DomainError::Conflict);
        }
        Ok(result)
    }
    pub(super) async fn lifecycle_event(
        self: &Arc<Self>,
        tools: Arc<dyn super::ToolRunner>,
        event: serde_json::Value,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        if !tools.handles_session_event(event["type"].as_str().unwrap_or("")) {
            return Ok(None);
        }
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        self.pool
            .submit(move || {
                let _activity = activity;
                runtime.lifecycle_event_sync(tools.as_ref(), &event)
            })
            .await
    }
}

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Active,
    Frozen,
    Retired,
}
#[derive(Default)]
pub(super) struct Lifecycle {
    phase: Phase,
    operations: usize,
    workers: usize,
}
pub(super) struct Activity<'a>(&'a NativeSession);
pub(super) struct OwnedActivity(Arc<NativeSession>);
pub(super) struct WorkerActivity {
    lifecycle: Arc<std::sync::Mutex<Lifecycle>>,
    changed: Arc<tokio::sync::Notify>,
}
impl Drop for WorkerActivity {
    fn drop(&mut self) {
        if let Ok(mut state) = self.lifecycle.lock() {
            state.operations -= 1;
            state.workers -= 1;
        }
        self.changed.notify_waiters();
    }
}
impl Drop for OwnedActivity {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lifecycle.lock() {
            state.operations -= 1;
        }
        self.0.changed.notify_waiters();
    }
}
impl Drop for Activity<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lifecycle.lock() {
            state.operations -= 1;
        }
        self.0.changed.notify_waiters();
    }
}
pub(super) struct Frozen {
    session: Arc<NativeSession>,
    finished: bool,
}
impl NativeSession {
    /// Descendant of already accepted work, including its receipt path during
    /// freeze. This is not admission for a new external command.
    pub(super) fn retain_work(&self) -> Result<WorkerActivity, DomainError> {
        let mut state = self.lifecycle.lock().map_err(|_| DomainError::Conflict)?;
        if state.phase == Phase::Retired || state.operations == 0 {
            return Err(DomainError::Conflict);
        }
        state.operations = state
            .operations
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        state.workers += 1;
        Ok(WorkerActivity {
            lifecycle: self.lifecycle.clone(),
            changed: self.changed.clone(),
        })
    }
    pub(super) fn background_work_settled(&self) -> Result<bool, DomainError> {
        Ok(self
            .lifecycle
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .workers
            == 0)
    }
    pub(super) fn enter_owned(self: &Arc<Self>) -> Result<OwnedActivity, DomainError> {
        let mut state = self.lifecycle.lock().map_err(|_| DomainError::Conflict)?;
        if state.phase != Phase::Active {
            return Err(DomainError::Conflict);
        }
        state.operations = state
            .operations
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        Ok(OwnedActivity(self.clone()))
    }
    /// Display hint only; public mutations atomically acquire their own lease.
    pub fn accepts_commands(&self) -> Result<bool, DomainError> {
        Ok(self
            .lifecycle
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .phase
            == Phase::Active)
    }
    pub(super) fn enter(&self) -> Result<Activity<'_>, DomainError> {
        let mut state = self.lifecycle.lock().map_err(|_| DomainError::Conflict)?;
        if state.phase != Phase::Active {
            return Err(DomainError::Conflict);
        }
        state.operations = state
            .operations
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        Ok(Activity(self))
    }
    pub(super) fn freeze(self: &Arc<Self>) -> Result<Frozen, DomainError> {
        let mut state = self.lifecycle.lock().map_err(|_| DomainError::Conflict)?;
        if state.phase != Phase::Active {
            return Err(DomainError::Conflict);
        }
        state.phase = Phase::Frozen;
        Ok(Frozen {
            session: self.clone(),
            finished: false,
        })
    }
}
impl Frozen {
    pub(super) fn idle(&self) -> Result<bool, DomainError> {
        Ok(self
            .session
            .lifecycle
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .operations
            == 0)
    }
    /// No deadline substitutes for settlement. An unresolved transport or lost
    /// writer receipt requires explicit recovery once public calls have drained.
    pub(super) async fn drain(&self) -> Result<(), DomainError> {
        self.session.cancel()?;
        loop {
            let changed = self.session.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.idle()? {
                return if self.session.settled()? && self.session.persistence_healthy()? {
                    Ok(())
                } else {
                    Err(DomainError::Unavailable(
                        "session requires settlement recovery before replacement".into(),
                    ))
                };
            }
            changed.await;
        }
    }
    pub(super) fn retire(mut self) -> Result<(), DomainError> {
        // Retirement is irrevocable even if an invariant or poisoned lock
        // prevents completion. Never thaw an owner after a durable cutover.
        self.finished = true;
        let mut state = self
            .session
            .lifecycle
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if state.phase != Phase::Frozen || state.operations != 0 {
            return Err(DomainError::Conflict);
        }
        state.phase = Phase::Retired;
        Ok(())
    }
}
impl Drop for Frozen {
    fn drop(&mut self) {
        if !self.finished
            && let Ok(mut state) = self.session.lifecycle.lock()
            && state.phase == Phase::Frozen
        {
            state.phase = Phase::Active;
        }
        self.session.changed.notify_waiters();
    }
}
