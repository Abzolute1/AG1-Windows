//! Request admission and durable-work barriers for one session mutation owner.
//! Observing a public event is not an acknowledgement. The composition owner
//! calls acknowledge only after the corresponding P05 commit has returned.
use crate::{DomainError, SessionId};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamp {
    owner: u64,
    session: SessionId,
    epoch: u64,
    revision: u64,
}
impl Stamp {
    pub fn same_owner_epoch(&self, other: &Self) -> bool {
        self.owner == other.owner && self.session == other.session && self.epoch == other.epoch
    }
    pub fn session(&self) -> &SessionId {
        &self.session
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
pub struct Preparation {
    owner: u64,
    stamp: Stamp,
    serial: u64,
}
pub struct Generation {
    owner: u64,
    session: SessionId,
    serial: u64,
}
pub struct Append {
    owner: u64,
    session: SessionId,
    serial: u64,
}
#[derive(Clone)]
pub struct Compaction {
    owner: u64,
    session: SessionId,
    serial: u64,
}

pub struct Boundary {
    owner: u64,
    stamp: Stamp,
    serial: u64,
    preparation: Option<u64>,
    generation: Option<u64>,
    appends: VecDeque<u64>,
    pending_tools: BTreeSet<String>,
    compaction: Option<u64>,
    cancelled: bool,
    persistence_failed: bool,
}
impl Boundary {
    pub fn new(session: SessionId, epoch: u64) -> Result<Self, DomainError> {
        let owner = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| DomainError::Conflict)?;
        Ok(Self {
            owner,
            stamp: Stamp {
                owner,
                session,
                epoch,
                revision: 0,
            },
            serial: 0,
            preparation: None,
            generation: None,
            appends: VecDeque::new(),
            pending_tools: BTreeSet::new(),
            compaction: None,
            cancelled: false,
            persistence_failed: false,
        })
    }
    pub fn stamp(&self) -> &Stamp {
        &self.stamp
    }
    /// Invoke for every committed journal/leaf/model/tool/thinking/task/context
    /// change. Prepared requests bind this revision, including in one tool loop.
    pub fn changed(&mut self) -> Result<(), DomainError> {
        self.stamp.revision = self
            .stamp
            .revision
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        Ok(())
    }
    fn serial(&mut self) -> Result<u64, DomainError> {
        self.serial = self.serial.checked_add(1).ok_or(DomainError::Conflict)?;
        Ok(self.serial)
    }
    pub fn settled(&self) -> bool {
        self.preparation.is_none()
            && self.generation.is_none()
            && self.appends.is_empty()
            && self.pending_tools.is_empty()
            && self.compaction.is_none()
    }
    /// Drained work and acknowledged persistence are separate properties.
    pub fn persistence_healthy(&self) -> bool {
        !self.persistence_failed
    }
    pub fn prepare(&mut self) -> Result<Preparation, DomainError> {
        if self.cancelled {
            return Err(DomainError::Cancelled);
        }
        if self.persistence_failed {
            return Err(DomainError::DurabilityUnknown);
        }
        if !self.settled() {
            return Err(DomainError::Unavailable("pending session work".into()));
        }
        let serial = self.serial()?;
        self.preparation = Some(serial);
        Ok(Preparation {
            owner: self.owner,
            stamp: self.stamp.clone(),
            serial,
        })
    }
    /// Final check after asynchronous context, transforms and credential work,
    /// immediately before entering the shared provider scheduler/transport.
    pub fn admit(&mut self, preparation: Preparation) -> Result<Generation, DomainError> {
        if preparation.owner != self.owner
            || self.preparation != Some(preparation.serial)
            || self.stamp.session != preparation.stamp.session
        {
            return Err(DomainError::Conflict);
        }
        self.preparation = None;
        if self.cancelled {
            return Err(DomainError::Cancelled);
        }
        if self.persistence_failed {
            return Err(DomainError::DurabilityUnknown);
        }
        if self.stamp != preparation.stamp || !self.settled() {
            return Err(DomainError::Conflict);
        }
        self.generation = Some(preparation.serial);
        Ok(Generation {
            owner: self.owner,
            session: self.stamp.session.clone(),
            serial: preparation.serial,
        })
    }
    /// Release a failed transform/compiler/credential preparation without
    /// cancelling the turn or clearing a newer owner's preparation.
    pub fn discard_preparation(&mut self, preparation: &Preparation) -> Result<(), DomainError> {
        if preparation.owner != self.owner
            || preparation.stamp.session != self.stamp.session
            || self.preparation != Some(preparation.serial)
        {
            return Err(DomainError::Conflict);
        }
        self.preparation = None;
        Ok(())
    }
    pub fn generation_terminal(&mut self, generation: &Generation) -> Result<(), DomainError> {
        if generation.owner != self.owner
            || generation.session != self.stamp.session
            || self.generation != Some(generation.serial)
        {
            return Err(DomainError::Conflict);
        }
        self.generation = None;
        Ok(())
    }
    pub fn begin_append(&mut self) -> Result<Append, DomainError> {
        if self.appends.len() >= 256 {
            return Err(DomainError::QueueFull);
        }
        let serial = self.serial()?;
        self.appends.push_back(serial);
        Ok(Append {
            owner: self.owner,
            session: self.stamp.session.clone(),
            serial,
        })
    }
    fn check_append(&self, append: &Append) -> Result<(), DomainError> {
        if append.owner != self.owner
            || append.session != self.stamp.session
            || self.appends.front() != Some(&append.serial)
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, append: &Append) -> Result<(), DomainError> {
        self.check_append(append)?;
        self.changed()?;
        self.appends.pop_front();
        Ok(())
    }
    pub fn append_failed(&mut self, append: &Append) -> Result<(), DomainError> {
        self.check_append(append)?;
        self.persistence_failed = true;
        self.appends.pop_front();
        Ok(())
    }
    /// Accepted output could not be staged or its worker receipt was lost.
    /// Keep all in-flight barriers and require explicit storage reconciliation.
    pub fn fail_persistence(&mut self) {
        self.persistence_failed = true;
    }
    pub fn register_tools(&mut self, ids: &[String]) -> Result<(), DomainError> {
        if !self.pending_tools.is_empty() {
            return Err(DomainError::Conflict);
        }
        self.pending_tools = Self::tool_ids(ids)?;
        Ok(())
    }
    fn tool_ids(ids: &[String]) -> Result<BTreeSet<String>, DomainError> {
        if ids.len() > 128 {
            return Err(DomainError::LimitExceeded("runtime tool batch".into()));
        }
        let mut pending = BTreeSet::new();
        for id in ids {
            if id.is_empty()
                || id.len() > 128
                || id.chars().any(char::is_control)
                || !pending.insert(id.clone())
            {
                return Err(DomainError::InvalidInput("tool call identity".into()));
            }
        }
        Ok(pending)
    }
    /// Publishing an assistant's committed calls and acknowledging its append
    /// is one owner transition: no observable request-admission gap in between.
    pub fn acknowledge_tool_batch(
        &mut self,
        append: &Append,
        ids: &[String],
    ) -> Result<(), DomainError> {
        if !self.pending_tools.is_empty() {
            return Err(DomainError::Conflict);
        }
        let pending = Self::tool_ids(ids)?;
        self.acknowledge(append)?;
        self.pending_tools = pending;
        Ok(())
    }
    /// Only a durably acknowledged tool result clears the pending ID. Repeated
    /// terminal notifications cannot clear a different call or reopen the batch.
    pub fn acknowledge_tool(&mut self, append: &Append, id: &str) -> Result<(), DomainError> {
        if !self.pending_tools.contains(id) {
            return Err(DomainError::Conflict);
        }
        self.acknowledge(append)?;
        self.pending_tools.remove(id);
        Ok(())
    }
    /// Reserve before requesting generation cancellation. This prevents a new
    /// provider request while the old generation/event work drains.
    pub fn reserve_compaction(&mut self) -> Result<Compaction, DomainError> {
        if self.cancelled {
            return Err(DomainError::Cancelled);
        }
        if self.persistence_failed {
            return Err(DomainError::DurabilityUnknown);
        }
        if self.compaction.is_some() {
            return Err(DomainError::Conflict);
        }
        let serial = self.serial()?;
        self.compaction = Some(serial);
        self.preparation = None;
        Ok(Compaction {
            owner: self.owner,
            session: self.stamp.session.clone(),
            serial,
        })
    }
    pub fn compaction_ready(&self) -> bool {
        self.compaction_drained() && !self.persistence_failed && !self.cancelled
    }
    fn compaction_drained(&self) -> bool {
        self.compaction.is_some()
            && self.generation.is_none()
            && self.appends.is_empty()
            && self.pending_tools.is_empty()
    }
    pub fn finish_compaction(
        &mut self,
        compaction: &Compaction,
        committed: bool,
    ) -> Result<(), DomainError> {
        self.check_compaction(compaction)?;
        // This is acknowledgement of completed work, not permission to start
        // compaction. Cancellation must drain an already-running summary or
        // journal commit; it cannot erase a commit whose acknowledgement raced
        // the cancel command. A failed write may release only an aborted run.
        if committed && (!self.compaction_drained() || self.persistence_failed) {
            return Err(DomainError::Conflict);
        }
        if committed {
            let epoch = self
                .stamp
                .epoch
                .checked_add(1)
                .ok_or(DomainError::Conflict)?;
            self.changed()?;
            self.stamp.epoch = epoch;
        }
        // Abandoning this reservation acknowledges only the summary operation.
        // Other generations/tools/appends keep their own barriers closed.
        self.compaction = None;
        Ok(())
    }
    pub(crate) fn check_compaction(&self, compaction: &Compaction) -> Result<(), DomainError> {
        if compaction.owner != self.owner
            || compaction.session != self.stamp.session
            || self.compaction != Some(compaction.serial)
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.preparation = None;
        // Active generation, tool effects, appends and compaction must still
        // drain. A compaction owner acknowledges an aborted run with false.
    }
    pub fn next_turn(&mut self) -> Result<(), DomainError> {
        if self.persistence_failed {
            return Err(DomainError::DurabilityUnknown);
        }
        if !self.settled() {
            return Err(DomainError::Conflict);
        }
        self.changed()?;
        self.cancelled = false;
        Ok(())
    }
}
