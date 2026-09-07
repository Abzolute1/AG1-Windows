//! Bounded, requester-authorized artifact navigation for native tools. Catalog
//! positions only choose a page; they never grant access to an origin/artifact.
use crate::{Boundary, StorageError, StorageResult, Store, journal::active_session};
use rusqlite::params;
use serde_json::Value;

// Native custody envelopes are retained and recoverable by id. They are not
// externalized evidence occurrences: indexing a tool's own arguments/request
// would make every search find itself and recursively duplicate old results.
pub(crate) const CUSTODY_KINDS: &str = r#"["tool-arguments","tool-rejected-arguments","tool-native-result","tool-original-result","tool-post-hook-result","tool-request-result","runtime-provider-output","episode-draft-output","runtime-request-input","runtime-extension-input","runtime-extension-output","context-original-journal","context-original-request"]"#;

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
        self.catalog_page(session, before, limit, kind, false)
    }
    pub fn evidence_catalog(
        &self,
        session: &str,
        limit: usize,
    ) -> StorageResult<Vec<CatalogRecord>> {
        self.catalog_page(session, None, limit, None, true)
    }
    fn catalog_page(
        &self,
        session: &str,
        before: Option<&CatalogPosition>,
        limit: usize,
        kind: Option<&str>,
        evidence_only: bool,
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
        let mut query = db.prepare("SELECT a.origin,a.id,a.record,json_extract(CAST(a.record AS TEXT),'$.createdAt') AS created FROM artifacts a JOIN access x ON x.origin=a.origin AND x.artifact=a.id WHERE x.session=?1 AND (?2 IS NULL OR json_extract(CAST(a.record AS TEXT),'$.kind')=?2) AND (?3 IS NULL OR (json_extract(CAST(a.record AS TEXT),'$.createdAt'),a.id,a.origin)<(?3,?4,?5)) AND (NOT ?7 OR json_extract(CAST(a.record AS TEXT),'$.kind') NOT IN (SELECT value FROM json_each(?8))) ORDER BY created DESC,a.id DESC,a.origin DESC LIMIT ?6")?;
        let rows = query.query_map(
            params![
                session,
                kind,
                before.map(|p| &p.created_at),
                before.map(|p| &p.id),
                before.map(|p| &p.origin),
                limit,
                evidence_only,
                CUSTODY_KINDS
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
        for row in rows {
            let (origin, id, bytes, created_at) = row?;
            out.push(CatalogRecord {
                position: CatalogPosition {
                    origin,
                    id,
                    created_at,
                },
                record: serde_json::from_slice(&bytes)?,
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
        use crate::persistence::{domain_error, no_symlink};
        use ag1_core::{DomainError, evidence::EvidencePort, tools::artifact_scan};
        check()?;
        let record = EvidencePort::record(self, request)?;
        if record["encoding"] != "utf8" {
            return Err(DomainError::InvalidInput(
                "Binary artifacts cannot be searched as text".into(),
            ));
        }
        let hash = record["sha256"].as_str().ok_or(DomainError::Conflict)?;
        let path = self.blob_path(hash).map_err(domain_error)?;
        no_symlink(&path).map_err(domain_error)?;
        let file =
            std::fs::File::open(&path).map_err(|error| DomainError::Io(error.to_string()))?;
        if !file
            .metadata()
            .map_err(|error| DomainError::Io(error.to_string()))?
            .is_file()
        {
            return Err(DomainError::InvalidInput("Non-file artifact blob".into()));
        }
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
