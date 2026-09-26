//! Disk-backed immutable catalog selection, with streaming blob search and a
//! bounded top-results set. Background inserts cannot change a captured search.
use crate::{
    CatalogPosition, CatalogRecord, StorageError, StorageResult, Store, journal::active_session,
};
use ag1_core::{
    DomainError, SessionId, evidence::EvidenceRequest, tools::artifact_scan::LiteralHit,
};
use rusqlite::params;
use serde_json::Value;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(1);

pub struct ArtifactSearchSnapshot {
    store: Arc<Store>,
    session: String,
    table: String,
    count: u64,
}
pub struct ArtifactSearchMatch {
    pub record: CatalogRecord,
    pub hits: Vec<LiteralHit>,
    pub total_matches: u64,
    pub occurrence_count: u64,
    salient: bool,
}
pub struct ArtifactSearchResult {
    pub matches: Vec<ArtifactSearchMatch>,
    pub scanned_artifacts: u64,
    pub candidate_artifacts: u64,
    pub scanned_blobs: u64,
    pub bytes_scanned: u64,
    pub truncated: bool,
}
struct Candidate {
    record: CatalogRecord,
    occurrences: u64,
}
impl Store {
    pub fn capture_artifact_search(
        self: &Arc<Self>,
        session: &str,
        kind: Option<&str>,
    ) -> StorageResult<ArtifactSearchSnapshot> {
        self.capture_search(session, kind, false)
    }
    pub fn capture_evidence_search(
        self: &Arc<Self>,
        session: &str,
        kind: Option<&str>,
    ) -> StorageResult<ArtifactSearchSnapshot> {
        self.capture_search(session, kind, true)
    }
    fn capture_search(
        self: &Arc<Self>,
        session: &str,
        kind: Option<&str>,
        evidence_only: bool,
    ) -> StorageResult<ArtifactSearchSnapshot> {
        if kind.is_some_and(|kind| kind.len() > 128) {
            return Err(StorageError::Limit("Artifact kind".into()));
        }
        let serial = NEXT_SNAPSHOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| StorageError::Limit("Catalog snapshot identity".into()))?;
        let table = format!("ag1_search_{serial}");
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let active: u64 = db.query_row("SELECT count(*) FROM sqlite_temp_master WHERE type='table' AND name LIKE 'ag1_search_%'",[],|row|row.get(0))?;
        if active >= 8 {
            return Err(StorageError::Limit("Eight active catalog searches".into()));
        }
        let tx = db.transaction()?;
        // Identifiers derive only from the host's monotonic integer above.
        tx.execute_batch(&format!("CREATE TEMP TABLE {table}(origin TEXT NOT NULL,id TEXT NOT NULL,digest TEXT NOT NULL,created TEXT NOT NULL,PRIMARY KEY(origin,id)) WITHOUT ROWID; CREATE INDEX {table}_digest ON {table}(digest,created DESC,id DESC,origin DESC); CREATE INDEX {table}_position ON {table}(created DESC,id DESC,origin DESC);"))?;
        tx.execute(&format!("INSERT INTO {table} SELECT a.origin,a.id,a.digest,json_extract(CAST(a.record AS TEXT),'$.createdAt') FROM artifacts a JOIN access x ON x.origin=a.origin AND x.artifact=a.id WHERE x.session=?1 AND a.encoding='utf8' AND (?2 IS NULL OR json_extract(CAST(a.record AS TEXT),'$.kind')=?2) AND (NOT ?3 OR json_extract(CAST(a.record AS TEXT),'$.kind') NOT IN (SELECT value FROM json_each(?4)))"),params![session,kind,evidence_only,crate::artifact_catalog::CUSTODY_KINDS])?;
        let count = tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })?;
        tx.commit()?;
        Ok(ArtifactSearchSnapshot {
            store: self.clone(),
            session: session.into(),
            table,
            count,
        })
    }
}
impl Drop for ArtifactSearchSnapshot {
    fn drop(&mut self) {
        if let Ok(db) = self.store.conn() {
            let _ = db.execute_batch(&format!("DROP TABLE IF EXISTS temp.{}", self.table));
        }
    }
}
impl ArtifactSearchSnapshot {
    fn candidates(&self, before: Option<&CatalogPosition>) -> StorageResult<Vec<Candidate>> {
        let db = self.store.conn()?;
        Store::ready(&db)?;
        active_session(&db, &self.session)?;
        let table = &self.table;
        let sql = format!(
            "SELECT t.origin,t.id,t.created,a.record,(SELECT count(*) FROM {table} d WHERE d.digest=t.digest) FROM {table} t LEFT JOIN artifacts a ON a.origin=t.origin AND a.id=t.id WHERE NOT EXISTS(SELECT 1 FROM {table} newer WHERE newer.digest=t.digest AND (newer.created,newer.id,newer.origin)>(t.created,t.id,t.origin)) AND (?1 IS NULL OR (t.created,t.id,t.origin)<(?1,?2,?3)) ORDER BY t.created DESC,t.id DESC,t.origin DESC LIMIT 16"
        );
        let mut query = db.prepare(&sql)?;
        let rows = query.query_map(
            params![
                before.map(|p| &p.created_at),
                before.map(|p| &p.id),
                before.map(|p| &p.origin)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, u64>(4)?,
                ))
            },
        )?;
        let mut candidates = Vec::new();
        for row in rows {
            let (origin, id, created_at, raw, occurrences) = row?;
            let raw =
                raw.ok_or_else(|| StorageError::Missing("Captured artifact metadata".into()))?;
            candidates.push(Candidate {
                record: CatalogRecord {
                    position: CatalogPosition {
                        created_at,
                        id,
                        origin,
                    },
                    record: serde_json::from_slice(&raw)?,
                },
                occurrences,
            });
        }
        Ok(candidates)
    }
    pub fn search(
        &self,
        query: &str,
        ignore_case: bool,
        max_artifacts: usize,
        matches_per_artifact: usize,
        mut check: impl FnMut() -> Result<(), DomainError>,
    ) -> Result<ArtifactSearchResult, DomainError> {
        use crate::persistence::domain_error;
        if query.is_empty()
            || query.encode_utf16().count() > 512
            || !(1..=50).contains(&max_artifacts)
            || !(1..=8).contains(&matches_per_artifact)
        {
            return Err(DomainError::InvalidInput(
                "Artifact catalog search bounds".into(),
            ));
        }
        let needle = if ignore_case {
            query.to_lowercase()
        } else {
            query.into()
        };
        let mut result = ArtifactSearchResult {
            matches: Vec::new(),
            scanned_artifacts: self.count,
            candidate_artifacts: 0,
            scanned_blobs: 0,
            bytes_scanned: 0,
            truncated: false,
        };
        let mut before = None;
        loop {
            check()?;
            let candidates = self.candidates(before.as_ref()).map_err(domain_error)?;
            if candidates.is_empty() {
                break;
            }
            before = Some(candidates.last().unwrap().record.position.clone());
            for candidate in candidates {
                check()?;
                let request = EvidenceRequest {
                    requester: SessionId::new(&self.session)?,
                    origin: candidate.record.position.origin.clone(),
                    artifact: candidate.record.position.id.clone(),
                };
                let scan = self.store.scan_artifact(
                    &request,
                    query,
                    ignore_case,
                    matches_per_artifact,
                    &mut check,
                )?;
                result.candidate_artifacts += candidate.occurrences;
                result.scanned_blobs += 1;
                result.bytes_scanned += scan.bytes;
                if scan.total_matches == 0 {
                    continue;
                }
                let salient = candidate.record.record["salientLines"]
                    .as_array()
                    .is_some_and(|lines| {
                        lines.iter().filter_map(Value::as_str).any(|line| {
                            if ignore_case {
                                line.to_lowercase().contains(&needle)
                            } else {
                                line.contains(&needle)
                            }
                        })
                    });
                result.truncated |= scan.total_matches > scan.hits.len() as u64;
                result.matches.push(ArtifactSearchMatch {
                    record: candidate.record,
                    hits: scan.hits,
                    total_matches: scan.total_matches,
                    occurrence_count: candidate.occurrences,
                    salient,
                });
                result.matches.sort_by(|a, b| {
                    b.salient
                        .cmp(&a.salient)
                        .then_with(|| b.total_matches.cmp(&a.total_matches))
                        .then_with(|| {
                            b.record
                                .position
                                .created_at
                                .cmp(&a.record.position.created_at)
                        })
                        .then_with(|| b.record.position.id.cmp(&a.record.position.id))
                        .then_with(|| b.record.position.origin.cmp(&a.record.position.origin))
                });
                if result.matches.len() > max_artifacts {
                    result.truncated = true;
                    result.matches.truncate(max_artifacts);
                }
            }
        }
        check()?;
        Ok(result)
    }
}
