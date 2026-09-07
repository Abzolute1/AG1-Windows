//! Explicit client binding, separate from preparing an SDK runtime. The worker
//! retains the callback result if the client drops its initialization future.
use super::*;

impl RuntimeCommands {
    pub(super) fn check_initialization(&self) -> Result<(), DomainError> {
        let state = self
            .initialization
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if state.attempted && state.result != Some(w::OperationStatus::Completed) {
            return Err(DomainError::Unavailable(
                "native client initialization".into(),
            ));
        }
        Ok(())
    }

    /// Bind the initial client before accepting input. Like the source SDK,
    /// constructing the runtime alone does not emit session_start. Subsequent
    /// calls inspect the retained result; they never replay the callback.
    pub async fn initialize(self: &Arc<Self>) -> Result<(), DomainError> {
        let (runtime, tools, frozen, starting) = {
            let book = self.book.lock().map_err(|_| DomainError::Conflict)?;
            if book.pending.is_some() {
                return Err(DomainError::Conflict);
            }
            let mut state = self
                .initialization
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            if let Some(status) = &state.result {
                return if *status == w::OperationStatus::Completed {
                    Ok(())
                } else {
                    Err(DomainError::Unavailable(
                        "native client initialization".into(),
                    ))
                };
            }
            let snapshot = book.current.snapshot()?;
            if state.attempted || !snapshot.queued.is_empty() || snapshot.watermark.get() != 0 {
                return Err(DomainError::Conflict);
            }
            let selected = self.registry.selected()?;
            if selected.epoch != book.current.scope().epoch.get() {
                return Err(DomainError::Conflict);
            }
            let runtime = selected.services.runtime().clone();
            if !runtime.settled()? {
                return Err(DomainError::Conflict);
            }
            // Fence callers that obtained the queue just before binding began.
            // Initialization never cancels or drains already accepted input.
            let frozen = runtime.freeze()?;
            let snapshot = book.current.snapshot()?;
            if !frozen.idle()? || !snapshot.queued.is_empty() || snapshot.watermark.get() != 0 {
                return Err(DomainError::Conflict);
            }
            state.attempted = true;
            (
                runtime,
                selected.services.tools().clone(),
                frozen,
                Starting(self.initialization.clone()),
            )
        };
        let pool = runtime.pool.clone();
        let owner = self.clone();
        let timestamp = i64::try_from(self.clock.now_ms())
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .ok_or(DomainError::Conflict)?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let command = format!(
            "resources-{}",
            ag1_storage::digest(
                json!([
                    self.instance,
                    runtime.workspace,
                    runtime.state()?.stamp().session().as_str(),
                    self.registry.selected()?.epoch,
                    "startup"
                ])
                .to_string()
                .as_bytes()
            )
        );
        pool.submit(move || {
            let (_owner, _frozen) = (owner, frozen);
            let result = runtime.bind_client_sync(
                tools.as_ref(),
                &json!({"type":"session_start", "reason":"startup"}),
                &command,
                &timestamp,
            );
            starting.0.lock().map_err(|_| DomainError::Conflict)?.result =
                Some(if result.is_ok() {
                    w::OperationStatus::Completed
                } else {
                    w::OperationStatus::Failed
                });
            result.map(|_| ())
        })
        .await
    }
}
