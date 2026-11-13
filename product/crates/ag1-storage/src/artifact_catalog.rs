//! Bounded, requester-authorized artifact navigation for native tools. Catalog
//! positions only choose a page; they never grant access to an origin/artifact.
use crate::{Boundary, StorageError, StorageResult, Store, journal::active_session};
use rusqlite::params;
use serde_json::Value;

// Native custody envelopes are retained and recoverable by id. They are not
// externalized evidence occurrences: indexing a tool's own arguments/request
// would make every search find itself and recursively duplicate old results.
pub(crate) const CUSTODY_KINDS: &str = r#"["tool-arguments","tool-rejected-arguments","tool-native-result","tool-original-result","tool-post-hook-result","tool-request-result","runtime-provider-output","episode-draft-output","runtime-request-input","runtime-request-compiled","runtime-extension-input","runtime-extension-output","context-original-journal","context-original-request","context-history-segment","native-change-bytes"]"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogPosition {
    pub created_at: String,
    pub id: String,
    pub origin: String,
}
#[derive(Clone, Debug)]
pub struct CatalogRecord {
    pub position: CatalogPosition,
    pub record: Value,
}
// One bounded preview cache per store. Database ownership remains the outer
// lock; both same-connection writes and external commits invalidate the entry.
pub(crate) struct CatalogCache {
    own_changes: u64,
    external_version: u64,
    session: String,
    limit: usize,
    records: Vec<CatalogRecord>,
}
const MAX_CACHED_RECORD_BYTES: usize = 64 * 1024;

