use crate::{Boundary, StorageError, StorageResult, Store, digest};
use crate::{
    journal::active_session,
    persistence::{valid_digest, valid_id},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use hmac::{Hmac, Mac};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use std::fs;

fn output_origin_key() -> StorageResult<[u8; 32]> {
    let mut key = [0u8; 32];
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
    }
    #[cfg(windows)]
    {
        // New output origins need an unpredictable cursor key. Windows CNG is
        // the native OS CSPRNG; using a clock or a process ID would make cursor
        // authorization predictable after a restart.
        let status = unsafe {
            windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
                std::ptr::null_mut(),
                key.as_mut_ptr(),
                key.len() as u32,
                windows_sys::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status != 0 {
            return Err(StorageError::Unsupported(
                "Windows system random generator".into(),
            ));
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        return Err(StorageError::Unsupported(
            "output origin random generator".into(),
        ));
    }
    Ok(key)
}

#[derive(Clone, Debug)]
pub struct ArtifactInput {
    pub origin: String,
    pub session: String,
    pub record: Vec<u8>,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactPage {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub next_offset: Option<u64>,
}
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecodedCursor {
    pub origin: String,
    pub artifact: String,
    pub offset: u64,
    pub max_bytes: u32,
    pub context: bool,
    pub anchor_line: Option<u32>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub origin: String,
    pub artifact: String,
    pub line: u64,
    pub text: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Lease {
    pub origin: String,
    pub artifact: String,
    pub reason: String,
    pub expires: u64,
    pub release_on_verification: bool,
}
fn artifact_id(id: &str) -> StorageResult<()> {
    if id.len() != 36
        || !id.starts_with("art_")
        || !id[4..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(StorageError::Invalid("Artifact ID".into()));
    }
    Ok(())
}
pub(crate) fn validate_metadata(raw: &[u8]) -> StorageResult<Value> {
    let v: Value = serde_json::from_slice(raw)?;
    let id = v["id"]
        .as_str()
        .ok_or_else(|| StorageError::Invalid("Artifact ID".into()))?;
    artifact_id(id)?;
    if v["version"] != 1
        || !v["sessionId"].is_string()
        || !v["kind"].is_string()
        || !v["producer"]["name"].is_string()
        || !v["sha256"].as_str().is_some_and(valid_digest)
        || v["bytes"].as_u64().is_none_or(|n| n > 256 * 1024 * 1024)
    {
        return Err(StorageError::Corrupt(
            "Artifact record/bytes mismatch".into(),
        ));
    }
    if v["encoding"] == "utf8" && v["mimeType"] != "text/plain" {
        return Err(StorageError::Corrupt("Text media type".into()));
    }
    if v["encoding"] == "binary" {
        let m = &v["binaryMetadata"];
        if !matches!(
            v["kind"].as_str(),
            Some("godot-visual" | "web-visual" | "native-visual")
        ) || !matches!(v["mimeType"].as_str(), Some("image/png" | "image/jpeg"))
            || v["bytes"]
                .as_u64()
                .is_none_or(|n| n == 0 || n > 8 * 1024 * 1024)
            || m["width"].as_u64().is_none_or(|n| n == 0 || n > 2048)
            || m["height"].as_u64().is_none_or(|n| n == 0 || n > 2048)
            || !m["purpose"].as_str().is_some_and(|s| !s.trim().is_empty())
            || !m["source"].is_string()
            || m["retentionState"] != "retained"
        {
            return Err(StorageError::Corrupt("Binary metadata bounds".into()));
        }
    }
    if !matches!(v["encoding"].as_str(), Some("utf8" | "binary")) {
        return Err(StorageError::Corrupt("Artifact encoding".into()));
    }
    Ok(v)
}
pub(crate) fn validate_record(raw: &[u8], bytes: &[u8]) -> StorageResult<Value> {
    let v = validate_metadata(raw)?;
    if v["sha256"] != digest(bytes) || v["bytes"].as_u64() != Some(bytes.len() as u64) {
        return Err(StorageError::Corrupt("Artifact bytes mismatch".into()));
    }
    match v["encoding"].as_str() {
        Some("utf8") => {
            std::str::from_utf8(bytes)
                .map_err(|_| StorageError::Corrupt("Invalid UTF-8 artifact".into()))?;
        }
        Some("binary") => {
            if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
                return Err(StorageError::Limit("Binary artifact exceeds 8 MiB".into()));
            }
            if v["mimeType"] == "image/png" {
                if bytes.len() < 24 || bytes[..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
                    return Err(StorageError::Corrupt("PNG envelope".into()));
                }
                let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                if v["binaryMetadata"]["width"] != w || v["binaryMetadata"]["height"] != h {
                    return Err(StorageError::Corrupt("PNG dimensions".into()));
                }
            } else if v["mimeType"] != "image/jpeg" {
                return Err(StorageError::Unsupported("Binary media type".into()));
            }
        }
        _ => return Err(StorageError::Invalid("Artifact encoding".into())),
    }
    Ok(v)
}
pub(crate) fn insert_artifact(
    db: &Connection,
    origin: &str,
    session: &str,
    raw: &[u8],
    hash: &str,
    record_hash: &str,
    v: &Value,
) -> StorageResult<()> {
    let origin_session: String = db
        .query_row("SELECT session FROM origins WHERE id=?1", [origin], |r| {
            r.get(0)
        })
        .optional()?
        .ok_or_else(|| StorageError::Missing("Artifact origin/key".into()))?;
    if v["sessionId"] != origin_session {
        return Err(StorageError::Denied);
    }
    let id = v["id"].as_str().unwrap();
    let previous: Option<String> = db
        .query_row(
            "SELECT record_digest FROM artifacts WHERE origin=?1 AND id=?2",
            params![origin, id],
            |r| r.get(0),
        )
        .optional()?;
    if previous.as_deref().is_some_and(|h| h != record_hash) {
        return Err(StorageError::Conflict("Immutable record collision".into()));
    }
    db.execute(
        "INSERT OR IGNORE INTO artifacts VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            origin,
            id,
            hash,
            record_hash,
            raw,
            v["encoding"].as_str().unwrap(),
            v["bytes"].as_u64().unwrap()
        ],
    )?;
    db.execute(
        "INSERT OR IGNORE INTO access VALUES(?1,?2,?3,'historical-evidence')",
        params![session, origin, id],
    )?;
    if v["encoding"] == "utf8" {
        db.execute(
            "INSERT OR REPLACE INTO catalog VALUES(?1,?2,?3)",
            params![origin, id, hash],
        )?;
    }
    Ok(())
}
fn authorized(
    db: &Connection,
    session: &str,
    origin: &str,
    id: &str,
) -> StorageResult<(String, String, u64)> {
    active_session(db, session)?;
    let (hash,encoding,size,raw,record_hash):(String,String,u64,Vec<u8>,String)=db.query_row("SELECT a.digest,a.encoding,a.bytes,a.record,a.record_digest FROM artifacts a JOIN access x ON x.origin=a.origin AND x.artifact=a.id WHERE x.session=?1 AND a.origin=?2 AND a.id=?3",params![session,origin,id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?.ok_or(StorageError::Denied)?;
    let record: Value = serde_json::from_slice(&raw)?;
    if digest(&raw) != record_hash
        || record["sha256"] != hash
        || record["encoding"] != encoding
        || record["bytes"].as_u64() != Some(size)
    {
        return Err(StorageError::Corrupt("Artifact metadata integrity".into()));
    }
    Ok((hash, encoding, size))
}
impl Store {
    /// Allocate custody for new native output after resuming a journal without
    /// an artifact store. Existing origins and cursor keys are never replaced.
    pub fn ensure_output_origin(&self, origin: &str, session: &str) -> StorageResult<bool> {
        valid_id(origin)?;
        valid_id(session)?;
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let tx = db.transaction()?;
        let existing: Option<(String, u64)> = tx
            .query_row(
                "SELECT session,length(key) FROM origins WHERE id=?1",
                [origin],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((owner, bytes)) = existing {
            if owner != session {
                return Err(StorageError::Denied);
            }
            if bytes != 32 {
                return Err(StorageError::Corrupt("Origin cursor key".into()));
            }
            return Ok(false);
        }
        let key = output_origin_key()?;
        tx.execute(
            "INSERT INTO origins VALUES(?1,?2,?3)",
            params![origin, session, key.as_slice()],
        )?;
        Self::next_sequence(&tx)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn register_origin(&self, origin: &str, session: &str, key: &[u8]) -> StorageResult<()> {
        valid_id(origin)?;
        valid_id(session)?;
        if key.len() != 32 {
            return Err(StorageError::Invalid(
                "Cursor key must contain 32 bytes".into(),
            ));
        }
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO origins VALUES(?1,?2,?3)",
            params![origin, session, key],
        )?;
        Self::next_sequence(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn put_artifact(&self, input: &ArtifactInput) -> StorageResult<String> {
        let v = validate_record(&input.record, &input.bytes)?;
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, &input.session)?;
        let owner: String = db
            .query_row(
                "SELECT session FROM origins WHERE id=?1",
                [&input.origin],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StorageError::Denied)?;
        if owner != input.session {
            return Err(StorageError::Denied);
        }
        if v["encoding"] == "binary" {
            let (count,total):(u64,u64)=db.query_row("SELECT count(*),COALESCE(sum(a.bytes),0) FROM artifacts a JOIN access x ON x.origin=a.origin AND x.artifact=a.id WHERE x.session=?1 AND a.encoding='binary'",[&input.session],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM access WHERE session=?1 AND origin=?2 AND artifact=?3)",params![input.session,input.origin,v["id"].as_str().unwrap()],|r|r.get(0))?;
            if !exists && (count >= 32 || total + input.bytes.len() as u64 > 128 * 1024 * 1024) {
                return Err(StorageError::Limit(
                    "Binary roots are pinned; explicit archival required before admission".into(),
                ));
            }
        }
        let (hash, compressed) =
            self.publish_custody(&input.bytes, v["kind"].as_str().unwrap_or(""))?;
        let record_hash = self.publish(&input.record)?;
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
        if compressed {
            tx.execute_batch("PRAGMA user_version=2;")?;
        }
        insert_artifact(
            &tx,
            &input.origin,
            &input.session,
            &input.record,
            &hash,
            &record_hash,
            &v,
        )?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok(v["id"].as_str().unwrap().into())
    }
    pub fn artifact_record(&self, session: &str, origin: &str, id: &str) -> StorageResult<Vec<u8>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        authorized(&db, session, origin, id)?;
        let (raw, hash): (Vec<u8>, String) = db.query_row(
            "SELECT record,record_digest FROM artifacts WHERE origin=?1 AND id=?2",
            params![origin, id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if digest(&raw) != hash {
            return Err(StorageError::Corrupt("Immutable record hash".into()));
        }
        self.verify_blob(&hash)?;
        Ok(raw)
    }
    /// Internal execution replay only. Renderer/model reads continue to use bounded authenticated cursors.
    pub(crate) fn execution_artifact_bytes(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        limit: u64,
    ) -> StorageResult<Vec<u8>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        let (hash, encoding, size) = authorized(&db, session, origin, id)?;
        if encoding != "utf8" || size > limit || limit > 128 * 1024 * 1024 {
            return Err(StorageError::Limit("execution artifact bytes".into()));
        }
        let evicted: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM evictions WHERE origin=?1 AND artifact=?2)",
            params![origin, id],
            |r| r.get(0),
        )?;
        if evicted {
            return Err(StorageError::Missing("evicted artifact".into()));
        }
        drop(db);
        let (actual_size, bytes) = self.blob_window(&hash, 0, limit as usize)?;
        if actual_size != size || bytes.len() as u64 != size {
            return Err(StorageError::Corrupt("execution artifact size".into()));
        }
        Ok(bytes)
    }
    pub fn page(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        offset: u64,
        limit: u32,
    ) -> StorageResult<ArtifactPage> {
        if limit == 0 || limit > 32768 || offset > 9_007_199_254_740_991 {
            return Err(StorageError::Limit("Artifact page bounds".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        let (hash, encoding, size) = authorized(&db, session, origin, id)?;
        let evicted: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM evictions WHERE origin=?1 AND artifact=?2)",
            params![origin, id],
            |r| r.get(0),
        )?;
        if evicted {
            return Err(StorageError::Missing(
                "Binary evidence was already evicted in the source snapshot; metadata is retained"
                    .into(),
            ));
        }
        let (actual_size, bytes) = self.blob_window(&hash, offset, limit as usize + 6)?;
        if actual_size != size {
            return Err(StorageError::Corrupt("Artifact size".into()));
        }
        if offset >= size {
            return Ok(ArtifactPage {
                offset,
                bytes: vec![],
                next_offset: None,
            });
        }
        let mut skip = 0;
        if encoding == "utf8" {
            while skip < 3 && skip < bytes.len() && bytes[skip] & 0xc0 == 0x80 {
                skip += 1;
            }
        }
        let mut end = (skip + limit as usize).min(bytes.len());
        if encoding == "utf8" {
            while end < bytes.len() && std::str::from_utf8(&bytes[skip..end]).is_err() {
                end += 1;
            }
            std::str::from_utf8(&bytes[skip..end])
                .map_err(|_| StorageError::Corrupt("Invalid UTF-8 page".into()))?;
        }
        let next = offset + end as u64;
        Ok(ArtifactPage {
            offset: offset + skip as u64,
            bytes: bytes[skip..end].into(),
            next_offset: (next < size).then_some(next),
        })
    }
    pub fn cursor(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        offset: u64,
        max_bytes: u32,
    ) -> StorageResult<String> {
        self.cursor_for_range(session, origin, id, (offset, max_bytes), None)
    }
    /// Authenticated surrounding-context navigation uses the same wire format
    /// and fallback range as the pinned SDK. The returned cursor remains subject
    /// to requester authorization on every read, including after fork/reopen.
    pub fn context_cursor(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        range: std::ops::Range<u64>,
        anchor_line: u32,
    ) -> StorageResult<String> {
        if range.end < range.start || range.end > 9_007_199_254_740_991 || anchor_line == 0 {
            return Err(StorageError::Invalid("Context cursor range/anchor".into()));
        }
        let exact = range.end - range.start;
        let (offset, max_bytes) = if (1..=32768).contains(&exact) {
            (range.start, exact as u32)
        } else {
            (range.start.saturating_sub(256), 1024)
        };
        self.cursor_for_range(session, origin, id, (offset, max_bytes), Some(anchor_line))
    }
    fn cursor_for_range(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        range: (u64, u32),
        anchor_line: Option<u32>,
    ) -> StorageResult<String> {
        let (offset, max_bytes) = range;
        artifact_id(id)?;
        if offset > 9_007_199_254_740_991 || max_bytes == 0 || max_bytes > 32768 {
            return Err(StorageError::Limit("Cursor bounds".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        authorized(&db, session, origin, id)?;
        let key: Vec<u8> = db.query_row("SELECT key FROM origins WHERE id=?1", [origin], |r| {
            r.get(0)
        })?;
        let mut bytes = [0u8; 32];
        bytes[0] = 2;
        bytes[1] = u8::from(anchor_line.is_some());
        bytes[28..32].copy_from_slice(&anchor_line.unwrap_or(0).to_be_bytes());
        for i in 0..16 {
            bytes[2 + i] = u8::from_str_radix(&id[4 + i * 2..6 + i * 2], 16)
                .map_err(|_| StorageError::Invalid("Artifact ID".into()))?;
        }
        bytes[18..26].copy_from_slice(&offset.to_be_bytes());
        bytes[26..28].copy_from_slice(&(max_bytes as u16).to_be_bytes());
        let encoded = B64.encode(bytes);
        let mut mac = Hmac::<Sha256>::new_from_slice(&key)
            .map_err(|_| StorageError::Corrupt("Cursor key".into()))?;
        mac.update(encoded.as_bytes());
        Ok(format!(
            "ac2.{encoded}.{}",
            B64.encode(&mac.finalize().into_bytes()[..16])
        ))
    }
    pub fn decode_cursor(&self, session: &str, cursor: &str) -> StorageResult<DecodedCursor> {
        let db = self.conn()?;
        Self::ready(&db)?;
        Self::decode_cursor_db(&db, session, cursor)
    }
    pub(crate) fn decode_cursor_db(
        db: &Connection,
        session: &str,
        cursor: &str,
    ) -> StorageResult<DecodedCursor> {
        if cursor.len() > 2048 {
            return Err(StorageError::Limit("Cursor length".into()));
        }
        let parts: Vec<_> = cursor.split('.').collect();
        if parts.len() != 3 || !matches!(parts[0], "ac1" | "ac2") {
            return Err(StorageError::Invalid("Cursor format".into()));
        }
        let decode = |s: &str| {
            B64.decode(s)
                .map_err(|_| StorageError::Invalid("Cursor base64".into()))
        };
        let bytes = decode(parts[1])?;
        let signature = decode(parts[2])?;
        if B64.encode(&bytes) != parts[1] || B64.encode(&signature) != parts[2] {
            return Err(StorageError::Invalid("Noncanonical cursor".into()));
        }
        let (id, offset, max_bytes, context, anchor, legacy_session) = if parts[0] == "ac2" {
            if bytes.len() != 32 || signature.len() != 16 || bytes[0] != 2 || bytes[1] > 1 {
                return Err(StorageError::Invalid("Binary cursor".into()));
            }
            let id = format!(
                "art_{}",
                bytes[2..18]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            let offset = u64::from_be_bytes(bytes[18..26].try_into().unwrap());
            let max = u16::from_be_bytes(bytes[26..28].try_into().unwrap()) as u32;
            let line = u32::from_be_bytes(bytes[28..32].try_into().unwrap());
            (
                id,
                offset,
                max,
                bytes[1] == 1,
                (line > 0).then_some(line),
                None,
            )
        } else {
            if signature.len() != 32 {
                return Err(StorageError::Invalid("Legacy cursor signature".into()));
            }
            let v: Value = serde_json::from_slice(&bytes)?;
            if v["v"] != 1 || !matches!(v["k"].as_str(), Some("page" | "context")) {
                return Err(StorageError::Invalid("Legacy cursor".into()));
            }
            let number = |k: &str| {
                v[k].as_u64()
                    .ok_or_else(|| StorageError::Invalid(format!("Cursor {k}")))
            };
            let max = u32::try_from(number("m")?)
                .map_err(|_| StorageError::Invalid("Cursor page size".into()))?;
            let anchor = if v.get("l").is_some() {
                Some(
                    u32::try_from(number("l")?)
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| StorageError::Invalid("Cursor anchor".into()))?,
                )
            } else {
                None
            };
            (
                v["i"]
                    .as_str()
                    .ok_or_else(|| StorageError::Invalid("Cursor artifact".into()))?
                    .into(),
                number("o")?,
                max,
                v["k"] == "context",
                anchor,
                Some(
                    v["s"]
                        .as_str()
                        .ok_or_else(|| StorageError::Invalid("Cursor session".into()))?
                        .to_owned(),
                ),
            )
        };
        artifact_id(&id)?;
        if offset > 9_007_199_254_740_991 || max_bytes == 0 || max_bytes > 32768 {
            return Err(StorageError::Invalid("Cursor bounds".into()));
        }
        active_session(db, session)?;
        let mut q=db.prepare("SELECT o.id,o.session,o.key FROM origins o JOIN access a ON a.origin=o.id WHERE a.session=?1 AND a.artifact=?2")?;
        let candidates = q.query_map(params![session, id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Vec<u8>>(2)?,
            ))
        })?;
        for row in candidates {
            let (origin, origin_session, key) = row?;
            if legacy_session
                .as_ref()
                .is_some_and(|s| s != &origin_session)
            {
                continue;
            }
            let mut mac = Hmac::<Sha256>::new_from_slice(&key)
                .map_err(|_| StorageError::Corrupt("Cursor key".into()))?;
            mac.update(parts[1].as_bytes());
            if mac.verify_truncated_left(&signature).is_ok() {
                return Ok(DecodedCursor {
                    origin,
                    artifact: id,
                    offset,
                    max_bytes,
                    context,
                    anchor_line: anchor,
                });
            }
        }
        Err(StorageError::Denied)
    }
    pub fn read_cursor(&self, session: &str, cursor: &str) -> StorageResult<ArtifactPage> {
        let c = self.decode_cursor(session, cursor)?;
        let mut page = self.page(session, &c.origin, &c.artifact, c.offset, c.max_bytes)?;
        if c.context {
            page.next_offset = None;
        }
        Ok(page)
    }
    pub fn search(
        &self,
        session: &str,
        query: &str,
        limit: usize,
    ) -> StorageResult<Vec<SearchHit>> {
        if query.is_empty() || query.len() > 512 || limit == 0 || limit > 100 {
            return Err(StorageError::Limit("Search bounds".into()));
        }
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let mut q=db.prepare("SELECT a.origin,a.id,a.digest FROM artifacts a JOIN access x ON x.origin=a.origin AND x.artifact=a.id WHERE x.session=?1 AND a.encoding='utf8' ORDER BY a.rowid DESC")?;
        let rows = q.query_map([session], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let needle = query.to_lowercase();
        let mut out = Vec::new();
        for row in rows {
            let (origin, id, hash) = row?;
            let bytes = self.blob_bytes(&hash)?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| StorageError::Corrupt("Search UTF-8".into()))?;
            for (line, text) in text.lines().enumerate() {
                if text.to_lowercase().contains(&needle) {
                    out.push(SearchHit {
                        origin: origin.clone(),
                        artifact: id.clone(),
                        line: line as u64 + 1,
                        text: text.chars().take(1024).collect(),
                    });
                    if out.len() == limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
    pub fn lease(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        reason: &str,
        ttl_ms: u64,
        release_on_verification: bool,
    ) -> StorageResult<Lease> {
        self.write_lease(
            session,
            origin,
            id,
            reason,
            ttl_ms,
            release_on_verification,
            None,
        )
        .map(|(lease, _)| lease)
    }
    /// Returns the exact record captured in the acknowledged transaction, not a
    /// later lookup that could observe a concurrent renewal or release.
    pub fn lease_record(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        reason: &str,
        ttl_ms: u64,
        release_on_verification: bool,
    ) -> StorageResult<Value> {
        self.write_lease(
            session,
            origin,
            id,
            reason,
            ttl_ms,
            release_on_verification,
            None,
        )
        .map(|(_, record)| record)
    }
    pub fn scoped_lease_record(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        reason: &str,
        ttl_ms: u64,
        release_on_verification: bool,
        verification_scope: Option<&str>,
    ) -> StorageResult<Value> {
        self.write_lease(
            session,
            origin,
            id,
            reason,
            ttl_ms,
            release_on_verification,
            verification_scope,
        )
        .map(|(_, record)| record)
    }
    fn write_lease(
        &self,
        session: &str,
        origin: &str,
        id: &str,
        reason: &str,
        ttl_ms: u64,
        release_on_verification: bool,
        verification_scope: Option<&str>,
    ) -> StorageResult<(Lease, Value)> {
        if let Some(scope) = verification_scope {
            ag1_core::tools::verification::scope(&serde_json::json!({"verificationScope":scope}))
                .map_err(|_| StorageError::Invalid("Verification scope".into()))?;
        }
        let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
        if reason.is_empty() || reason.encode_utf16().count() > 240 {
            return Err(StorageError::Invalid("Lease reason".into()));
        }
        let now = self.clock.now_ms();
        let expires = now
            .checked_add(ttl_ms.clamp(60_000, 86_400_000))
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| StorageError::Limit("Lease clock".into()))?;
        let mut db = self.conn()?;
        Self::ready(&db)?;
        authorized(&db, session, origin, id)?;
        let tx = db.transaction()?;
        tx.execute("DELETE FROM leases WHERE expires<=?1", [now])?;
        let count: u64 = tx.query_row(
            "SELECT count(*) FROM leases WHERE session=?1 AND NOT(origin=?2 AND artifact=?3)",
            params![session, origin, id],
            |r| r.get(0),
        )?;
        if count >= 16 {
            return Err(StorageError::Limit("16 active evidence leases".into()));
        }
        let stamp = |ms: u64| {
            chrono::DateTime::from_timestamp_millis(ms as i64)
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .ok_or_else(|| StorageError::Limit("Lease timestamp".into()))
        };
        let mut record = serde_json::json!({"version":1,"id":format!("lease_{}",&digest(format!("{session}:{origin}:{id}:{now}:{reason}").as_bytes())[..32]),"artifactId":id,"reason":reason,"createdAt":stamp(now)?,"expiresAt":stamp(expires)?,"releaseOnVerification":release_on_verification,"verificationScope":verification_scope});
        if verification_scope.is_none() {
            record.as_object_mut().unwrap().remove("verificationScope");
        }
        let raw = serde_json::to_vec(&record)?;
        tx.execute(
            "INSERT OR REPLACE INTO leases VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                session,
                origin,
                id,
                reason,
                expires,
                release_on_verification,
                raw
            ],
        )?;
        Self::next_sequence(&tx)?;
        self.hit(Boundary::BeforeCommit)?;
        tx.commit()?;
        self.hit(Boundary::AfterCommit)?;
        self.hit(Boundary::BeforeAck)?;
        Ok((
            Lease {
                origin: origin.into(),
                artifact: id.into(),
                reason,
                expires,
                release_on_verification,
            },
            serde_json::from_slice(&raw)?,
        ))
    }
    pub fn leases(&self, session: &str) -> StorageResult<Vec<Lease>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let mut q=db.prepare("SELECT origin,artifact,reason,expires,release_on_verification FROM leases WHERE session=?1 AND expires>?2 ORDER BY expires")?;
        Ok(q.query_map(params![session, self.clock.now_ms()], |r| {
            Ok(Lease {
                origin: r.get(0)?,
                artifact: r.get(1)?,
                reason: r.get(2)?,
                expires: r.get(3)?,
                release_on_verification: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?)
    }
    /// Explicit scoped settlement. Unscoped historical leases are never swept.
    pub fn release_verified_leases(
        &self,
        session: &str,
        scope: &str,
        started_ms: u64,
    ) -> StorageResult<usize> {
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let tx = db.transaction()?;
        let n = release_scoped_leases(&tx, session, Some(scope), started_ms)?;
        Self::next_sequence(&tx)?;
        tx.commit()?;
        Ok(n)
    }
    pub fn collect_orphans(&self) -> StorageResult<usize> {
        let db = self.conn()?;
        Self::ready(&db)?;
        let roots = Self::referenced_blobs(&db)?;
        let mut removed = 0;
        for e in fs::read_dir(self.root.join("blobs"))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            if crate::blob_codec::physical_hash(&name).is_some_and(|hash| !roots.contains(hash)) {
                fs::remove_file(e.path())?;
                removed += 1;
            }
        }
        super::persistence::sync_dir(&self.root.join("blobs"))?;
        Ok(removed)
    }
}
impl ag1_core::ArtifactPort for Store {
    fn read_page<'a>(
        &'a self,
        session: &'a ag1_core::SessionId,
        artifact: &'a ag1_core::ArtifactId,
        offset: u64,
        limit: u32,
    ) -> ag1_core::PortFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let origin = {
                let db = self.conn().map_err(super::persistence::domain_error)?;
                let mut q = db
                    .prepare("SELECT origin FROM access WHERE session=?1 AND artifact=?2")
                    .map_err(|e| ag1_core::DomainError::Io(e.to_string()))?;
                let origins = q
                    .query_map(params![session.as_str(), artifact.as_str()], |r| {
                        r.get::<_, String>(0)
                    })
                    .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
                    .map_err(|e| ag1_core::DomainError::Io(e.to_string()))?;
                if origins.len() != 1 {
                    return Err(ag1_core::DomainError::PermissionDenied);
                }
                origins[0].clone()
            };
            self.page(session.as_str(), &origin, artifact.as_str(), offset, limit)
                .map(|p| p.bytes)
                .map_err(super::persistence::domain_error)
        })
    }
}

/// Called inside the same transaction as the native verification receipt.
pub(crate) fn release_scoped_leases(
    tx: &rusqlite::Transaction<'_>,
    session: &str,
    scope: Option<&str>,
    started_ms: u64,
) -> StorageResult<usize> {
    let Some(scope) = scope else {
        return Ok(0);
    };
    ag1_core::tools::verification::scope(&serde_json::json!({"verificationScope":scope}))
        .map_err(|_| StorageError::Invalid("Verification scope".into()))?;
    let rows = {
        let mut query = tx.prepare(
            "SELECT origin,artifact,raw FROM leases WHERE session=?1 AND release_on_verification=1",
        )?;
        query
            .query_map([session], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut released = 0;
    for (origin, artifact, raw) in rows {
        let lease: Value = serde_json::from_slice(&raw)?;
        if lease["verificationScope"].as_str() != Some(scope) {
            continue;
        }
        let created = lease["createdAt"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis());
        // Timestamp ties are retained conservatively: evidence must predate the check.
        if created.is_none_or(|n| n < 0 || n as u64 >= started_ms) {
            continue;
        }
        released += tx.execute(
            "DELETE FROM leases WHERE session=?1 AND origin=?2 AND artifact=?3 AND raw=?4",
            params![session, origin, artifact, raw],
        )?;
    }
    Ok(released)
}
