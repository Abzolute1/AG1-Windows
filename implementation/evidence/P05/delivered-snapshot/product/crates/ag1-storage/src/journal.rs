use crate::persistence::valid_id;
use crate::{Boundary, StorageError, StorageResult, Store, digest};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ack {
    pub sequence: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandResult {
    pub state: String,
    pub intent: Vec<u8>,
    pub result: Option<Vec<u8>>,
    pub sequence: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalEntry {
    pub id: String,
    pub parent: Option<String>,
    pub ordinal: u64,
    pub raw: Vec<u8>,
    pub normalized: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalSnapshot {
    pub session: String,
    pub revision: u64,
    pub sequence: u64,
    pub header: Vec<u8>,
    pub leaf: Option<String>,
    pub entries: Vec<JournalEntry>,
}
pub(crate) fn active_session(db: &Connection, session: &str) -> StorageResult<u64> {
    db.query_row(
        "SELECT revision FROM sessions WHERE id=?1 AND archived=0",
        [session],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| StorageError::Missing("Session".into()))
}
pub(crate) fn command(
    db: &Connection,
    id: &str,
    fingerprint: &str,
) -> StorageResult<Option<CommandResult>> {
    let row = db
        .query_row(
            "SELECT fingerprint,state,intent,result,sequence FROM commands WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    CommandResult {
                        state: r.get(1)?,
                        intent: r.get(2)?,
                        result: r.get(3)?,
                        sequence: r.get(4)?,
                    },
                ))
            },
        )
        .optional()?;
    match row {
        Some((hash, _)) if hash != fingerprint => Err(StorageError::Conflict(
            "Command ID reused for different input".into(),
        )),
        Some((_, value)) => Ok(Some(value)),
        None => Ok(None),
    }
}
pub(crate) fn entry(raw: &[u8], ordinal: u64) -> StorageResult<JournalEntry> {
    if raw.len() > 4 * 1024 * 1024 {
        return Err(StorageError::Limit("Journal record exceeds 4 MiB".into()));
    }
    let v: Value = serde_json::from_slice(raw)?;
    let id = v["id"]
        .as_str()
        .ok_or_else(|| StorageError::Invalid("Entry ID".into()))?;
    valid_id(id)?;
    if !v["type"].is_string() || v["type"] == "session" {
        return Err(StorageError::Invalid("Entry type".into()));
    }
    let valid = match v["type"].as_str().unwrap() {
        "message" => v["message"].is_object(),
        "thinking_level_change" => v["thinkingLevel"].is_string(),
        "model_change" => v["provider"].is_string() && v["modelId"].is_string(),
        "compaction" => {
            v["summary"].is_string()
                && v["tokensBefore"].is_number()
                && v["firstKeptEntryId"].is_string()
        }
        "branch_summary" => v["fromId"].is_string() && v["summary"].is_string(),
        "custom" | "custom_message" => v["customType"].is_string(),
        "label" => v["targetId"].is_string(),
        _ => true,
    };
    if !valid {
        return Err(StorageError::Corrupt(
            "Malformed known journal entry; raw import remains archived".into(),
        ));
    }
    let parent = match &v["parentId"] {
        Value::Null => None,
        Value::String(p) => {
            valid_id(p)?;
            Some(p.clone())
        }
        _ => return Err(StorageError::Invalid("Parent ID".into())),
    };
    Ok(JournalEntry {
        id: id.into(),
        parent,
        ordinal,
        raw: raw.to_vec(),
        normalized: serde_json::to_vec(&v)?,
    })
}
pub(crate) fn insert_entry(db: &Connection, session: &str, e: &JournalEntry) -> StorageResult<()> {
    if let Some(parent) = &e.parent {
        let found: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM entries WHERE session=?1 AND id=?2)",
            params![session, parent],
            |r| r.get(0),
        )?;
        if !found {
            return Err(StorageError::Corrupt(
                "Missing/forward journal parent".into(),
            ));
        }
    }
    db.execute(
        "INSERT INTO entries VALUES(?1,?2,?3,?4,?5,?6)",
        params![session, e.id, e.parent, e.ordinal, e.raw, e.normalized],
    )?;
    Ok(())
}
impl Store {
    pub fn create_session(&self, id: &str, header: &[u8]) -> StorageResult<()> {
        valid_id(id)?;
        let value: Value = serde_json::from_slice(header)?;
        if value["id"] != id || value["type"] != "session" || value["version"] != 3 {
            return Err(StorageError::Invalid(
                "New session needs a matching v3 header".into(),
            ));
        }
        let mut db = self.conn()?;
        Self::ready(&db)?;
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO sessions(id,header,source_version) VALUES(?1,?2,3)",
            params![id, header],
        )?;
        Self::next_sequence(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn append_batch(
        &self,
        session: &str,
        command_id: &str,
        expected: u64,
        records: &[Vec<u8>],
    ) -> StorageResult<Ack> {
        valid_id(command_id)?;
        if records.is_empty() || records.len() > 100_000 {
            return Err(StorageError::Limit(
                "Journal batch must contain 1..100000 entries".into(),
            ));
        }
        if records
            .iter()
            .try_fold(0usize, |n, r| n.checked_add(r.len()))
            .is_none_or(|n| n > 128 * 1024 * 1024)
        {
            return Err(StorageError::Limit("Journal batch exceeds 128 MiB".into()));
        }
        let record_hashes: Vec<_> = records.iter().map(|r| digest(r)).collect();
        let intent = serde_json::to_vec(&(session, expected, record_hashes))?;
        let hash = digest(&intent);
        let mut db = self.conn()?;
        Self::ready(&db)?;
        if let Some(old) = command(&db, command_id, &hash)? {
            if old.state != "completed" {
                return Err(StorageError::Conflict(
                    "Command outcome uncertain; reconcile explicitly".into(),
                ));
            }
            return Ok(serde_json::from_slice(old.result.as_deref().ok_or_else(
                || StorageError::Corrupt("Missing command result".into()),
            )?)?);
        }
        let revision = active_session(&db, session)?;
        if revision != expected {
            return Err(StorageError::Conflict("Journal revision".into()));
        }
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        let mut ordinal: u64 = tx.query_row(
            "SELECT COALESCE(MAX(ordinal),0) FROM entries WHERE session=?1",
            [session],
            |r| r.get(0),
        )?;
        let mut leaf = None;
        for raw in records {
            ordinal += 1;
            let e = entry(raw, ordinal)?;
            insert_entry(&tx, session, &e)?;
            leaf = Some(e.id);
        }
        let revision = revision
            .checked_add(records.len() as u64)
            .filter(|x| *x <= i64::MAX as u64)
            .ok_or_else(|| StorageError::Limit("Revision exhausted".into()))?;
        tx.execute(
            "UPDATE sessions SET leaf=?2,revision=?3 WHERE id=?1",
            params![session, leaf, revision],
        )?;
        let ack = Ack {
            sequence: Self::next_sequence(&tx)?,
            revision,
        };
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,'completed',?4,?5)",
            params![
                command_id,
                hash,
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
    pub fn record_intent(&self, id: &str, intent: &[u8]) -> StorageResult<CommandResult> {
        valid_id(id)?;
        if intent.len() > 1024 * 1024 {
            return Err(StorageError::Limit("Intent exceeds 1 MiB".into()));
        }
        let hash = digest(intent);
        let mut db = self.conn()?;
        Self::ready(&db)?;
        if let Some(old) = command(&db, id, &hash)? {
            return Ok(old);
        }
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        let sequence = Self::next_sequence(&tx)?;
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,'intent',NULL,?4)",
            params![id, hash, intent, sequence],
        )?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(CommandResult {
            state: "intent".into(),
            intent: intent.into(),
            result: None,
            sequence,
        })
    }
    pub fn settle_intent(
        &self,
        id: &str,
        intent: &[u8],
        evidence: &[u8],
    ) -> StorageResult<CommandResult> {
        if evidence.is_empty() || evidence.len() > 4 * 1024 * 1024 {
            return Err(StorageError::Limit(
                "Bounded reconciliation evidence required".into(),
            ));
        }
        let mut db = self.conn()?;
        Self::ready(&db)?;
        let old = command(&db, id, &digest(intent))?
            .ok_or_else(|| StorageError::Missing("Intent".into()))?;
        if old.state == "completed" {
            if old.result.as_deref() != Some(evidence) {
                return Err(StorageError::Conflict("Outcome is immutable".into()));
            }
            return Ok(old);
        }
        let tx = db.transaction()?;
        let sequence = Self::next_sequence(&tx)?;
        tx.execute(
            "UPDATE commands SET state='completed',result=?2,sequence=?3 WHERE id=?1",
            params![id, evidence, sequence],
        )?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(CommandResult {
            state: "completed".into(),
            intent: intent.into(),
            result: Some(evidence.into()),
            sequence,
        })
    }
    pub fn command_result(&self, id: &str) -> StorageResult<Option<CommandResult>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        Ok(db
            .query_row(
                "SELECT state,intent,result,sequence FROM commands WHERE id=?1",
                [id],
                |r| {
                    Ok(CommandResult {
                        state: r.get(0)?,
                        intent: r.get(1)?,
                        result: r.get(2)?,
                        sequence: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }
    pub fn snapshot(&self, session: &str) -> StorageResult<JournalSnapshot> {
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        Self::snapshot_db(&db, session)
    }
    pub(crate) fn snapshot_db(db: &Connection, session: &str) -> StorageResult<JournalSnapshot> {
        let (header, leaf, revision) = db.query_row(
            "SELECT header,leaf,revision FROM sessions WHERE id=?1",
            [session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut q=db.prepare("SELECT id,parent,ordinal,raw,normalized FROM entries WHERE session=?1 ORDER BY ordinal")?;
        let entries = q
            .query_map([session], |r| {
                Ok(JournalEntry {
                    id: r.get(0)?,
                    parent: r.get(1)?,
                    ordinal: r.get(2)?,
                    raw: r.get(3)?,
                    normalized: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let sequence = db.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='sequence'",
            [],
            |r| r.get(0),
        )?;
        Ok(JournalSnapshot {
            session: session.into(),
            revision,
            sequence,
            header,
            leaf,
            entries,
        })
    }
    pub fn branch(&self, session: &str, leaf: Option<&str>) -> StorageResult<Vec<JournalEntry>> {
        let snapshot = self.snapshot(session)?;
        Self::branch_snapshot(&snapshot, leaf)
    }
    fn branch_snapshot(
        snapshot: &JournalSnapshot,
        leaf: Option<&str>,
    ) -> StorageResult<Vec<JournalEntry>> {
        let entries: BTreeMap<_, _> = snapshot
            .entries
            .iter()
            .map(|e| (e.id.as_str(), e))
            .collect();
        let mut id = leaf.or(snapshot.leaf.as_deref());
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        while let Some(current) = id {
            if !seen.insert(current) {
                return Err(StorageError::Corrupt("Cyclic journal ancestry".into()));
            }
            let e = entries
                .get(current)
                .ok_or_else(|| StorageError::Missing("Branch leaf/parent".into()))?;
            out.push((*e).clone());
            id = e.parent.as_deref();
        }
        out.reverse();
        Ok(out)
    }
    pub fn fork(&self, parent: &str, child: &str, leaf: Option<&str>) -> StorageResult<()> {
        valid_id(child)?;
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, parent)?;
        let snapshot = Self::snapshot_db(&db, parent)?;
        let branch = Self::branch_snapshot(&snapshot, leaf)?;
        let mut header: Value = serde_json::from_slice(&snapshot.header)?;
        header["id"] = child.into();
        header["parentSession"] = parent.into();
        header["version"] = 3.into();
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO sessions(id,header,leaf,revision,source_version) VALUES(?1,?2,?3,?4,3)",
            params![
                child,
                serde_json::to_vec(&header)?,
                branch.last().map(|e| &e.id),
                branch.len() as u64
            ],
        )?;
        for e in &branch {
            insert_entry(&tx, child, e)?;
        }
        // Opaque custom payloads may carry old cursors: copy only this parent's authorized closure.
        // Conservative over-retention is intentional; no global access widening.
        tx.execute("INSERT INTO access SELECT ?1,origin,artifact,'fork-closure' FROM access WHERE session=?2",params![child,parent])?;
        tx.execute("INSERT INTO leases SELECT ?1,origin,artifact,reason,expires,release_on_verification,raw FROM leases WHERE session=?2 AND expires>?3",params![child,parent,self.clock.now_ms()])?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(())
    }
    pub fn archive_session(&self, id: &str) -> StorageResult<()> {
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, id)?;
        let tx = db.transaction()?;
        tx.execute("UPDATE sessions SET archived=1 WHERE id=?1", [id])?;
        // Keep archive roots: raw historical journals can contain opaque references.
        // The parent can be hidden, but only an explicit destination discard removes this archive.
        tx.execute(
            "UPDATE access SET root='archived-history' WHERE session=?1",
            [id],
        )?;
        Self::next_sequence(&tx)?;
        tx.commit()?;
        Ok(())
    }
}

impl ag1_core::JournalPort for Store {
    fn append<'a>(
        &'a self,
        session: &'a ag1_core::SessionId,
        expected_revision: u64,
        record: &'a [u8],
    ) -> ag1_core::PortFuture<'a, u64> {
        Box::pin(async move {
            let command = format!(
                "append-{}",
                digest(
                    &serde_json::to_vec(&(session.as_str(), expected_revision, record))
                        .map_err(|e| ag1_core::DomainError::InvalidInput(e.to_string()))?
                )
            );
            self.append_batch(
                session.as_str(),
                &command,
                expected_revision,
                &[record.to_vec()],
            )
            .map(|a| a.revision)
            .map_err(super::persistence::domain_error)
        })
    }
}
