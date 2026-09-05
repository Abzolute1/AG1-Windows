//! Endpoint admission. A dropped or aborted transport is not evidence of completion.
//! The composition host shares one Scheduler between root and isolated calls.
use crate::{DomainError, OperationId};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointKind {
    Local,
    Remote,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Waiting,
    Granted,
    Running,
    Quarantined,
}
#[derive(Clone, Debug)]
pub struct Ticket {
    owner: Arc<()>,
    endpoint: String,
    generation: u64,
}
impl PartialEq for Ticket {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.owner, &other.owner)
            && self.endpoint == other.endpoint
            && self.generation == other.generation
    }
}
impl Eq for Ticket {}
struct Entry {
    generation: u64,
    operation: OperationId,
    state: State,
}
struct Pool {
    kind: EndpointKind,
    entries: VecDeque<Entry>,
}
struct Inner {
    next: u64,
    pools: BTreeMap<String, Pool>,
}
#[derive(Clone)]
pub struct Scheduler {
    owner: Arc<()>,
    inner: Arc<Mutex<Inner>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueStatus {
    pub waiting: usize,
    pub active: usize,
    pub quarantined: usize,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            owner: Arc::new(()),
            inner: Arc::new(Mutex::new(Inner {
                next: 0,
                pools: BTreeMap::new(),
            })),
        }
    }
}
impl Pool {
    fn pump(&mut self) {
        if self.entries.iter().any(|e| e.state == State::Quarantined) {
            return;
        }
        let capacity = match self.kind {
            EndpointKind::Local => 1,
            EndpointKind::Remote => 3,
        };
        let occupied = self
            .entries
            .iter()
            .filter(|e| e.state != State::Waiting)
            .count();
        for entry in self
            .entries
            .iter_mut()
            .filter(|e| e.state == State::Waiting)
            .take(capacity - occupied)
        {
            entry.state = State::Granted;
        }
    }
}
impl Scheduler {
    /// Endpoint identity is resolved by the host, independently of model/provider aliases.
    /// Registration is bounded and cannot reconfigure an occupied pool.
    pub fn register(&self, endpoint: &str, kind: EndpointKind) -> Result<(), DomainError> {
        if endpoint.is_empty() || endpoint.len() > 2048 || endpoint.chars().any(char::is_control) {
            return Err(DomainError::InvalidInput("endpoint identity".into()));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DomainError::Unavailable("scheduler poisoned".into()))?;
        if let Some(pool) = inner.pools.get(endpoint) {
            return if pool.kind == kind {
                Ok(())
            } else {
                Err(DomainError::Conflict)
            };
        }
        if inner.pools.len() == 256 {
            return Err(DomainError::QueueFull);
        }
        inner.pools.insert(
            endpoint.into(),
            Pool {
                kind,
                entries: VecDeque::new(),
            },
        );
        Ok(())
    }
    pub fn submit(&self, endpoint: &str, operation: OperationId) -> Result<Ticket, DomainError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DomainError::Unavailable("scheduler poisoned".into()))?;
        let generation = inner.next.checked_add(1).ok_or(DomainError::QueueFull)?;
        let pool = inner.pools.get_mut(endpoint).ok_or(DomainError::NotFound)?;
        if pool.entries.iter().any(|e| e.operation == operation) {
            return Err(DomainError::Conflict);
        }
        if pool
            .entries
            .iter()
            .filter(|e| e.state == State::Waiting)
            .count()
            >= 128
        {
            return Err(DomainError::QueueFull);
        }
        pool.entries.push_back(Entry {
            generation,
            operation,
            state: State::Waiting,
        });
        pool.pump();
        inner.next = generation;
        Ok(Ticket {
            owner: self.owner.clone(),
            endpoint: endpoint.into(),
            generation,
        })
    }
    /// Grants exactly once, including isolated summary/episode requests.
    pub fn start(&self, ticket: &Ticket) -> Result<Option<GenerationPermit>, DomainError> {
        self.check_owner(ticket)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DomainError::Unavailable("scheduler poisoned".into()))?;
        let pool = inner
            .pools
            .get_mut(&ticket.endpoint)
            .ok_or(DomainError::NotFound)?;
        if pool.entries.iter().any(|e| e.state == State::Quarantined) {
            return Ok(None);
        }
        let entry = pool
            .entries
            .iter_mut()
            .find(|e| e.generation == ticket.generation)
            .ok_or(DomainError::NotFound)?;
        match entry.state {
            State::Waiting => Ok(None),
            State::Granted => {
                entry.state = State::Running;
                Ok(Some(GenerationPermit {
                    scheduler: self.clone(),
                    ticket: ticket.clone(),
                    settled: false,
                }))
            }
            _ => Err(DomainError::Conflict),
        }
    }
    /// Queued cancellation needs no transport settlement. Running cancellation quarantines.
    pub fn cancel(&self, ticket: &Ticket) -> Result<(), DomainError> {
        self.check_owner(ticket)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DomainError::Unavailable("scheduler poisoned".into()))?;
        let pool = inner
            .pools
            .get_mut(&ticket.endpoint)
            .ok_or(DomainError::NotFound)?;
        let index = pool
            .entries
            .iter()
            .position(|e| e.generation == ticket.generation)
            .ok_or(DomainError::NotFound)?;
        if matches!(pool.entries[index].state, State::Waiting | State::Granted) {
            pool.entries.remove(index);
            pool.pump();
        } else {
            pool.entries[index].state = State::Quarantined;
        }
        Ok(())
    }
    fn finish(&self, ticket: &Ticket) -> Result<(), DomainError> {
        self.check_owner(ticket)?;
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| DomainError::Unavailable("scheduler poisoned".into()))?;
        let pool = inner
            .pools
            .get_mut(&ticket.endpoint)
            .ok_or(DomainError::NotFound)?;
        let index = pool
            .entries
            .iter()
            .position(|e| e.generation == ticket.generation)
            .ok_or(DomainError::NotFound)?;
        if !matches!(
            pool.entries[index].state,
            State::Running | State::Quarantined
        ) {
            return Err(DomainError::Conflict);
        }
        pool.entries.remove(index);
        pool.pump();
        Ok(())
    }
    /// Host-only recovery after transport terminal evidence or verified process exit.
    /// This must never be exposed as an untrusted command or called on a timer alone.
    pub fn reconcile(&self, ticket: &Ticket) -> Result<(), DomainError> {
        self.finish(ticket)
    }
    fn check_owner(&self, ticket: &Ticket) -> Result<(), DomainError> {
        if Arc::ptr_eq(&self.owner, &ticket.owner) {
            Ok(())
        } else {
            Err(DomainError::PermissionDenied)
        }
    }
    pub fn status(&self, endpoint: &str) -> Result<QueueStatus, DomainError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| DomainError::Unavailable("scheduler poisoned".into()))?;
        let pool = inner.pools.get(endpoint).ok_or(DomainError::NotFound)?;
        Ok(QueueStatus {
            waiting: pool
                .entries
                .iter()
                .filter(|e| e.state == State::Waiting)
                .count(),
            active: pool
                .entries
                .iter()
                .filter(|e| e.state != State::Waiting)
                .count(),
            quarantined: pool
                .entries
                .iter()
                .filter(|e| e.state == State::Quarantined)
                .count(),
        })
    }
}
/// A host-held capability, deliberately not Clone/Serialize. Drop quarantines instead of
/// releasing a possibly still-running generation. Terminal acknowledgement consumes it.
pub struct GenerationPermit {
    scheduler: Scheduler,
    ticket: Ticket,
    settled: bool,
}
impl GenerationPermit {
    pub fn terminal(mut self) -> Result<(), DomainError> {
        self.scheduler.finish(&self.ticket)?;
        self.settled = true;
        Ok(())
    }
}
impl Drop for GenerationPermit {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.scheduler.cancel(&self.ticket);
        }
    }
}
