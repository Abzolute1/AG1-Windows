use super::{Listener, NativeSession, SessionEvent};
use ag1_core::{
    DomainError,
    runtime::{
        recovery::{Decision, Owner},
        retry,
    },
};
use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub(super) struct RetryInput<'a> {
    pub command: String,
    pub failed_id: &'a str,
    pub message: &'a Value,
    pub attempt: u32,
    pub settings: &'a retry::Settings,
    pub cancelled: &'a AtomicBool,
}
pub(super) struct RetryContinuation {
    pub stamp: ag1_core::runtime::boundary::Stamp,
    pub checkpoint: ag1_core::runtime::session::BranchCheckpoint,
    pub command: String,
}
pub(super) struct RecoveryLease<'a> {
    pub session: &'a NativeSession,
    pub owner: Owner,
}
impl Drop for RecoveryLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut recovery) = self.session.recovery.lock() {
            recovery.clear(Some(self.owner));
        }
        self.session.changed.notify_waiters();
    }
}
impl NativeSession {
    pub(super) async fn retry_failure(
        &self,
        input: RetryInput<'_>,
        listener: &dyn Listener,
    ) -> Result<RetryContinuation, DomainError> {
        let message = input.message["errorMessage"]
            .as_str()
            .unwrap_or("Unknown error");
        let delay = retry::backoff_ms(message, input.attempt, input.settings.base_delay_ms)?;
        let stamp = self.state()?.stamp().clone();
        if !self
            .recovery
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .schedule(Owner::Retry, &stamp, 0, delay)?
        {
            return Err(DomainError::Conflict);
        }
        let _lease = RecoveryLease {
            session: self,
            owner: Owner::Retry,
        };
        Self::emit(
            listener,
            SessionEvent::Recovery(
                json!({"type":"auto_retry_start","attempt":input.attempt,"maxAttempts":input.settings.max_retries,"delayMs":delay,"errorMessage":message}),
            ),
        );
        let work = async {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(delay);
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if input.cancelled.load(Ordering::Acquire) {
                    return Err(DomainError::Cancelled);
                }
                if self.state()?.stamp() != &stamp {
                    return Err(DomainError::Conflict);
                }
                tokio::select! {_ = tokio::time::sleep_until(deadline) => break, _ = changed => {}}
            }
            let current = self.state()?.stamp().clone();
            if self
                .recovery
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .poll(delay, &current, false)
                != Some(Decision::Run(Owner::Retry))
            {
                return Err(DomainError::Conflict);
            }
            let branch =
                self.state()?
                    .branch_before_failure(&input.command, input.failed_id, &stamp)?;
            let checkpoint = branch.checkpoint();
            let stamp = self.commit_branch(branch, listener).await?;
            Ok(RetryContinuation {
                stamp,
                checkpoint,
                command: format!("{}-restore", input.command),
            })
        }
        .await;
        if let Err(error) = &work {
            Self::emit(
                listener,
                SessionEvent::Recovery(
                    json!({"type":"auto_retry_end","success":false,"attempt":input.attempt,
                "finalError":if *error==DomainError::Cancelled {"Retry cancelled"} else {"Retry skipped because the session changed before replay."}}),
                ),
            );
        }
        work
    }
}
