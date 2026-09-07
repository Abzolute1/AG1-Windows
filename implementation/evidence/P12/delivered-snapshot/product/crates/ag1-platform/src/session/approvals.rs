//! Bounded host approval mailbox. P09 retains and consumes the actual capability;
//! a client answer only releases its wait after command intent is acknowledged.
use super::files::ApprovalHandler;
use ag1_core::{
    Clock, DomainError,
    tools::approval::{ApprovalRequest, Binding},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

struct Pending {
    request: ApprovalRequest,
    answer: Option<bool>,
}
pub struct PendingApprovals {
    pending: Mutex<BTreeMap<String, Pending>>,
    changed: Condvar,
    clock: Arc<dyn Clock>,
}
impl PendingApprovals {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            pending: Mutex::new(BTreeMap::new()),
            changed: Condvar::new(),
            clock,
        }
    }
}
impl ApprovalHandler for PendingApprovals {
    fn supports_commands(&self) -> bool {
        true
    }

    fn pending(&self) -> Result<Vec<ApprovalRequest>, DomainError> {
        Ok(self
            .pending
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .values()
            .filter(|p| p.answer.is_none() && p.request.binding.expires_ms > self.clock.now_ms())
            .map(|p| p.request.clone())
            .collect())
    }

    fn answer(&self, binding: &Binding, allow: bool) -> Result<(), DomainError> {
        let mut pending = self.pending.lock().map_err(|_| DomainError::Conflict)?;
        let request = pending.get_mut(&binding.id).ok_or(DomainError::NotFound)?;
        if request.request.binding != *binding
            || request.answer.is_some()
            || binding.expires_ms <= self.clock.now_ms()
        {
            return Err(DomainError::Conflict);
        }
        request.answer = Some(allow);
        self.changed.notify_all();
        Ok(())
    }

    fn approve(&self, request: &ApprovalRequest, cancel: &AtomicBool) -> Result<bool, DomainError> {
        let ttl = request
            .binding
            .expires_ms
            .saturating_sub(self.clock.now_ms());
        if ttl == 0 || ttl > 300_000 {
            return Err(DomainError::TimedOut);
        }
        if serde_json::to_vec(&request.display)
            .map_err(|_| DomainError::Conflict)?
            .len()
            > 32 * 1024
        {
            return Err(DomainError::LimitExceeded("approval display".into()));
        }
        let deadline = Instant::now() + Duration::from_millis(ttl);
        let mut pending = self.pending.lock().map_err(|_| DomainError::Conflict)?;
        if pending.len() >= 256 {
            return Err(DomainError::QueueFull);
        }
        if pending.contains_key(&request.binding.id) {
            return Err(DomainError::Conflict);
        }
        pending.insert(
            request.binding.id.clone(),
            Pending {
                request: request.clone(),
                answer: None,
            },
        );
        let result = loop {
            if cancel.load(Ordering::Acquire) {
                break Err(DomainError::Cancelled);
            }
            if Instant::now() >= deadline || self.clock.now_ms() >= request.binding.expires_ms {
                break Err(DomainError::TimedOut);
            }
            if let Some(answer) = pending.get(&request.binding.id).and_then(|p| p.answer) {
                break Ok(answer);
            }
            pending = self
                .changed
                .wait_timeout(pending, Duration::from_millis(20))
                .map_err(|_| DomainError::Conflict)?
                .0;
        };
        pending.remove(&request.binding.id);
        result
    }
}
