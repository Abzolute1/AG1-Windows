//! Bounded catalog projection for an explicitly owned destination.
use crate::{StorageError, StorageResult, Store};
impl Store {
    pub fn client_sessions(&self, limit: usize) -> StorageResult<Vec<String>> {
        if limit == 0 || limit > 1000 {
            return Err(StorageError::Limit("session catalog page".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        let mut query =
            db.prepare("SELECT id FROM sessions WHERE archived=0 ORDER BY id LIMIT ?1")?;
        Ok(query
            .query_map([limit as u64], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
