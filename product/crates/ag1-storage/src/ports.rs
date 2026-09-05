use crate::persistence::{domain_error, valid_id};
use crate::{Boundary, StorageError, StorageResult, Store};
use ag1_core::{
    DomainError, SessionId,
    storage::{StoragePort, StoredDocument, StoredEntry, StoredJournal},
};
use rusqlite::{OptionalExtension, params};

impl Store {
    pub fn save_metadata(
        &self,
        kind: &str,
        id: &str,
        expected: Option<&str>,
        raw: &[u8],
    ) -> StorageResult<String> {
        valid_id(kind)?;
        valid_id(id)?;
        if matches!(
            kind,
            "historical" | "extra" | "metadata-history" | "memory-history"
        ) {
            return Err(StorageError::Denied);
        }
        if raw.len() > 4 * 1024 * 1024 {
            return Err(StorageError::Limit("Metadata envelope".into()));
        }
        let _: serde_json::Value = serde_json::from_slice(raw)?;
        let mut db = self.conn()?;
        Self::ready(&db)?;
        let old: Option<(String, Vec<u8>)> = db
            .query_row(
                "SELECT digest,raw FROM metadata WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if old.as_ref().map(|v| v.0.as_str()) != expected {
            return Err(StorageError::Conflict("Metadata revision".into()));
        }
        let hash = self.publish(raw)?;
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        if let Some((previous, bytes)) = old {
            tx.execute(
                "INSERT OR IGNORE INTO metadata VALUES('metadata-history',?1,?2,?3,1)",
                params![format!("{kind}:{id}:{previous}"), previous, bytes],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO metadata VALUES(?1,?2,?3,?4,0)",
            params![kind, id, hash, raw],
        )?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(hash)
    }
}
impl StoragePort for Store {
    fn journal_snapshot(&self, session: &SessionId) -> Result<StoredJournal, DomainError> {
        self.snapshot(session.as_str())
            .map(|s| StoredJournal {
                revision: s.revision,
                committed_sequence: s.sequence,
                header: s.header,
                leaf: s.leaf,
                entries: s
                    .entries
                    .into_iter()
                    .map(|e| StoredEntry {
                        id: e.id,
                        parent: e.parent,
                        physical_order: e.ordinal,
                        raw: e.raw,
                        normalized: e.normalized,
                    })
                    .collect(),
            })
            .map_err(domain_error)
    }
    fn fork_branch(
        &self,
        parent: &SessionId,
        child: &SessionId,
        leaf: Option<&str>,
    ) -> Result<(), DomainError> {
        self.fork(parent.as_str(), child.as_str(), leaf)
            .map_err(domain_error)
    }
    fn load_memory(&self, id: &str) -> Result<Option<StoredDocument>, DomainError> {
        let view = self.memory(id).map_err(domain_error)?;
        view.map(|v| {
            let bytes = self.blob_bytes(&v.digest).map_err(domain_error)?;
            Ok(StoredDocument {
                sha256: v.digest,
                bytes,
                diagnostic: v.diagnostic,
            })
        })
        .transpose()
    }
    fn commit_memory(
        &self,
        id: &str,
        expected: Option<&str>,
        bytes: &[u8],
    ) -> Result<String, DomainError> {
        self.save_memory(id, expected, bytes).map_err(domain_error)
    }
    fn load_metadata(&self, kind: &str, id: &str) -> Result<Option<StoredDocument>, DomainError> {
        self.metadata(kind, id)
            .map(|value| {
                value.map(|bytes| StoredDocument {
                    sha256: crate::digest(&bytes),
                    bytes,
                    diagnostic: None,
                })
            })
            .map_err(domain_error)
    }
    fn commit_metadata(
        &self,
        kind: &str,
        id: &str,
        expected: Option<&str>,
        bytes: &[u8],
    ) -> Result<String, DomainError> {
        self.save_metadata(kind, id, expected, bytes)
            .map_err(domain_error)
    }
}
