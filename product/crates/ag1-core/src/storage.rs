//! Persistence views contain domain values and opaque source bytes, never SQLite handles.
//! Adapters perform blocking durable work: composition hosts must use a bounded storage executor.
use crate::{DomainError, SessionId};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEntry {
    pub id: String,
    pub parent: Option<String>,
    pub physical_order: u64,
    pub raw: Vec<u8>,
    pub normalized: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct StoredJournal {
    pub revision: u64,
    pub committed_sequence: u64,
    pub header: Vec<u8>,
    pub leaf: Option<String>,
    pub entries: Vec<StoredEntry>,
}
#[derive(Clone, Debug)]
pub struct StoredDocument {
    pub sha256: String,
    pub bytes: Vec<u8>,
    pub diagnostic: Option<String>,
}
/// Synchronous transaction boundary intended for the host's bounded blocking executor.
pub trait StoragePort: Send + Sync {
    fn journal_snapshot(&self, session: &SessionId) -> Result<StoredJournal, DomainError>;
    fn fork_branch(
        &self,
        parent: &SessionId,
        child: &SessionId,
        leaf: Option<&str>,
    ) -> Result<(), DomainError>;
    fn load_memory(&self, id: &str) -> Result<Option<StoredDocument>, DomainError>;
    fn commit_memory(
        &self,
        id: &str,
        expected_sha256: Option<&str>,
        bytes: &[u8],
    ) -> Result<String, DomainError>;
    fn load_metadata(&self, kind: &str, id: &str) -> Result<Option<StoredDocument>, DomainError>;
    fn commit_metadata(
        &self,
        kind: &str,
        id: &str,
        expected_sha256: Option<&str>,
        bytes: &[u8],
    ) -> Result<String, DomainError>;
}
