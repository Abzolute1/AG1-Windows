//! Read-only, scoped restart inspection. Historical operation records grant no
//! process ownership or permission to replay a tool.
use crate::{StorageError, StorageResult, Store, persistence::valid_id};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Clone, Serialize, Deserialize)]
pub struct InterruptedCommand {
    pub key: String,
    pub intent: Value,
    pub intent_sha256: String,
}
impl Store {
    /// Startup turns unfinished command intents into `uncertain`. Page those
    /// records explicitly; live intents from this writer are never orphans.
    pub fn interrupted_commands(
        &self,
        workspace: &str,
        session: &str,
        after: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<InterruptedCommand>> {
        valid_id(workspace)?;
        valid_id(session)?;
        if limit == 0 || limit > 256 {
            return Err(StorageError::Limit("recovery page".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        let mut query=db.prepare("SELECT id,intent,fingerprint FROM commands WHERE state='uncertain' AND json_extract(CAST(intent AS TEXT),'$.kind')='native-session-command' AND json_extract(CAST(intent AS TEXT),'$.session')=?1 AND json_extract(CAST(intent AS TEXT),'$.payload.scope.workspace')=?2 AND (?3 IS NULL OR id>?3) ORDER BY id LIMIT ?4")?;
        query
            .query_map(params![session, workspace, after, limit as u64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .map(|row| {
                let (key, raw, sha) = row?;
                if crate::digest(&raw) != sha {
                    return Err(StorageError::Corrupt("command intent digest".into()));
                }
                Ok(InterruptedCommand {
                    key,
                    intent: serde_json::from_slice(&raw)?,
                    intent_sha256: sha,
                })
            })
            .collect()
    }
    pub fn session_tool_records(
        &self,
        workspace: &str,
        session: &str,
        after: Option<&str>,
        limit: usize,
    ) -> StorageResult<Vec<ag1_core::tools::executor::Record>> {
        valid_id(workspace)?;
        valid_id(session)?;
        if limit == 0 || limit > 256 {
            return Err(StorageError::Limit("recovery page".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        let mut query=db.prepare("SELECT raw,digest FROM metadata WHERE kind='tool-operation' AND json_extract(CAST(raw AS TEXT),'$.session')=?1 AND json_extract(CAST(raw AS TEXT),'$.workspace')=?2 AND (?3 IS NULL OR json_extract(CAST(raw AS TEXT),'$.operation')>?3) ORDER BY json_extract(CAST(raw AS TEXT),'$.operation') LIMIT ?4")?;
        query
            .query_map(params![session, workspace, after, limit as u64], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (raw, sha) = row?;
                if crate::digest(&raw) != sha {
                    return Err(StorageError::Corrupt("tool receipt digest".into()));
                }
                let record: ag1_core::tools::executor::Record = serde_json::from_slice(&raw)?;
                if record.version != 1 || record.workspace != workspace || record.session != session
                {
                    return Err(StorageError::Corrupt("tool receipt scope".into()));
                }
                Ok(record)
            })
            .collect()
    }
}
