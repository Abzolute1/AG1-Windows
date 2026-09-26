//! Dedicated private MCP state, using the existing native single-writer store.
//! Secrets never enter blob publication, metadata history, artifacts or journals.
//! This is private account storage, not OS-keychain encryption or forensic erase.
use crate::{Boundary, StorageError, StorageResult, Store, digest};
use ag1_core::SecretBytes;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::path::Path;

pub const MAX_BYTES: usize = 1024 * 1024;
pub struct PrivateState {
    pub revision: u64,
    pub bytes: SecretBytes,
}
pub struct PrivateStore(Store);
impl PrivateStore {
    /// The caller supplies a dedicated native account directory, never a project
    /// directory or the public MCP/provider/GitHub store.
    pub fn open(path: &Path) -> StorageResult<Self> {
        Self::from_store(Store::open(path)?)
    }
    pub fn from_store(store: Store) -> StorageResult<Self> {
        {
            let db = store.conn()?;
            Store::ready(&db)?;
            db.execute_batch("CREATE TABLE IF NOT EXISTS mcp_private_state(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL CHECK(revision>0), digest TEXT NOT NULL, raw BLOB NOT NULL CHECK(length(raw)>0 AND length(raw)<=1048576)); PRAGMA secure_delete=ON;")?;
        }
        Ok(Self(store))
    }
    pub fn load(&self) -> StorageResult<Option<PrivateState>> {
        let db = self.0.conn()?;
        Store::ready(&db)?;
        load(&db)
    }
    /// No retry after an uncertain acknowledgment. Reopen and inspect instead.
    /// Logical replacement does not promise erasure from WAL/filesystem backups.
    pub fn commit(&self, expected: u64, bytes: &SecretBytes) -> StorageResult<u64> {
        if bytes.expose().is_empty() || bytes.expose().len() > MAX_BYTES {
            return Err(StorageError::Limit("MCP private state bytes".into()));
        }
        let next = expected
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| StorageError::Limit("MCP private state revision".into()))?;
        let mut db = self.0.conn()?;
        Store::ready(&db)?;
        self.0.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let revision: Option<u64> = tx
            .query_row(
                "SELECT revision FROM mcp_private_state WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if revision.unwrap_or(0) != expected {
            return Err(StorageError::Conflict("MCP private state revision".into()));
        }
        tx.execute(
            "INSERT OR REPLACE INTO mcp_private_state VALUES(1,?1,?2,?3)",
            params![next, digest(bytes.expose()), bytes.expose()],
        )?;
        self.0.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.0.hit(Boundary::AfterCommit)?;
        self.0.hit(Boundary::BeforeAck)?;
        Ok(next)
    }
}
fn load(db: &rusqlite::Connection) -> StorageResult<Option<PrivateState>> {
    let row: Option<(u64, i64, String)> = db
        .query_row(
            "SELECT revision,length(raw),digest FROM mcp_private_state WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((revision, length, hash)) = row else {
        return Ok(None);
    };
    // Inspect length in SQLite before bringing the secret into host memory.
    if revision == 0 || revision > i64::MAX as u64 || length <= 0 || length > MAX_BYTES as i64 {
        return Err(StorageError::Corrupt("MCP private state bounds".into()));
    }
    let bytes = SecretBytes::new(db.query_row(
        "SELECT raw FROM mcp_private_state WHERE id=1",
        [],
        |r| r.get(0),
    )?);
    if digest(bytes.expose()) != hash {
        return Err(StorageError::Corrupt("MCP private state digest".into()));
    }
    Ok(Some(PrivateState { revision, bytes }))
}
