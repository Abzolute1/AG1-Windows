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
        if v["kind"] != "godot-visual"
            || !matches!(v["mimeType"].as_str(), Some("image/png" | "image/jpeg"))
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
        let hash = self.publish(&input.bytes)?;
        let record_hash = self.publish(&input.record)?;
        self.hit(Boundary::BeforeTransaction)?;
        let tx = db.transaction()?;
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
        let bytes = self.blob_bytes(&hash)?;
        if bytes.len() as u64 != size {
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
        let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
        if reason.is_empty() || reason.chars().count() > 240 {
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
        let raw = serde_json::to_vec(
            &serde_json::json!({"version":1,"id":format!("lease_{}",&digest(format!("{session}:{origin}:{id}:{now}:{reason}").as_bytes())[..32]),"artifactId":id,"reason":reason,"createdAt":stamp(now)?,"expiresAt":stamp(expires)?,"releaseOnVerification":release_on_verification}),
        )?;
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
        tx.commit()?;
        Ok(Lease {
            origin: origin.into(),
            artifact: id.into(),
            reason,
            expires,
            release_on_verification,
        })
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
    pub fn release_verified_leases(&self, session: &str) -> StorageResult<usize> {
        let mut db = self.conn()?;
        Self::ready(&db)?;
        active_session(&db, session)?;
        let tx = db.transaction()?;
        let n = tx.execute(
            "DELETE FROM leases WHERE session=?1 AND release_on_verification=1",
            [session],
        )?;
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
            if valid_digest(&name) && !roots.contains(&name) {
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
