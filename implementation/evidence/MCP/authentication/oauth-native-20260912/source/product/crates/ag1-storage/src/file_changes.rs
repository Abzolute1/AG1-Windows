//! Native file-effect receipts. Imported historical metadata is not authority
//! for a new local Undo operation. All reads are bound to the active requester.
use crate::{StorageError, StorageResult, Store, digest, journal::active_session};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

impl Store {
    pub fn native_file_undo(&self, session: &str, id: &str) -> StorageResult<Option<Value>> {
        if id.len() > 128 {
            return Err(StorageError::Limit("file undo id".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let row: Option<(Vec<u8>, String)> = db.query_row(
            "SELECT raw,digest FROM metadata WHERE kind='native-file-undo' AND id=?1 AND historical=0 AND json_extract(CAST(raw AS TEXT),'$.session')=?2",
            params![id, session], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        row.map(|(raw, sha)| verify(&raw, &sha, session))
            .transpose()
    }
    pub fn native_file_change(&self, session: &str, id: &str) -> StorageResult<Value> {
        if id.len() > 128 {
            return Err(StorageError::Limit("file change id".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let row: Option<(Vec<u8>, String)> = db.query_row(
            "SELECT raw,digest FROM metadata WHERE kind='native-file-change' AND id=?1 AND historical=0 AND json_extract(CAST(raw AS TEXT),'$.session')=?2",
            params![id, session], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let (raw, sha) = row.ok_or(StorageError::Denied)?;
        verify(&raw, &sha, session)
    }

    pub fn native_file_changes(
        &self,
        session: &str,
        calls: &[String],
    ) -> StorageResult<Vec<Value>> {
        if calls.len() > 32 || calls.iter().any(|call| call.len() > 256) {
            return Err(StorageError::Limit("file change call page".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let mut query = db.prepare("SELECT raw,digest FROM metadata WHERE kind='native-file-change' AND historical=0 AND json_extract(CAST(raw AS TEXT),'$.session')=?1 AND json_extract(CAST(raw AS TEXT),'$.call_id') IN (SELECT value FROM json_each(?2)) ORDER BY json_extract(CAST(raw AS TEXT),'$.created_ms'),id LIMIT 64")?;
        let rows = query.query_map(params![session, serde_json::to_string(calls)?], |row| {
            Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (raw, sha) = row?;
            verify(&raw, &sha, session)
        })
        .collect()
    }
}

fn verify(raw: &[u8], sha: &str, session: &str) -> StorageResult<Value> {
    if raw.len() > 65536 || digest(raw) != sha {
        return Err(StorageError::Corrupt("native file change receipt".into()));
    }
    let value: Value = serde_json::from_slice(raw)?;
    if value["version"] != 1 || value["session"] != session {
        return Err(StorageError::Corrupt("native file change scope".into()));
    }
    Ok(value)
}
