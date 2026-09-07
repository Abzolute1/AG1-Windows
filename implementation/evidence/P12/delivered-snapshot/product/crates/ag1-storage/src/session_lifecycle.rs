//! Atomic native session bootstrap. Legacy import/shadow primitives remain
//! available; the runtime facade creates its journal and output origin together.
use crate::{
    Ack, Boundary, StorageError, StorageResult, Store, digest,
    journal::{active_session, command, entry, insert_entry},
    persistence::valid_id,
};
use rusqlite::params;
use serde_json::{Value, json};

mod native_fork;

pub enum SessionSeed<'a> {
    Fresh {
        header: &'a [u8],
        records: &'a [Vec<u8>],
    },
    Fork {
        parent: &'a str,
        expected_revision: u64,
        leaf: Option<&'a str>,
    },
    /// Product session fork: a new header, current resolved labels and repaired
    /// ancestry when old label records are removed. None means an empty branch,
    /// unlike the lower-level Fork compatibility primitive above.
    RuntimeFork {
        parent: &'a str,
        expected_revision: u64,
        leaf: Option<&'a str>,
        timestamp: &'a str,
    },
}
/// The cursor key is host-generated secret material, never a renderer argument
/// or part of the command intent. Replaying an acknowledged bootstrap retains
/// the original key even when a restarted host generated a new proposal.
pub struct SessionBootstrap<'a> {
    pub command: &'a str,
    pub session: &'a str,
    pub origin: &'a str,
    pub cursor_key: &'a [u8],
    pub seed: SessionSeed<'a>,
}
impl Store {
    pub fn bootstrap_session(&self, input: SessionBootstrap<'_>) -> StorageResult<Ack> {
        for id in [input.command, input.session, input.origin] {
            valid_id(id)?;
        }
        if input.cursor_key.len() != 32 {
            return Err(StorageError::Invalid(
                "Cursor key must contain 32 bytes".into(),
            ));
        }
        let seed_intent = match &input.seed {
            SessionSeed::Fresh { header, records } => {
                if header.len() > 4 * 1024 * 1024 {
                    return Err(StorageError::Limit("Session header exceeds 4 MiB".into()));
                }
                let value: Value = serde_json::from_slice(header)?;
                if value["id"] != input.session
                    || value["type"] != "session"
                    || value["version"] != 3
                {
                    return Err(StorageError::Invalid(
                        "New session needs a matching v3 header".into(),
                    ));
                }
                if records.len() > 100000
                    || records
                        .iter()
                        .try_fold(0usize, |n, r| n.checked_add(r.len()))
                        .is_none_or(|n| n > 128 * 1024 * 1024)
                {
                    return Err(StorageError::Limit("Initial session records".into()));
                }
                json!({"kind":"fresh","headerSha256":digest(header),"recordHashes":records.iter().map(|r|digest(r)).collect::<Vec<_>>()})
            }
            SessionSeed::Fork {
                parent,
                expected_revision,
                leaf,
            } => {
                valid_id(parent)?;
                if let Some(leaf) = leaf {
                    valid_id(leaf)?;
                }
                if *parent == input.session {
                    return Err(StorageError::Conflict(
                        "Cannot fork a session into itself".into(),
                    ));
                }
                json!({"kind":"fork","parent":parent,"expectedRevision":expected_revision,"leaf":leaf})
            }
            SessionSeed::RuntimeFork {
                parent,
                expected_revision,
                leaf,
                timestamp,
            } => {
                valid_id(parent)?;
                if let Some(leaf) = leaf {
                    valid_id(leaf)?;
                }
                if *parent == input.session {
                    return Err(StorageError::Conflict(
                        "Cannot fork a session into itself".into(),
                    ));
                }
                if timestamp.len() > 64 || chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
                {
                    return Err(StorageError::Invalid("Fork timestamp".into()));
                }
                json!({"kind":"runtime-fork","parent":parent,"expectedRevision":expected_revision,"leaf":leaf,"timestamp":timestamp})
            }
        };
        let intent = serde_json::to_vec(
            &json!({"kind":"native-session-bootstrap","session":input.session,"origin":input.origin,"seed":seed_intent}),
        )?;
        let fingerprint = digest(&intent);
        let mut db = self.conn()?;
        Self::ready(&db)?;
        // Idempotency precedes current parent revision checks. An identical
        // accepted fork must remain queryable after its parent advances.
        if let Some(old) = command(&db, input.command, &fingerprint)? {
            if old.state != "completed" {
                return Err(StorageError::Conflict(
                    "Session bootstrap needs reconciliation".into(),
                ));
            }
            return Ok(serde_json::from_slice(old.result.as_deref().ok_or_else(
                || StorageError::Corrupt("Missing session bootstrap result".into()),
            )?)?);
        }
        let occupied:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1) OR EXISTS(SELECT 1 FROM origins WHERE id=?2)",params![input.session,input.origin],|r|r.get(0))?;
        if occupied {
            return Err(StorageError::Conflict(
                "Session or artifact origin already exists".into(),
            ));
        }
        let (header, entries, parent) = match input.seed {
            SessionSeed::Fresh { header, records } => {
                let mut entries = Vec::<crate::JournalEntry>::with_capacity(records.len());
                for (index, raw) in records.iter().enumerate() {
                    let record = entry(raw, index as u64 + 1)?;
                    if record.parent.as_deref() != entries.last().map(|e| e.id.as_str()) {
                        return Err(StorageError::Invalid(
                            "Initial session records must form one branch".into(),
                        ));
                    }
                    entries.push(record);
                }
                (header.to_vec(), entries, None)
            }
            SessionSeed::Fork {
                parent,
                expected_revision,
                leaf,
            } => {
                if active_session(&db, parent)? != expected_revision {
                    return Err(StorageError::Conflict("Fork parent revision".into()));
                }
                let snapshot = Self::snapshot_db(&db, parent)?;
                let branch = Self::branch_snapshot(&snapshot, leaf)?;
                if branch.len() > 100000
                    || branch
                        .iter()
                        .try_fold(0usize, |n, e| n.checked_add(e.raw.len()))
                        .is_none_or(|n| n > 128 * 1024 * 1024)
                {
                    return Err(StorageError::Limit("Fork branch size".into()));
                }
                let mut header: Value = serde_json::from_slice(&snapshot.header)?;
                header["id"] = input.session.into();
                header["parentSession"] = parent.into();
                header["version"] = 3.into();
                (serde_json::to_vec(&header)?, branch, Some(parent))
            }
            SessionSeed::RuntimeFork {
                parent,
                expected_revision,
                leaf,
                timestamp,
            } => {
                if active_session(&db, parent)? != expected_revision {
                    return Err(StorageError::Conflict("Fork parent revision".into()));
                }
                let snapshot = Self::snapshot_db(&db, parent)?;
                let branch = native_fork::records(&snapshot, leaf, input.command)?;
                let mut header: Value = serde_json::from_slice(&snapshot.header)?;
                header["id"] = input.session.into();
                header["parentSession"] = parent.into();
                header["version"] = 3.into();
                header["timestamp"] = timestamp.into();
                let closure = (!branch.is_empty()).then_some(parent);
                (serde_json::to_vec(&header)?, branch, closure)
            }
        };
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        let revision = entries.len() as u64;
        tx.execute(
            "INSERT INTO sessions(id,header,leaf,revision,source_version) VALUES(?1,?2,?3,?4,3)",
            params![
                input.session,
                header,
                entries.last().map(|e| &e.id),
                revision
            ],
        )?;
        for entry in &entries {
            insert_entry(&tx, input.session, entry)?;
        }
        tx.execute(
            "INSERT INTO origins VALUES(?1,?2,?3)",
            params![input.origin, input.session, input.cursor_key],
        )?;
        if let Some(parent) = parent {
            // Opaque journal extensions can carry old cursors. Inherit only the
            // parent's authorized closure, matching the established P05 fork.
            tx.execute("INSERT INTO access SELECT ?1,origin,artifact,'fork-closure' FROM access WHERE session=?2",params![input.session,parent])?;
            tx.execute("INSERT INTO leases SELECT ?1,origin,artifact,reason,expires,release_on_verification,raw FROM leases WHERE session=?2 AND expires>?3",params![input.session,parent,self.clock.now_ms()])?;
        }
        let ack = Ack {
            sequence: Self::next_sequence(&tx)?,
            revision,
        };
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,'completed',?4,?5)",
            params![
                input.command,
                fingerprint,
                intent,
                serde_json::to_vec(&ack)?,
                ack.sequence
            ],
        )?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(ack)
    }
}
