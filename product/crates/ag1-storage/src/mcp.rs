//! Atomic, bounded MCP manager state. A saved record is never live authority.
use crate::{Boundary, StorageError, StorageResult, Store, digest};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

pub const MAX_MCP_STATE_BYTES: usize = 1024 * 1024;
pub struct McpState {
    pub revision: u64,
    pub bytes: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    revision: u64,
    payload: serde_json::Value,
}
fn load(db: &rusqlite::Connection) -> StorageResult<Option<McpState>> {
    // Bound the blob in SQLite before copying it into host memory.
    let row: Option<(i64, String)> = db
        .query_row(
            "SELECT length(raw), digest FROM metadata WHERE kind='mcp-manager' AND id='state'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((length, hash)) = row else {
        return Ok(None);
    };
    if length <= 0 || length > (MAX_MCP_STATE_BYTES + 128) as i64 {
        return Err(StorageError::Corrupt("MCP state bounds".into()));
    }
    let raw: Vec<u8> = db.query_row(
        "SELECT raw FROM metadata WHERE kind='mcp-manager' AND id='state'",
        [],
        |r| r.get(0),
    )?;
    if digest(&raw) != hash {
        return Err(StorageError::Corrupt("MCP state digest".into()));
    }
    let envelope: Envelope = serde_json::from_slice(&raw)
        .map_err(|_| StorageError::Corrupt("MCP state envelope".into()))?;
    if envelope.revision == 0 || envelope.revision > i64::MAX as u64 {
        return Err(StorageError::Corrupt("MCP state revision".into()));
    }
    Ok(Some(McpState {
        revision: envelope.revision,
        bytes: serde_json::to_vec(&envelope.payload)?,
    }))
}
impl Store {
    pub fn mcp_state(&self) -> StorageResult<Option<McpState>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        load(&db)
    }
    /// A failed/uncertain acknowledgment requires a fresh read; it must never
    /// activate a connection. No automatic retry of stale state is performed.
    pub fn commit_mcp_state(&self, expected: u64, bytes: &[u8]) -> StorageResult<u64> {
        if bytes.is_empty() || bytes.len() > MAX_MCP_STATE_BYTES {
            return Err(StorageError::Limit("MCP state bytes".into()));
        }
        let revision = expected
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| StorageError::Limit("MCP state revision".into()))?;
        let payload = serde_json::from_slice(bytes)
            .map_err(|_| StorageError::Invalid("MCP state JSON".into()))?;
        let raw = serde_json::to_vec(&Envelope { revision, payload })?;
        if raw.len() > MAX_MCP_STATE_BYTES + 128 {
            return Err(StorageError::Limit("MCP state envelope".into()));
        }
        let mut db = self.conn()?;
        Self::ready(&db)?;
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if load(&tx)?.map_or(0, |s| s.revision) != expected {
            return Err(StorageError::Conflict("MCP state revision".into()));
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT digest FROM metadata WHERE kind='mcp-manager' AND id='state'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let hash = self.publish(&raw)?;
        tx.execute(
            "INSERT OR REPLACE INTO metadata(kind,id,digest,raw,historical) VALUES('mcp-manager','state',?1,?2,0)",
            params![hash, raw],
        )?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        // Only superseded successful MCP state is pruned. Never sweep unrelated
        // or failed-publication evidence, and never remove a shared live blob.
        if let Some(previous) = previous
            && !Self::referenced_blobs(&db)?.contains(&previous)
        {
            std::fs::remove_file(self.root.join("blobs").join(previous))?;
            crate::persistence::sync_dir(&self.root.join("blobs"))?;
        }
        Ok(revision)
    }
}
