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

impl ag1_core::task::JournalWritePort for Store {
    fn append_records(
        &self,
        session: &SessionId,
        command: &str,
        expected: u64,
        records: &[Vec<u8>],
    ) -> Result<(u64, u64), DomainError> {
        self.append_batch(session.as_str(), command, expected, records)
            .map(|ack| (ack.revision, ack.sequence))
            .map_err(domain_error)
    }
}

impl ag1_core::evidence::EvidencePort for Store {
    fn record(
        &self,
        request: &ag1_core::evidence::EvidenceRequest,
    ) -> Result<serde_json::Value, DomainError> {
        let bytes = self
            .artifact_record(
                request.requester.as_str(),
                &request.origin,
                &request.artifact,
            )
            .map_err(domain_error)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| DomainError::InvalidInput("evidence metadata".into()))
    }
    fn page(
        &self,
        request: &ag1_core::evidence::EvidenceRequest,
        offset: u64,
        max_bytes: u32,
    ) -> Result<ag1_core::evidence::EvidencePage, DomainError> {
        self.page(
            request.requester.as_str(),
            &request.origin,
            &request.artifact,
            offset,
            max_bytes,
        )
        .map(|p| ag1_core::evidence::EvidencePage {
            bytes: p.bytes,
            offset: p.offset,
            next_offset: p.next_offset,
        })
        .map_err(domain_error)
    }
    fn read_cursor(
        &self,
        requester: &SessionId,
        cursor: &str,
    ) -> Result<ag1_core::evidence::EvidencePage, DomainError> {
        self.read_cursor(requester.as_str(), cursor)
            .map(|p| ag1_core::evidence::EvidencePage {
                bytes: p.bytes,
                offset: p.offset,
                next_offset: p.next_offset,
            })
            .map_err(domain_error)
    }
    fn cursor(
        &self,
        request: &ag1_core::evidence::EvidenceRequest,
        offset: u64,
        max_bytes: u32,
    ) -> Result<String, DomainError> {
        self.cursor(
            request.requester.as_str(),
            &request.origin,
            &request.artifact,
            offset,
            max_bytes,
        )
        .map_err(domain_error)
    }
    fn search(
        &self,
        requester: &SessionId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ag1_core::evidence::EvidenceHit>, DomainError> {
        self.search(requester.as_str(), query, limit)
            .map(|rows| {
                rows.into_iter()
                    .map(|r| ag1_core::evidence::EvidenceHit {
                        origin: r.origin,
                        artifact: r.artifact,
                        line: r.line,
                        text: r.text,
                    })
                    .collect()
            })
            .map_err(domain_error)
    }
    fn lease(
        &self,
        request: &ag1_core::evidence::EvidenceRequest,
        reason: &str,
        ttl_ms: u64,
        release_on_verification: bool,
    ) -> Result<(), DomainError> {
        self.lease(
            request.requester.as_str(),
            &request.origin,
            &request.artifact,
            reason,
            ttl_ms,
            release_on_verification,
        )
        .map(|_| ())
        .map_err(domain_error)
    }
}
