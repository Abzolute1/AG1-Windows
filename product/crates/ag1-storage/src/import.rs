use crate::{Boundary, JournalEntry, StorageError, StorageResult, Store, digest};
use crate::{
    artifacts::{insert_artifact, validate_metadata, validate_record},
    journal::{entry, insert_entry},
    persistence::{
        no_symlink, private_dir, read_bounded, safe_relative, sync_dir, valid_digest, valid_id,
    },
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Journal,
    Memory,
    Artifact,
    Blob,
    CursorKey,
    Leases,
    Metadata,
    Extra,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrozenFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub kind: FileKind,
    pub session: Option<String>,
    pub origin: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrozenManifest {
    pub version: u32,
    pub frozen: bool,
    pub files: Vec<FrozenFile>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportStatus {
    pub id: String,
    pub checkpoint: u64,
    pub state: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryView {
    pub status: String,
    pub digest: String,
    pub normalized: Option<Value>,
    pub diagnostic: Option<String>,
}
type MemoryRow = (String, String, Option<Vec<u8>>, Option<String>);

/// Raw records are retained separately. Legacy IDs are deterministic, since v1 had none.
pub fn normalize_journal(
    raw: &[u8],
    source_hash: &str,
) -> StorageResult<(Vec<u8>, u32, Vec<JournalEntry>)> {
    if raw.len() > 256 * 1024 * 1024 {
        return Err(StorageError::Limit("Journal import exceeds 256 MiB".into()));
    }
    let mut lines = Vec::new();
    for line in raw.split_inclusive(|b| *b == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if line.len() > 4 * 1024 * 1024 {
            return Err(StorageError::Limit("JSONL entry exceeds 4 MiB".into()));
        }
        let v: Value = serde_json::from_slice(line)?;
        if !v.is_object() {
            return Err(StorageError::Corrupt("JSONL non-object".into()));
        }
        lines.push((line.to_vec(), v));
        if lines.len() > 100_001 {
            return Err(StorageError::Limit(
                "Journal snapshot exceeds 100000 entries".into(),
            ));
        }
    }
    if lines.is_empty() {
        return Err(StorageError::Corrupt("Empty journal".into()));
    }
    let header = &mut lines[0].1;
    if header["type"] != "session" {
        return Err(StorageError::Corrupt("Journal header".into()));
    }
    let id = header["id"]
        .as_str()
        .ok_or_else(|| StorageError::Corrupt("Session ID".into()))?;
    valid_id(id)?;
    let version = header
        .get("version")
        .map(|v| v.as_u64().unwrap_or(0))
        .unwrap_or(1);
    if !(1..=3).contains(&version) {
        return Err(StorageError::Unsupported(format!(
            "Session version {version}"
        )));
    }
    header["version"] = 3.into();
    let header = serde_json::to_vec(header)?;
    let ids: Vec<String> = (1..lines.len())
        .map(|i| {
            format!(
                "legacy-{}",
                &digest(format!("{source_hash}:{i}").as_bytes())[..32]
            )
        })
        .collect();
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for i in 1..lines.len() {
        let (raw, v) = &mut lines[i];
        if version == 1 {
            // Existing IDs are not silently destroyed if a nominal v1 extension supplied them.
            if v.get("id").is_some() || v.get("parentId").is_some() {
                return Err(StorageError::Unsupported(
                    "v1 with explicit IDs needs manual migration".into(),
                ));
            }
            v["id"] = ids[i - 1].clone().into();
            v["parentId"] = if i == 1 {
                Value::Null
            } else {
                ids[i - 2].clone().into()
            };
            if v["type"] == "compaction" {
                let index = v["firstKeptEntryIndex"]
                    .as_u64()
                    .filter(|n| *n > 0 && (*n as usize) < i)
                    .ok_or_else(|| StorageError::Corrupt("Legacy compaction index".into()))?;
                v["firstKeptEntryId"] = ids[index as usize - 1].clone().into();
                v.as_object_mut().unwrap().remove("firstKeptEntryIndex");
            }
        }
        if version < 3 && v["type"] == "message" && v["message"]["role"] == "hookMessage" {
            v["message"]["role"] = "custom".into();
        }
        let mut e = entry(&serde_json::to_vec(v)?, i as u64)?;
        e.raw = raw.clone();
        if e.parent.as_ref().is_some_and(|p| !seen.contains(p)) || !seen.insert(e.id.clone()) {
            return Err(StorageError::Corrupt(
                "Duplicate ID or missing/forward parent".into(),
            ));
        }
        entries.push(e);
    }
    Ok((header, version as u32, entries))
}
fn normalize_memory(raw: &[u8]) -> StorageResult<Value> {
    let mut v: Value = serde_json::from_slice(raw)?;
    if !matches!(v["version"].as_u64(), Some(1..=3))
        || !v["entries"].is_array()
        || v.get("archived").is_some_and(|a| !a.is_array())
    {
        return Err(StorageError::Corrupt("Invalid memory schema".into()));
    }
    let mut ids = BTreeSet::new();
    for key in ["entries", "archived"] {
        for e in v[key].as_array().into_iter().flatten() {
            let valid = e["id"].is_string()
                && e["text"].is_string()
                && e["createdAt"].is_number()
                && matches!(
                    e["kind"].as_str(),
                    Some("fact" | "decision" | "style" | "breakage")
                )
                && e.get("updatedAt").is_none_or(Value::is_number)
                && e.get("priority")
                    .is_none_or(|p| matches!(p.as_str(), Some("core" | "normal")))
                && e.get("component").is_none_or(|p| {
                    p.as_str()
                        .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 100)
                })
                && e.get("sources").is_none_or(|p| {
                    p.as_array().is_some_and(|a| {
                        a.len() <= 8
                            && a.iter().all(|p| {
                                p.as_str().is_some_and(|s| {
                                    !s.trim().is_empty()
                                        && s.chars().count() <= 500
                                        && !s.contains(['\r', '\n', '\0'])
                                })
                            })
                    })
                });
            if !valid
                || !ids.insert(e["id"].as_str().unwrap_or_default().to_owned())
                || e.get("component").is_some()
                    && matches!(e["kind"].as_str(), Some("fact" | "breakage"))
                    && e["sources"].as_array().is_none_or(Vec::is_empty)
            {
                return Err(StorageError::Corrupt(
                    "Invalid/duplicate memory entry; entire original retained".into(),
                ));
            }
        }
    }
    v["version"] = 3.into();
    Ok(v)
}
impl FrozenManifest {
    pub fn preflight(&self, source: &Path) -> StorageResult<()> {
        if self.version != 1 || !self.frozen || self.files.is_empty() || self.files.len() > 100_000
        {
            return Err(StorageError::Invalid(
                "A bounded frozen manifest is required".into(),
            ));
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        for f in &self.files {
            safe_relative(&f.path)?;
            if !valid_digest(&f.sha256) || !paths.insert(&f.path) {
                return Err(StorageError::Invalid(
                    "Manifest digest/duplicate path".into(),
                ));
            }
            total = total
                .checked_add(f.bytes)
                .ok_or_else(|| StorageError::Limit("Manifest size".into()))?;
            if f.bytes > 256 * 1024 * 1024 || total > 4 * 1024 * 1024 * 1024 {
                return Err(StorageError::Limit("Import file/bundle bounds".into()));
            }
            if let Some(s) = &f.session {
                valid_id(s)?;
            }
            if let Some(o) = &f.origin {
                valid_id(o)?;
            }
            let bytes = read_bounded(&source.join(&f.path), f.bytes)?;
            if bytes.len() as u64 != f.bytes || digest(&bytes) != f.sha256 {
                return Err(StorageError::Conflict(format!(
                    "Frozen source changed: {}",
                    f.path
                )));
            }
        }
        let mut actual = BTreeSet::new();
        let mut directories = vec![source.to_path_buf()];
        while let Some(dir) = directories.pop() {
            for item in fs::read_dir(dir)? {
                let item = item?;
                let kind = item.file_type()?;
                if kind.is_symlink() {
                    return Err(StorageError::Invalid("Symlink in frozen source".into()));
                }
                if kind.is_dir() {
                    directories.push(item.path());
                } else {
                    if !kind.is_file() {
                        return Err(StorageError::Invalid("Non-file frozen input".into()));
                    }
                    let path = item.path();
                    let relative = path
                        .strip_prefix(source)
                        .map_err(|_| StorageError::Invalid("Frozen source root".into()))?
                        .to_str()
                        .ok_or_else(|| StorageError::Invalid("Non-UTF8 source path".into()))?
                        .to_owned();
                    actual.insert(relative);
                    if actual.len() > 100_000 {
                        return Err(StorageError::Limit("Frozen source file count".into()));
                    }
                }
            }
        }
        if actual != paths.into_iter().cloned().collect() {
            return Err(StorageError::Conflict(
                "Frozen manifest does not cover the complete source directory".into(),
            ));
        }
        Ok(())
    }
}
impl Store {
    pub fn import_frozen(
        &self,
        source: &Path,
        manifest: &FrozenManifest,
    ) -> StorageResult<ImportStatus> {
        manifest.preflight(source)?;
        if source.canonicalize()?.starts_with(&self.root)
            || self.root.starts_with(source.canonicalize()?)
        {
            return Err(StorageError::Invalid(
                "Source and destination overlap".into(),
            ));
        }
        let encoded = serde_json::to_vec(manifest)?;
        let id = digest(&encoded);
        let mut db = self.conn()?;
        let existing: Option<(u64, String)> = db
            .query_row(
                "SELECT checkpoint,state FROM imports WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((checkpoint, state)) = &existing {
            if state == "complete" {
                return Ok(ImportStatus {
                    id,
                    checkpoint: *checkpoint,
                    state: state.clone(),
                });
            }
        } else {
            let count:i64=db.query_row("SELECT (SELECT count(*) FROM imports)+(SELECT count(*) FROM sessions)+(SELECT count(*) FROM commands)+(SELECT count(*) FROM memory)+(SELECT count(*) FROM metadata)",[],|r|r.get(0))?;
            if count != 0 {
                return Err(StorageError::Conflict(
                    "Import requires a new destination or the same interrupted import".into(),
                ));
            }
            db.execute(
                "INSERT INTO imports(id,manifest,state) VALUES(?1,?2,'staging')",
                params![id, encoded],
            )?;
        }
        let checkpoint = existing.map(|v| v.0).unwrap_or(0);
        for (index, file) in manifest.files.iter().enumerate().skip(checkpoint as usize) {
            let bytes = read_bounded(&source.join(&file.path), file.bytes)?;
            if digest(&bytes) != file.sha256 {
                return Err(StorageError::Conflict(
                    "Source changed after preflight".into(),
                ));
            }
            let hash = self.publish(&bytes)?;
            self.hit(Boundary::BeforeImportCheckpoint)?;
            let tx = db.transaction()?;
            tx.execute(
                "INSERT INTO archives VALUES(?1,?2,?3,?4)",
                params![id, file.path, hash, serde_json::to_vec(file)?],
            )?;
            tx.execute(
                "UPDATE imports SET checkpoint=?2 WHERE id=?1",
                params![id, index + 1],
            )?;
            tx.commit()?;
            self.hit(Boundary::AfterImportCheckpoint)?;
        }
        // Recheck the frozen set before final publication; all typed references use retained bytes.
        manifest.preflight(source)?;
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        for f in manifest
            .files
            .iter()
            .filter(|f| f.kind == FileKind::Journal)
        {
            let raw = self.blob_bytes(&f.sha256)?;
            let (header, version, entries) = normalize_journal(&raw, &f.sha256)?;
            let h: Value = serde_json::from_slice(&header)?;
            let session = h["id"].as_str().unwrap();
            if f.session.as_deref() != Some(session) {
                return Err(StorageError::Corrupt(
                    "Manifest/header session mismatch".into(),
                ));
            }
            tx.execute("INSERT INTO sessions(id,header,leaf,revision,source_version) VALUES(?1,?2,?3,?4,?5)",params![session,header,entries.last().map(|e|&e.id),entries.len(),version])?;
            for e in &entries {
                insert_entry(&tx, session, e)?;
            }
        }
        for f in manifest
            .files
            .iter()
            .filter(|f| f.kind == FileKind::CursorKey)
        {
            let key = self.blob_bytes(&f.sha256)?;
            let origin = f
                .origin
                .as_deref()
                .ok_or_else(|| StorageError::Invalid("Cursor origin".into()))?;
            let session = f
                .session
                .as_deref()
                .ok_or_else(|| StorageError::Invalid("Cursor session".into()))?;
            if key.len() != 32 {
                return Err(StorageError::Corrupt("Cursor key".into()));
            }
            tx.execute(
                "INSERT INTO origins VALUES(?1,?2,?3)",
                params![origin, session, key],
            )?;
        }
        // Legacy stores without cursor support have no key file. Allocate a new private origin key;
        // never invent a replacement for a key that was present but corrupt/missing from its manifest.
        for f in manifest
            .files
            .iter()
            .filter(|f| f.kind == FileKind::Artifact)
        {
            let origin = f
                .origin
                .as_deref()
                .ok_or_else(|| StorageError::Invalid("Artifact origin".into()))?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM origins WHERE id=?1)",
                [origin],
                |r| r.get(0),
            )?;
            if !exists {
                let raw = self.blob_bytes(&f.sha256)?;
                let record: Value = serde_json::from_slice(&raw)?;
                let session = record["sessionId"]
                    .as_str()
                    .ok_or_else(|| StorageError::Corrupt("Origin session".into()))?;
                let mut key = [0; 32];
                File::open("/dev/urandom")?.read_exact(&mut key)?;
                tx.execute(
                    "INSERT INTO origins VALUES(?1,?2,?3)",
                    params![origin, session, key.as_slice()],
                )?;
            }
        }
        for f in &manifest.files {
            match f.kind {
                FileKind::Artifact => {
                    let raw = self.blob_bytes(&f.sha256)?;
                    let v = validate_metadata(&raw)?;
                    let hash = v["sha256"]
                        .as_str()
                        .ok_or_else(|| StorageError::Corrupt("Artifact digest".into()))?;
                    let mut evicted = false;
                    if v["encoding"] == "binary"
                        && let Some(state_file) = manifest.files.iter().find(|x| {
                            x.origin == f.origin && x.path.ends_with("/godot-visual-retention.json")
                        })
                    {
                        let state: Value =
                            serde_json::from_slice(&self.blob_bytes(&state_file.sha256)?)?;
                        let retained = state["retainedArtifactIds"]
                            .as_array()
                            .filter(|a| a.len() <= 32 && a.iter().all(Value::is_string))
                            .ok_or_else(|| {
                                StorageError::Corrupt("Binary retention state".into())
                            })?;
                        if state["version"] != 1 {
                            return Err(StorageError::Corrupt("Binary retention version".into()));
                        }
                        evicted = !retained.contains(&v["id"]);
                    }
                    if !evicted
                        && !manifest
                            .files
                            .iter()
                            .any(|x| x.kind == FileKind::Blob && x.sha256 == hash)
                    {
                        return Err(StorageError::Corrupt(
                            "Artifact blob missing from frozen manifest".into(),
                        ));
                    }
                    if !evicted {
                        validate_record(&raw, &self.blob_bytes(hash)?)?;
                    }
                    insert_artifact(
                        &tx,
                        f.origin
                            .as_deref()
                            .ok_or_else(|| StorageError::Invalid("Artifact origin".into()))?,
                        f.session.as_deref().ok_or_else(|| {
                            StorageError::Invalid("Artifact access session".into())
                        })?,
                        &raw,
                        hash,
                        &f.sha256,
                        &v,
                    )?;
                    if evicted {
                        tx.execute(
                            "INSERT INTO evictions VALUES(?1,?2,'source visual retention')",
                            params![f.origin, v["id"].as_str().unwrap()],
                        )?;
                    }
                }
                FileKind::Memory => {
                    let bytes = self.blob_bytes(&f.sha256)?;
                    match normalize_memory(&bytes) {
                        Ok(v) => {
                            tx.execute(
                                "INSERT INTO memory VALUES(?1,'valid',?2,?3,NULL)",
                                params![f.path, f.sha256, serde_json::to_vec(&v)?],
                            )?;
                        }
                        Err(error) => {
                            tx.execute(
                                "INSERT INTO memory VALUES(?1,'corrupt',?2,NULL,?3)",
                                params![f.path, f.sha256, error.to_string()],
                            )?;
                        }
                    }
                }
                FileKind::Metadata | FileKind::Extra => {
                    let raw = self.blob_bytes(&f.sha256)?;
                    tx.execute(
                        "INSERT INTO metadata VALUES(?1,?2,?3,?4,1)",
                        params![
                            if f.kind == FileKind::Metadata {
                                "historical"
                            } else {
                                "extra"
                            },
                            f.path,
                            f.sha256,
                            raw
                        ],
                    )?;
                }
                _ => {}
            }
        }
        for f in manifest.files.iter().filter(|f| f.kind == FileKind::Leases) {
            let raw = self.blob_bytes(&f.sha256)?;
            let v: Value = serde_json::from_slice(&raw)?;
            let session = f
                .session
                .as_deref()
                .ok_or_else(|| StorageError::Invalid("Lease session".into()))?;
            let origin = f
                .origin
                .as_deref()
                .ok_or_else(|| StorageError::Invalid("Lease origin".into()))?;
            let leases = v["leases"]
                .as_array()
                .filter(|a| a.len() <= 16)
                .ok_or_else(|| StorageError::Corrupt("Lease list".into()))?;
            if v["version"] != 1 || v["sessionId"] != session {
                return Err(StorageError::Corrupt("Lease scope/version".into()));
            }
            for lease in leases {
                ag1_core::tools::verification::scope(lease).map_err(|_| StorageError::Corrupt("Lease verification scope".into()))?;
                let stamp = |key: &str| -> StorageResult<u64> {
                    let s = lease[key]
                        .as_str()
                        .ok_or_else(|| StorageError::Corrupt("Lease timestamp".into()))?;
                    u64::try_from(
                        chrono::DateTime::parse_from_rfc3339(s)
                            .map_err(|_| StorageError::Corrupt("Lease timestamp".into()))?
                            .timestamp_millis(),
                    )
                    .map_err(|_| StorageError::Corrupt("Lease timestamp".into()))
                };
                let expires = stamp("expiresAt")?;
                let created = stamp("createdAt")?;
                if expires < created || expires - created > 86_400_000 {
                    return Err(StorageError::Corrupt("Lease TTL".into()));
                }
                let artifact = lease["artifactId"]
                    .as_str()
                    .ok_or_else(|| StorageError::Corrupt("Lease artifact".into()))?;
                let authorized:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM access WHERE session=?1 AND origin=?2 AND artifact=?3)",params![session,origin,artifact],|r|r.get(0))?;
                if !authorized {
                    return Err(StorageError::Denied);
                }
                if expires > self.clock.now_ms() {
                    tx.execute(
                        "INSERT INTO leases VALUES(?1,?2,?3,?4,?5,?6,?7)",
                        params![
                            session,
                            origin,
                            artifact,
                            lease["reason"]
                                .as_str()
                                .ok_or_else(|| StorageError::Corrupt("Lease reason".into()))?,
                            expires,
                            lease["releaseOnVerification"].as_bool().unwrap_or(true),
                            serde_json::to_vec(lease)?
                        ],
                    )?;
                }
            }
        }
        // Validate recognizable evidence IDs and authenticated cursors before making the import visible.
        // Unknown reference encodings remain protected by retaining the complete imported store roots.
        {
            let mut q = tx.prepare("SELECT session,normalized FROM entries")?;
            for row in q.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })? {
                let (session, raw) = row?;
                let v: Value = serde_json::from_slice(&raw)?;
                validate_references(&tx, &session, &v)?;
            }
        }
        Self::next_sequence(&tx)?;
        tx.execute("UPDATE imports SET state='complete' WHERE id=?1", [&id])?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(ImportStatus {
            id,
            checkpoint: manifest.files.len() as u64,
            state: "complete".into(),
        })
    }
    pub fn import_status(&self) -> StorageResult<Option<ImportStatus>> {
        let db = self.conn()?;
        Ok(db
            .query_row("SELECT id,checkpoint,state FROM imports LIMIT 1", [], |r| {
                Ok(ImportStatus {
                    id: r.get(0)?,
                    checkpoint: r.get(1)?,
                    state: r.get(2)?,
                })
            })
            .optional()?)
    }
    pub fn memory(&self, id: &str) -> StorageResult<Option<MemoryView>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        let row: Option<MemoryRow> = db
            .query_row(
                "SELECT status,digest,normalized,diagnostic FROM memory WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        row.map(|(status, digest, raw, diagnostic)| {
            Ok(MemoryView {
                status,
                digest,
                normalized: raw.map(|b| serde_json::from_slice(&b)).transpose()?,
                diagnostic,
            })
        })
        .transpose()
    }
    pub fn save_memory(
        &self,
        id: &str,
        expected: Option<&str>,
        raw: &[u8],
    ) -> StorageResult<String> {
        if id.is_empty() || id.len() > 512 || id.contains('\0') {
            return Err(StorageError::Invalid("Memory identity".into()));
        }
        if raw.len() > 8 * 1024 * 1024 {
            return Err(StorageError::Limit("Memory envelope".into()));
        }
        let normalized = normalize_memory(raw)?;
        let mut db = self.conn()?;
        Self::ready(&db)?;
        let old: Option<(String, String)> = db
            .query_row("SELECT status,digest FROM memory WHERE id=?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if old.as_ref().is_some_and(|v| v.0 == "corrupt") {
            return Err(StorageError::Corrupt(
                "Quarantined memory cannot be silently replaced".into(),
            ));
        }
        if old.as_ref().map(|v| v.1.as_str()) != expected {
            return Err(StorageError::Conflict("Memory revision".into()));
        }
        let hash = self.publish(raw)?;
        let tx = db.transaction()?;
        if let Some((_, previous)) = old {
            let bytes = self.blob_bytes(&previous)?;
            tx.execute(
                "INSERT OR IGNORE INTO metadata VALUES('memory-history',?1,?2,?3,1)",
                params![format!("{id}:{previous}"), previous, bytes],
            )?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO memory VALUES(?1,'valid',?2,?3,NULL)",
            params![id, hash, serde_json::to_vec(&normalized)?],
        )?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(hash)
    }
    pub fn original_bytes(&self, path: &str) -> StorageResult<Vec<u8>> {
        let db = self.conn()?;
        let hash: String = db
            .query_row("SELECT digest FROM archives WHERE path=?1", [path], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(|| StorageError::Missing("Archived source".into()))?;
        self.blob_bytes(&hash)
    }
    /// Exports an atomic, independently verifiable store plus original bytes and compatible v3 views.
    /// Compatibility mode rejects evidence layouts an unpatched old reader cannot safely resume.
    pub fn export(&self, destination: &Path, require_compatible: bool) -> StorageResult<Value> {
        if destination.exists() {
            return Err(StorageError::Conflict("Export destination exists".into()));
        }
        let parent = destination
            .parent()
            .ok_or_else(|| StorageError::Invalid("Export parent".into()))?;
        no_symlink(parent)?;
        if parent.canonicalize()?.starts_with(&self.root) {
            return Err(StorageError::Invalid(
                "Export cannot be inside the store".into(),
            ));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        let corrupt: u64 = db.query_row(
            "SELECT count(*) FROM memory WHERE status='corrupt'",
            [],
            |r| r.get(0),
        )?;
        let foreign_roots:u64=db.query_row("SELECT count(*) FROM access x JOIN origins o ON x.origin=o.id WHERE x.session!=o.session",[],|r|r.get(0))?;
        let unknown:u64=db.query_row("SELECT count(*) FROM entries WHERE json_extract(CAST(normalized AS TEXT),'$.type') NOT IN ('message','thinking_level_change','model_change','compaction','branch_summary','custom','label','session_info','custom_message')",[],|r|r.get(0))?;
        let ambiguous: u64 = db.query_row(
            "SELECT count(*) FROM (SELECT session FROM origins GROUP BY session HAVING count(*)>1)",
            [],
            |r| r.get(0),
        )?;
        let uncertain: u64 = db.query_row(
            "SELECT count(*) FROM commands WHERE state!='completed'",
            [],
            |r| r.get(0),
        )?;
        let evicted: u64 = db.query_row("SELECT count(*) FROM evictions", [], |r| r.get(0))?;
        let structural =
            corrupt == 0 && foreign_roots == 0 && unknown == 0 && ambiguous == 0 && evicted == 0;
        let safe = structural && uncertain == 0;
        if require_compatible && !safe {
            return Err(StorageError::Unsupported("Unsafe original-runtime resume: corrupt memory, unknown entry type or inherited origin keys; use lossless archive".into()));
        }
        let temp = tempfile::Builder::new()
            .prefix(".ag1-export-")
            .tempdir_in(parent)?;
        private_dir(temp.path())?;
        db.backup(rusqlite::MAIN_DB, temp.path().join("storage.sqlite"), None)?;
        let mut files = Vec::new();
        {
            let mut write = |path: &str, bytes: &[u8]| -> StorageResult<()> {
                safe_relative(path)?;
                let output = temp.path().join(path);
                private_dir(output.parent().unwrap())?;
                let mut f = File::create(&output)?;
                f.write_all(bytes)?;
                f.sync_all()?;
                files.push(json!({"path":path,"sha256":digest(bytes),"bytes":bytes.len()}));
                Ok(())
            };
            let hashes = Self::referenced_blobs(&db)?;
            for hash in hashes {
                let (name, bytes) = self.export_blob(&hash)?;
                write(&format!("blobs/{name}"), &bytes)?;
            }
            let mut q = db.prepare("SELECT path,digest FROM archives ORDER BY path")?;
            for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                let (path, hash) = row?;
                write(&format!("original/{path}"), &self.blob_bytes(&hash)?)?;
            }
            let mut session_index = Vec::new();
            let mut q = db.prepare("SELECT id,archived FROM sessions ORDER BY id")?;
            for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))? {
                let (session, archived) = row?;
                let snap = Self::snapshot_db(&db, &session)?;
                session_index.push(json!({"id":session,"archived":archived,"leaf":snap.leaf}));
                let mut bytes = snap.header;
                bytes.push(b'\n');
                for e in snap.entries {
                    bytes.extend(e.normalized);
                    bytes.push(b'\n');
                }
                write(&format!("compatible/sessions/{session}.jsonl"), &bytes)?;
            }
            write("bridge/sessions.json", &serde_json::to_vec(&session_index)?)?;
            let mut q =
                db.prepare("SELECT id,normalized FROM memory WHERE status='valid' ORDER BY id")?;
            for row in q.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })? {
                let (id, bytes) = row?;
                write(
                    &format!("compatible/memory/{}.json", digest(id.as_bytes())),
                    &bytes,
                )?;
            }
            let mut q = db.prepare(
                "SELECT origin,id,record,digest,encoding FROM artifacts ORDER BY origin,id",
            )?;
            for row in q.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            })? {
                let (origin, id, record, hash, encoding) = row?;
                let evicted: bool = db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM evictions WHERE origin=?1 AND artifact=?2)",
                    params![origin, id],
                    |r| r.get(0),
                )?;
                let origin: String = if structural {
                    db.query_row("SELECT session FROM origins WHERE id=?1", [&origin], |r| {
                        r.get(0)
                    })?
                } else {
                    origin
                };
                write(
                    &format!("compatible/artifacts/{origin}/records/{id}.json"),
                    &record,
                )?;
                let path = format!(
                    "compatible/artifacts/{origin}/blobs/{hash}.{}",
                    if encoding == "binary" { "bin" } else { "txt" }
                );
                if !evicted && !temp.path().join(&path).exists() {
                    write(&path, &self.blob_bytes(&hash)?)?;
                }
            }
            let mut q = db.prepare("SELECT id,key FROM origins ORDER BY id")?;
            for row in q.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })? {
                let (id, key) = row?;
                let id: String = if structural {
                    db.query_row("SELECT session FROM origins WHERE id=?1", [&id], |r| {
                        r.get(0)
                    })?
                } else {
                    id
                };
                write(&format!("compatible/artifacts/{id}/cursor-key-v1"), &key)?;
            }
            if structural {
                let mut sessions = db.prepare("SELECT id FROM sessions ORDER BY id")?;
                for session in sessions.query_map([], |r| r.get::<_, String>(0))? {
                    let session = session?;
                    let mut query =
                        db.prepare("SELECT raw FROM leases WHERE session=?1 ORDER BY expires")?;
                    let mut leases = Vec::<Value>::new();
                    for raw in query.query_map([&session], |r| r.get::<_, Vec<u8>>(0))? {
                        leases.push(serde_json::from_slice(&raw?)?);
                    }
                    write(
                        &format!("compatible/artifacts/{session}/evidence-leases-v1.json"),
                        &serde_json::to_vec(
                            &json!({"version":1,"sessionId":session,"leases":leases}),
                        )?,
                    )?;
                }
            }
            // The P13 bridge can inspect interrupted receipts without treating
            // them as executable commands. Original-runtime compatibility above
            // remains stricter; the new projection never changes native records.
            let mut index = Vec::new();
            let mut query = db.prepare(
                "SELECT id,fingerprint,state,intent,result,sequence FROM commands ORDER BY id",
            )?;
            for row in query.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, Option<Vec<u8>>>(4)?,
                    r.get::<_, u64>(5)?,
                ))
            })? {
                let (id, fingerprint, state, intent, result, sequence) = row?;
                if digest(&intent) != fingerprint {
                    return Err(StorageError::Corrupt("Command intent digest".into()));
                }
                let path = format!("bridge/receipts/{}.json", digest(id.as_bytes()));
                write(
                    &path,
                    &serde_json::to_vec(
                        &json!({"version":1,"id":id,"fingerprint":fingerprint,"state":state,"intent":intent,"result":result,"sequence":sequence.to_string(),"replay_allowed":false}),
                    )?,
                )?;
                index.push(json!({"id":id,"path":path}));
            }
            write("bridge/receipts.json", &serde_json::to_vec(&index)?)?;
            let mut index = Vec::new();
            let mut query = db.prepare("SELECT id FROM memory ORDER BY id")?;
            for id in query.query_map([], |r| r.get::<_, String>(0))? {
                let id = id?;
                index.push(json!({"id":id,"path":format!("compatible/memory/{}.json", digest(id.as_bytes()))}));
            }
            write("bridge/memory.json", &serde_json::to_vec(&index)?)?;
            let mut index = Vec::new();
            let mut query =
                db.prepare("SELECT kind,id,digest,raw FROM metadata ORDER BY kind,id")?;
            for row in query.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            })? {
                let (kind, id, hash, raw) = row?;
                if digest(&raw) != hash {
                    return Err(StorageError::Corrupt("Metadata digest".into()));
                }
                let path = format!(
                    "bridge/metadata/{}.json",
                    digest(&serde_json::to_vec(&(&kind, &id))?)
                );
                write(&path, &raw)?;
                index.push(json!({"kind":kind,"id":id,"path":path}));
            }
            write("bridge/metadata.json", &serde_json::to_vec(&index)?)?;
        }
        let sqlite = fs::read(temp.path().join("storage.sqlite"))?;
        File::open(temp.path().join("storage.sqlite"))?.sync_all()?;
        files.push(json!({"path":"storage.sqlite","sha256":digest(&sqlite),"bytes":sqlite.len()}));
        let schema: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let manifest = json!({"version":1,"kind":"ag1-shadow-lossless-export","schema":schema,"compatible_resume":safe,"bridge_compatible_resume":structural,"runtime_authority":"historical-only","files":files});
        let mut f = File::create(temp.path().join("export.json"))?;
        f.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        f.sync_all()?;
        sync_tree(temp.path())?;
        self.hit(Boundary::BeforeRename)?;
        crate::persistence::publish_directory(temp.path(), destination)?;
        sync_dir(parent)?;
        self.hit(Boundary::AfterRename)?;
        Ok(manifest)
    }
    pub fn restore_export(source: &Path, destination: &Path) -> StorageResult<()> {
        if destination.exists() {
            return Err(StorageError::Conflict("Restore destination exists".into()));
        }
        let raw = read_bounded(&source.join("export.json"), 32 * 1024 * 1024)?;
        let manifest: Value = serde_json::from_slice(&raw)?;
        if manifest["version"] != 1
            || manifest["kind"] != "ag1-shadow-lossless-export"
            || !matches!(manifest["schema"].as_u64(), Some(1 | 2))
        {
            return Err(StorageError::Unsupported("Export format".into()));
        }
        let files = manifest["files"]
            .as_array()
            .filter(|f| f.len() <= 300_000)
            .ok_or_else(|| StorageError::Invalid("Export manifest".into()))?;
        let parent = destination
            .parent()
            .ok_or_else(|| StorageError::Invalid("Restore parent".into()))?;
        no_symlink(parent)?;
        let temp = tempfile::Builder::new()
            .prefix(".ag1-restore-")
            .tempdir_in(parent)?;
        let mut seen = BTreeSet::new();
        for f in files {
            let path = f["path"]
                .as_str()
                .ok_or_else(|| StorageError::Invalid("Export path".into()))?;
            safe_relative(path)?;
            if !seen.insert(path) {
                return Err(StorageError::Invalid("Duplicate export path".into()));
            }
            let length = f["bytes"]
                .as_u64()
                .filter(|n| *n <= 4 * 1024 * 1024 * 1024)
                .ok_or_else(|| StorageError::Limit("Export file size".into()))?;
            let bytes = read_bounded(&source.join(path), length)?;
            if bytes.len() as u64 != length || f["sha256"] != digest(&bytes) {
                return Err(StorageError::Corrupt("Export hash mismatch".into()));
            }
            // Original/compatible views are verified, but the independent store is restored from the DB + immutable blobs.
            if path == "storage.sqlite" || path.starts_with("blobs/") {
                let output = temp.path().join(path);
                private_dir(output.parent().unwrap())?;
                let mut file = File::create(output)?;
                file.write_all(&bytes)?;
                file.sync_all()?;
            }
        }
        if !seen.contains("storage.sqlite") {
            return Err(StorageError::Corrupt("Export database missing".into()));
        }
        // Validate the restored database and every referenced blob before publishing the destination.
        {
            let restored = Store::open(temp.path())?;
            let health = restored.reconcile()?;
            if Some(health.schema as u64) != manifest["schema"].as_u64() {
                return Err(StorageError::Corrupt("Export schema mismatch".into()));
            }
        }
        sync_tree(temp.path())?;
        crate::persistence::publish_directory(temp.path(), destination)?;
        sync_dir(parent)?;
        Ok(())
    }
}
fn sync_tree(path: &Path) -> StorageResult<()> {
    for e in fs::read_dir(path)? {
        let e = e?;
        if e.file_type()?.is_dir() {
            sync_tree(&e.path())?;
        }
    }
    sync_dir(path)
}

fn validate_references(
    db: &rusqlite::Connection,
    session: &str,
    value: &Value,
) -> StorageResult<()> {
    match value {
        Value::Array(a) => {
            for v in a {
                validate_references(db, session, v)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                validate_references(db, session, v)?;
            }
        }
        Value::String(s) => {
            for (start, _) in s.match_indices("art_") {
                if let Some(id) = s.get(start..start + 36).filter(|id| {
                    id[4..]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                }) {
                    let count: u64 = db.query_row(
                        "SELECT count(*) FROM access WHERE session=?1 AND artifact=?2",
                        params![session, id],
                        |r| r.get(0),
                    )?;
                    if count != 1 {
                        return Err(StorageError::Corrupt(format!(
                            "Missing/ambiguous historical evidence: {id}"
                        )));
                    }
                }
            }
            for prefix in ["ac1.", "ac2."] {
                for (start, _) in s.match_indices(prefix) {
                    let end = s[start..]
                        .bytes()
                        .position(|b| !b.is_ascii_alphanumeric() && !b"._-".contains(&b))
                        .unwrap_or(s.len() - start)
                        + start;
                    Store::decode_cursor_db(db, session, &s[start..end])?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
