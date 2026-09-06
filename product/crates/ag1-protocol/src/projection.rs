//! Operation projection for native hosts. Event sequence is contiguous, while
//! journal revision can stay equal on admission or advance by several records.
use crate::wire::{Event, MAX_SESSION_QUEUE, OperationStatus, ProtocolError, Snapshot};

pub struct Projection {
    pub snapshot: Snapshot,
    pub needs_resync: bool,
}
impl Projection {
    pub fn new(snapshot: Snapshot) -> Self {
        Self {
            snapshot,
            needs_resync: false,
        }
    }
    pub fn apply(&mut self, event: &Event) -> Result<bool, ProtocolError> {
        if event.scope != self.snapshot.scope {
            return Err(ProtocolError::StaleContext);
        }
        if event.sequence.get() <= self.snapshot.watermark.get() {
            return Ok(false);
        }
        let queued = self.snapshot.queued.contains(&event.operation.operation);
        let valid_transition = match event.operation.status {
            OperationStatus::Queued => !queued && self.snapshot.queued.len() < MAX_SESSION_QUEUE,
            _ => queued,
        };
        if self.needs_resync
            || Some(event.sequence.get()) != self.snapshot.watermark.get().checked_add(1)
            || event.revision.get() < self.snapshot.revision.get()
            || !valid_transition
        {
            self.needs_resync = true;
            return Err(ProtocolError::ResyncRequired);
        }
        self.snapshot.watermark = event.sequence.clone();
        self.snapshot.revision = event.revision.clone();
        if event.operation.status == OperationStatus::Queued {
            self.snapshot.queued.push(event.operation.operation.clone());
        } else {
            self.snapshot
                .queued
                .retain(|op| op != &event.operation.operation);
            self.snapshot.last_terminal = Some(event.operation.clone());
        }
        Ok(true)
    }
}
