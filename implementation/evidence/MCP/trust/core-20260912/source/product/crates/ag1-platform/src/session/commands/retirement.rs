//! Queue retirement runs only under the registry's frozen, drained owner.
//! Unstarted input gets a durable cancellation receipt, never a user append.
use super::*;

impl SessionCommands {
    pub(in crate::session) fn retire_pending(&self) -> Result<(), DomainError> {
        if self.runtime().accepts_commands()? {
            return Err(DomainError::Conflict);
        }
        loop {
            let queued = {
                let mut book = self.book()?;
                if book.active.is_some()
                    || book.reserved != 0
                    || book.queue.iter().any(|q| q.claimed && !q.retiring)
                {
                    return Err(DomainError::DurabilityUnknown);
                }
                let Some(queued) = book.queue.front_mut() else {
                    return Ok(());
                };
                // Retain the claim across failed custody. It cannot later be
                // mistaken for ordinary unstarted work and dispatched.
                queued.claimed = true;
                queued.retiring = true;
                queued.cancelled.store(true, Ordering::Release);
                queued.clone()
            };
            let record = self
                .runtime()
                .store
                .command_result(&queued.key)?
                .ok_or(DomainError::DurabilityUnknown)?;
            if record.intent != queued.intent {
                return Err(DomainError::Conflict);
            }
            let operation = if record.state == "completed" {
                // A prior lost acknowledgement needs observation only. The
                // original receipt is immutable even if configuration advanced.
                let operation = Self::terminal(&queued.key, &record)
                    .map_err(|_| DomainError::DurabilityUnknown)?;
                if operation.status != w::OperationStatus::Cancelled {
                    return Err(DomainError::DurabilityUnknown);
                }
                operation
            } else {
                let operation = w::OperationView {
                    operation: queued.operation.clone(),
                    status: w::OperationStatus::Cancelled,
                };
                let evidence = {
                    let state = self.runtime().state()?;
                    serde_json::to_vec(&Terminal {
                        version: 1,
                        operation: operation.clone(),
                        journal_revision: state.journal_revision(),
                        context_epoch: state.stamp().epoch(),
                        execution_settled: true,
                        error: None,
                        generations: Some(0),
                    })
                    .map_err(|_| DomainError::Conflict)?
                };
                self.runtime()
                    .store
                    .settle_intent(&queued.key, &queued.intent, &evidence)?;
                operation
            };
            let mut book = self.book()?;
            if book
                .queue
                .front()
                .is_none_or(|q| q.key != queued.key || !q.retiring)
            {
                return Err(DomainError::Conflict);
            }
            self.publish(&mut book, operation.clone())?;
            book.last_terminal = Some(operation);
            book.queue.pop_front();
        }
    }
}
