//! GUI-independent domain boundaries. No filesystem, database, process or model implementation.
use std::{collections::VecDeque, future::Future, pin::Pin};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DomainError {
    InvalidInput(String),
    Unavailable(String),
    Conflict,
    NotFound,
    QueueFull,
    Cancelled,
}

macro_rules! ids {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
                let value = value.into();
                if value.is_empty() || value.len() > 128 || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b)) {
                    return Err(DomainError::InvalidInput(stringify!($name).into()));
                }
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str { &self.0 }
        }
    )+};
}
ids!(WorkspaceId, SessionId, BranchId, OperationId, ArtifactId);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationState {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug)]
pub struct SessionView {
    pub workspace: WorkspaceId,
    pub session: SessionId,
    pub branch: BranchId,
    pub epoch: u64,
    pub revision: u64,
}

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, DomainError>> + Send + 'a>>;

/// Adapters resolve workspace identity before touching paths; no process-global cwd.
pub trait WorkspacePort: Send + Sync {
    fn read<'a>(
        &'a self,
        workspace: &'a WorkspaceId,
        relative_path: &'a str,
        max_bytes: usize,
    ) -> PortFuture<'a, Vec<u8>>;
}

/// A future adapter must commit the expected revision and bytes atomically.
pub trait JournalPort: Send + Sync {
    fn append<'a>(
        &'a self,
        session: &'a SessionId,
        expected_revision: u64,
        record: &'a [u8],
    ) -> PortFuture<'a, u64>;
}

pub trait ArtifactPort: Send + Sync {
    fn read_page<'a>(
        &'a self,
        session: &'a SessionId,
        artifact: &'a ArtifactId,
        offset: u64,
        limit: u32,
    ) -> PortFuture<'a, Vec<u8>>;
}

/// The adapter owns endpoint admission until completion/reconciliation, including cancellation.
pub trait ProviderPort: Send + Sync {
    fn cancel<'a>(&'a self, operation: &'a OperationId) -> PortFuture<'a, ()>;
}

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// FIFO waiting room, separate from active endpoint permits. Cancellation frees capacity.
pub struct EndpointQueue<T> {
    waiting: VecDeque<T>,
    capacity: usize,
}
impl<T> EndpointQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            waiting: VecDeque::new(),
            capacity,
        }
    }
    pub fn admit(&mut self, item: T) -> Result<(), DomainError> {
        if self.waiting.len() >= self.capacity {
            return Err(DomainError::QueueFull);
        }
        self.waiting.push_back(item);
        Ok(())
    }
    pub fn pop(&mut self) -> Option<T> {
        self.waiting.pop_front()
    }
    pub fn cancel(&mut self, predicate: impl Fn(&T) -> bool) {
        self.waiting.retain(|item| !predicate(item));
    }
    pub fn len(&self) -> usize {
        self.waiting.len()
    }
    pub fn is_empty(&self) -> bool {
        self.waiting.is_empty()
    }
}