impl Store {
    /// Newest first, with a stable origin tie-breaker for imported catalogs.
    /// The caller walks pages when it needs a complete catalog search.
    pub fn artifact_catalog(
        &self,
        session: &str,
        before: Option<&CatalogPosition>,
        limit: usize,
        kind: Option<&str>,
    ) -> StorageResult<Vec<CatalogRecord>> {
        self.catalog_page(session, before, limit, kind, false, None)
    }
    pub fn evidence_catalog(
        &self,
        session: &str,
        limit: usize,
    ) -> StorageResult<Vec<CatalogRecord>> {
        self.catalog_page(session, None, limit, None, true, None)
    }
    pub fn call_artifact_catalog(
        &self,
        session: &str,
        calls: &[String],
        before: Option<&CatalogPosition>,
        limit: usize,
    ) -> StorageResult<Vec<CatalogRecord>> {
        if calls.len() > 1024 || calls.iter().any(|id| id.len() > 256) {
            return Err(StorageError::Limit(
                "Turn output calls; use conversation artifacts".into(),
            ));
        }
        self.catalog_page(session, before, limit, None, true, Some(calls))
    }
    fn catalog_page(
        &self,
        session: &str,
        before: Option<&CatalogPosition>,
        limit: usize,
        kind: Option<&str>,
        evidence_only: bool,
        calls: Option<&[String]>,
    ) -> StorageResult<Vec<CatalogRecord>> {
        if limit == 0
            || limit > 128
            || kind.is_some_and(|k| k.len() > 128)
            || before.is_some_and(|p| {
                p.created_at.len() > 128 || p.id.len() > 128 || p.origin.len() > 128
            })
        {
            return Err(StorageError::Limit("Artifact catalog page".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let cache_stamp = if evidence_only && before.is_none() && kind.is_none() && calls.is_none()
        {
            let external_version: u64 =
                db.query_row("PRAGMA data_version", [], |row| row.get(0))?;
            let own_changes = db.total_changes();
            let cached = self
                .catalog_cache
                .lock()
                .map_err(|_| StorageError::Corrupt("Poisoned catalog cache".into()))?;
            if let Some(entry) = cached.as_ref().filter(|entry| {
                entry.own_changes == own_changes
                    && entry.external_version == external_version
                    && entry.session == session
                    && entry.limit == limit
            }) {
                return Ok(entry.records.clone());
            }
            Some((own_changes, external_version))
        } else {
            None
        };
        let mut query = db.prepare("SELECT a.origin,a.id,a.record,COALESCE(json_extract(CAST(a.record AS TEXT),'$.createdAt'),'') AS created FROM artifacts a JOIN access x ON x.origin=a.origin AND x.artifact=a.id WHERE x.session=?1 AND (?2 IS NULL OR json_extract(CAST(a.record AS TEXT),'$.kind')=?2) AND (?3 IS NULL OR (COALESCE(json_extract(CAST(a.record AS TEXT),'$.createdAt'),''),a.id,a.origin)<(?3,?4,?5)) AND (NOT ?7 OR json_extract(CAST(a.record AS TEXT),'$.kind') NOT IN (SELECT value FROM json_each(?8))) AND (?9 IS NULL OR (a.origin=?1 AND json_extract(CAST(a.record AS TEXT),'$.producer.callId') IN (SELECT value FROM json_each(?9)))) ORDER BY created DESC,a.id DESC,a.origin DESC LIMIT ?6")?;
        let rows = query.query_map(
            params![
                session,
                kind,
                before.map(|p| &p.created_at),
                before.map(|p| &p.id),
                before.map(|p| &p.origin),
                limit,
                evidence_only,
                CUSTODY_KINDS,
                calls.map(serde_json::to_string).transpose()?
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?;
        let mut out = Vec::with_capacity(limit);
        let mut record_bytes = 0usize;
        for row in rows {
            let (origin, id, bytes, created_at) = row?;
            record_bytes = record_bytes.saturating_add(bytes.len());
            out.push(CatalogRecord {
                position: CatalogPosition {
                    origin,
                    id,
                    created_at,
                },
                record: serde_json::from_slice(&bytes)?,
            });
        }
        if let Some((own_changes, external_version)) = cache_stamp {
            let mut cached = self
                .catalog_cache
                .lock()
                .map_err(|_| StorageError::Corrupt("Poisoned catalog cache".into()))?;
            *cached = (record_bytes <= MAX_CACHED_RECORD_BYTES).then(|| CatalogCache {
                own_changes,
                external_version,
                session: session.into(),
                limit,
                records: out.clone(),
            });
        }
        Ok(out)
    }
    pub fn artifact_origin(&self, session: &str, id: &str) -> StorageResult<String> {
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let mut query = db.prepare(
            "SELECT origin FROM access WHERE session=?1 AND artifact=?2 ORDER BY origin LIMIT 2",
        )?;
        let origins = query
            .query_map(params![session, id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        match origins.as_slice() {
            [origin] => Ok(origin.clone()),
            [] => Err(StorageError::Missing("Artifact".into())),
            _ => Err(StorageError::Conflict("Ambiguous artifact origin".into())),
        }
    }
    pub fn evidence_lease_records(&self, session: &str) -> StorageResult<Vec<Value>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let mut query = db.prepare("SELECT l.raw FROM leases l JOIN access x ON x.session=l.session AND x.origin=l.origin AND x.artifact=l.artifact WHERE l.session=?1 AND l.expires>?2 ORDER BY json_extract(CAST(l.raw AS TEXT),'$.createdAt'),l.artifact,l.origin")?;
        let rows = query.query_map(params![session, self.clock.now_ms()], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        let mut out = Vec::new();
        for row in rows {
            if out.len() == 16 {
                return Err(StorageError::Corrupt("Evidence lease count".into()));
            }
            out.push(serde_json::from_slice(&row?)?);
        }
        Ok(out)
    }
    /// Release only this requester's active lease, by lease id or artifact id.
    /// Foreign/unknown identifiers release nothing and reveal no foreign state.
    pub fn release_evidence_lease(&self, session: &str, target: &str) -> StorageResult<usize> {
        if target.is_empty() || target.len() > 128 || target.contains('\0') {
            return Err(StorageError::Invalid("Evidence lease target".into()));
        }
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let now = self.clock.now_ms();
        let tx = db.transaction()?;
        let count = tx.execute("DELETE FROM leases WHERE session=?1 AND expires>?2 AND (artifact=?3 OR json_extract(CAST(raw AS TEXT),'$.id')=?3)", params![session, now, target])?;
        tx.execute(
            "DELETE FROM leases WHERE session=?1 AND expires<=?2",
            params![session, now],
        )?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(count)
    }
}

impl Store {
    /// Match and hash the same streamed bytes. A mutation or corrupt blob cannot
    /// publish trusted matches based on a separate, earlier verification pass.
    pub fn scan_artifact(
        &self,
        request: &ag1_core::evidence::EvidenceRequest,
        query: &str,
        ignore_case: bool,
        max_matches: usize,
        mut check: impl FnMut() -> Result<(), ag1_core::DomainError>,
    ) -> Result<ag1_core::tools::artifact_scan::LiteralScan, ag1_core::DomainError> {
        use crate::persistence::domain_error;
        use ag1_core::{DomainError, evidence::EvidencePort, tools::artifact_scan};
        check()?;
        let record = EvidencePort::record(self, request)?;
        if record["encoding"] != "utf8" {
            return Err(DomainError::InvalidInput(
                "Binary artifacts cannot be searched as text".into(),
            ));
        }
        let hash = record["sha256"].as_str().ok_or(DomainError::Conflict)?;
        let file = self.blob_reader(hash).map_err(domain_error)?;
        let result = artifact_scan::scan(file, query, ignore_case, max_matches, &mut check)?;
        if result.sha256 != hash || record["bytes"].as_u64() != Some(result.bytes) {
            return Err(DomainError::InvalidInput(
                "Artifact changed or failed its integrity check".into(),
            ));
        }
        // Re-check requester access before publishing the complete scan.
        if EvidencePort::record(self, request)? != record {
            return Err(DomainError::Conflict);
        }
        check()?;
        Ok(result)
    }
}

#[cfg(test)]
mod preview_cache_tests {
    use super::*;
    use crate::{ArtifactInput, digest};
    use serde_json::json;

    fn fixture() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        for session in ["parent", "stranger"] {
            store.create_session(session, &serde_json::to_vec(&json!({"type":"session","version":3,"id":session,"cwd":"/fixture","timestamp":"2026-09-13T00:00:00Z"})).unwrap()).unwrap();
        }
        store.register_origin("origin", "parent", &[7; 32]).unwrap();
        (dir, store)
    }
    fn put(store: &Store, number: u32, kind: &str, extra: &str) -> String {
        let id = format!("art_{number:032x}");
        let bytes = format!("evidence {number}").into_bytes();
        let record = json!({"version":1,"id":id,"sessionId":"parent","kind":kind,"mimeType":"text/plain","encoding":"utf8","createdAt":"2026-09-13T00:00:00Z","sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"fixture"},"extra":extra});
        store
            .put_artifact(&ArtifactInput {
                origin: "origin".into(),
                session: "parent".into(),
                record: serde_json::to_vec(&record).unwrap(),
                bytes,
            })
            .unwrap();
        id
    }
    #[test]
    fn cached_preview_tracks_local_and_external_changes_without_cross_session_access() {
        let (_dir, store) = fixture();
        let first = put(&store, 1, "text", "");
        put(&store, 2, "context-history-segment", "custody only");
        assert_eq!(
            store
                .artifact_catalog("parent", None, 20, None)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(store.evidence_catalog("parent", 20).unwrap().len(), 1);
        let stamp = store
            .catalog_cache
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .own_changes;
        assert_eq!(
            store.evidence_catalog("parent", 20).unwrap()[0].position.id,
            first
        );
        assert_eq!(
            store
                .catalog_cache
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .own_changes,
            stamp
        );
        let third = put(&store, 3, "text", "");
        assert_eq!(store.evidence_catalog("parent", 20).unwrap().len(), 2);
        assert_eq!(
            store.evidence_catalog("parent", 1).unwrap()[0].position.id,
            third
        );
        assert!(store.evidence_catalog("stranger", 20).unwrap().is_empty());
        assert_eq!(store.evidence_catalog("parent", 20).unwrap().len(), 2);
        store
            .conn()
            .unwrap()
            .execute(
                "DELETE FROM access WHERE session='parent' AND artifact=?1",
                [&first],
            )
            .unwrap();
        assert_eq!(store.evidence_catalog("parent", 20).unwrap().len(), 1);
        // Deliberate external SQL against this disposable fixture proves that
        // total_changes alone is insufficient; data_version must also change.
        let external = rusqlite::Connection::open(store.root.join("storage.sqlite")).unwrap();
        external
            .execute(
                "DELETE FROM access WHERE session='parent' AND artifact=?1",
                [&third],
            )
            .unwrap();
        assert!(store.evidence_catalog("parent", 20).unwrap().is_empty());
        external
            .execute(
                "INSERT INTO access VALUES('parent','origin',?1,'historical-evidence')",
                [&third],
            )
            .unwrap();
        assert_eq!(
            store.evidence_catalog("parent", 20).unwrap()[0].position.id,
            third
        );
        store.archive_session("parent").unwrap();
        assert!(matches!(
            store.evidence_catalog("parent", 20),
            Err(StorageError::Missing(_))
        ));
    }
    #[test]
    fn cache_stays_bounded_and_never_bypasses_incomplete_import_or_argument_checks() {
        let (_dir, store) = fixture();
        put(&store, 1, "text", &"x".repeat(MAX_CACHED_RECORD_BYTES));
        assert_eq!(store.evidence_catalog("parent", 20).unwrap().len(), 1);
        assert!(store.catalog_cache.lock().unwrap().is_none());
        assert!(matches!(
            store.evidence_catalog("parent", 0),
            Err(StorageError::Limit(_))
        ));
        // Populate an ordinary empty preview, then fence storage with an import.
        assert!(store.evidence_catalog("stranger", 20).unwrap().is_empty());
        assert!(store.catalog_cache.lock().unwrap().is_some());
        store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO imports(id,manifest,state) VALUES('fixture',X'7b7d','staging')",
                [],
            )
            .unwrap();
        assert!(matches!(
            store.evidence_catalog("stranger", 20),
            Err(StorageError::ImportIncomplete)
        ));
    }
}
