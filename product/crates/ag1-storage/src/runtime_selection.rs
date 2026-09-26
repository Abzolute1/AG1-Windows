//! One durable selection commit point per native workspace. This does not adopt
//! a runtime, process or endpoint: the host must freeze/drain prior ownership.
use crate::{
    Boundary, StorageError, StorageResult, Store, digest,
    journal::{active_session, command},
    persistence::valid_id,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const KIND: &str = "native-runtime-selection";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeSelection {
    pub version: u32,
    pub workspace: String,
    pub session: String,
    /// Registry generation, independent of context-compaction epochs.
    pub epoch: u64,
    /// Fresh native owner identity; never a persisted PID or process capability.
    pub owner: String,
    pub journal_revision: u64,
    pub command: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionChange {
    pub expected: Option<String>,
    pub selection: RuntimeSelection,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectionAck {
    pub sequence: u64,
    pub sha256: String,
    pub selection: RuntimeSelection,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSelection {
    pub sha256: String,
    pub selection: RuntimeSelection,
}
fn validate(selection: &RuntimeSelection) -> StorageResult<()> {
    for id in [
        &selection.workspace,
        &selection.session,
        &selection.owner,
        &selection.command,
    ] {
        valid_id(id)?;
    }
    if selection.version != 1 || selection.epoch == 0 {
        return Err(StorageError::Invalid(
            "Native runtime selection version/epoch".into(),
        ));
    }
    Ok(())
}
fn load(
    db: &Connection,
    workspace: &str,
    conversation: Option<&str>,
) -> StorageResult<Option<(StoredSelection, Vec<u8>)>> {
    let (kind, key) = selection_key(workspace, conversation);
    let row: Option<(String, Vec<u8>)> = db
        .query_row(
            "SELECT digest,raw FROM metadata WHERE kind=?1 AND id=?2",
            params![kind, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(sha256, raw)| {
        if raw.len() > 16 * 1024 || digest(&raw) != sha256 {
            return Err(StorageError::Corrupt(
                "Native runtime selection digest/bounds".into(),
            ));
        }
        let selection: RuntimeSelection = serde_json::from_slice(&raw)?;
        validate(&selection)?;
        if selection.workspace != workspace
            || conversation.is_some_and(|id| id != selection.session)
        {
            return Err(StorageError::Corrupt("Native selection workspace".into()));
        }
        Ok((StoredSelection { sha256, selection }, raw))
    })
    .transpose()
}
fn selection_key(workspace: &str, conversation: Option<&str>) -> (&'static str, String) {
    match conversation {
        None => (KIND, workspace.into()),
        Some(session) => (
            "native-conversation-selection",
            digest(&serde_json::to_vec(&(workspace, session)).expect("string tuple")),
        ),
    }
}
impl Store {
    /// Independent conversation ownership, sharing the workspace journal store.
    pub fn conversation_selection(
        &self,
        workspace: &str,
        session: &str,
    ) -> StorageResult<Option<StoredSelection>> {
        valid_id(workspace)?;
        valid_id(session)?;
        let db = self.conn()?;
        Self::ready(&db)?;
        Ok(load(&db, workspace, Some(session))?.map(|(record, _)| record))
    }
    pub fn commit_conversation_selection(
        &self,
        change: &SelectionChange,
    ) -> StorageResult<SelectionAck> {
        self.commit_selection(change, true)
    }

    pub fn runtime_selection(&self, workspace: &str) -> StorageResult<Option<StoredSelection>> {
        valid_id(workspace)?;
        let db = self.conn()?;
        Self::ready(&db)?;
        Ok(load(&db, workspace, None)?.map(|(record, _)| record))
    }
    /// Atomically publish the selected owner and its immutable command receipt.
    /// Retry returns the original receipt, even after a later selection; callers
    /// must observe runtime_selection before activating an owner after recovery.
    pub fn commit_runtime_selection(
        &self,
        change: &SelectionChange,
    ) -> StorageResult<SelectionAck> {
        self.commit_selection(change, false)
    }
    fn commit_selection(
        &self,
        change: &SelectionChange,
        conversation: bool,
    ) -> StorageResult<SelectionAck> {
        validate(&change.selection)?;
        let session = conversation.then_some(change.selection.session.as_str());
        let (kind, key) = selection_key(&change.selection.workspace, session);
        if change
            .expected
            .as_ref()
            .is_some_and(|hash| hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(StorageError::Invalid("Native selection revision".into()));
        }
        let intent = serde_json::to_vec(&serde_json::json!({"kind":kind,"change":change}))?;
        if intent.len() > 32 * 1024 {
            return Err(StorageError::Limit("Native selection intent".into()));
        }
        let fingerprint = digest(&intent);
        let mut db = self.conn()?;
        Self::ready(&db)?;
        if let Some(record) = command(&db, &change.selection.command, &fingerprint)? {
            if record.state != "completed" {
                return Err(StorageError::Conflict(
                    "Native selection requires recovery".into(),
                ));
            }
            let ack: SelectionAck =
                serde_json::from_slice(record.result.as_deref().ok_or_else(|| {
                    StorageError::Corrupt("Missing native selection receipt".into())
                })?)?;
            if ack.selection != change.selection
                || ack.sha256 != digest(&serde_json::to_vec(&ack.selection)?)
            {
                return Err(StorageError::Corrupt("Native selection receipt".into()));
            }
            return Ok(ack);
        }
        let previous = load(&db, &change.selection.workspace, session)?;
        if previous.as_ref().map(|(old, _)| &old.sha256) != change.expected.as_ref() {
            return Err(StorageError::Conflict("Native selection revision".into()));
        }
        let expected_epoch = previous
            .as_ref()
            .map_or(Some(1), |(old, _)| old.selection.epoch.checked_add(1));
        if expected_epoch != Some(change.selection.epoch)
            || previous
                .as_ref()
                .is_some_and(|(old, _)| old.selection.owner == change.selection.owner)
        {
            return Err(StorageError::Conflict(
                "Native selection generation/owner".into(),
            ));
        }
        let revision = active_session(&db, &change.selection.session)?;
        if revision != change.selection.journal_revision {
            return Err(StorageError::RevisionConflict {
                expected: change.selection.journal_revision,
                actual: revision,
            });
        }
        let raw = serde_json::to_vec(&change.selection)?;
        let sha256 = self.publish(&raw)?;
        self.hit_journal_command(&change.selection.command, Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        if let Some((old, bytes)) = previous {
            tx.execute(
                "INSERT OR IGNORE INTO metadata VALUES('metadata-history',?1,?2,?3,1)",
                params![format!("{kind}:{key}:{}", old.sha256), old.sha256, bytes],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO metadata VALUES(?1,?2,?3,?4,0)",
            params![kind, key, sha256, raw],
        )?;
        let ack = SelectionAck {
            sequence: Self::next_sequence(&tx)?,
            sha256,
            selection: change.selection.clone(),
        };
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3,'completed',?4,?5)",
            params![
                change.selection.command,
                fingerprint,
                intent,
                serde_json::to_vec(&ack)?,
                ack.sequence
            ],
        )?;
        self.hit_journal_command(&change.selection.command, Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit_journal_command(&change.selection.command, Boundary::AfterCommit)?;
        self.hit_journal_command(&change.selection.command, Boundary::BeforeAck)?;
        Ok(ack)
    }
}
