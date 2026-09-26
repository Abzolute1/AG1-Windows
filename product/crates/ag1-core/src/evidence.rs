//! Source and observation evidence never becomes current merely by being persisted.
use crate::{
    DomainError, SessionId,
    task::{bounded_json, invalid, text},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Current,
    Stale,
    Missing,
    Invalid,
    Unverified,
}
impl Freshness {
    pub fn historical(self) -> bool {
        matches!(self, Self::Stale | Self::Missing | Self::Invalid)
    }
}
impl std::fmt::Display for Freshness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Missing => "missing",
            Self::Invalid => "invalid",
            Self::Unverified => "unverified",
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceReference {
    path: String,
    sha256: String,
}
impl SourceReference {
    pub fn new(path: &str, sha256: &str) -> Result<Self, DomainError> {
        Self::parse(&format!("file:{path}@sha256:{sha256}"))
    }
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        if value.len() > 8192 {
            return Err(invalid("source reference cap"));
        }
        let (path, digest) = value
            .strip_prefix("file:")
            .and_then(|s| s.rsplit_once("@sha256:"))
            .ok_or_else(|| invalid("Expected file:relative/path@sha256:<64 lowercase hex digits>. Reuse read details.sourceReference; a formatted tool-output artifact hash is not the source file revision."))?;
        let path = path.replace('\\', "/");
        if path.is_empty()
            || path.contains(['\r', '\n', '\0'])
            || path.starts_with('/')
            || path.split('/').any(|s| ["", ".", ".."].contains(&s))
            || path.as_bytes().get(1) == Some(&b':')
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("contained source reference"));
        }
        Ok(Self {
            path,
            sha256: digest.into(),
        })
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn digest(&self) -> &str {
        &self.sha256
    }
    pub fn reference(&self) -> String {
        format!("file:{}@sha256:{}", self.path, self.sha256)
    }
}
#[derive(Clone, Debug)]
pub struct SourceValidation {
    pub reference: String,
    pub status: Freshness,
    pub current: Option<String>,
}
#[derive(Clone, Debug)]
pub struct DisplaySlice {
    pub text: String,
    pub truncated: bool,
    pub full_bytes: usize,
    pub displayed_bytes: usize,
    pub normalized: bool,
}
/// Bytes/digest are produced together by the platform's validated file descriptor read.
/// Constructor is host-facing; the renderer never supplies a source snapshot as authority.
#[derive(Clone, Debug)]
pub struct SourceSnapshot {
    reference: SourceReference,
    snapshot: crate::ByteSnapshot,
}
impl SourceSnapshot {
    pub fn new(path: &str, snapshot: crate::ByteSnapshot) -> Result<Self, DomainError> {
        use sha2::{Digest, Sha256};
        if snapshot.bytes.len() > 8 * 1024 * 1024
            || snapshot.revision.size != snapshot.bytes.len() as u64
            || snapshot.revision.sha256 != format!("{:x}", Sha256::digest(&snapshot.bytes))
        {
            return Err(invalid("source bytes and revision must agree"));
        }
        let reference = SourceReference::new(path, &snapshot.revision.sha256)?;
        Ok(Self {
            reference,
            snapshot,
        })
    }
    pub fn reference(&self) -> &SourceReference {
        &self.reference
    }
    pub fn snapshot(&self) -> &crate::ByteSnapshot {
        &self.snapshot
    }

    pub fn display(&self, limit: usize) -> Result<DisplaySlice, DomainError> {
        if limit == 0 || limit > 8 * 1024 * 1024 {
            return Err(invalid("source display limit"));
        }
        let raw = std::str::from_utf8(&self.snapshot.bytes)
            .map_err(|_| invalid("binary source has no text projection"))?;
        let normalized = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
        let mut end = normalized.len().min(limit);
        while !normalized.is_char_boundary(end) {
            end -= 1;
        }
        Ok(DisplaySlice {
            text: normalized[..end].into(),
            truncated: end < normalized.len(),
            full_bytes: self.snapshot.bytes.len(),
            displayed_bytes: end,
            normalized: normalized != raw,
        })
    }
}
pub fn memory_sources(entry: &Value, validate: impl Fn(&str) -> Freshness) -> Freshness {
    let statuses: Vec<_> = entry["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| s.starts_with("file:"))
        .map(validate)
        .collect();
    for status in [Freshness::Invalid, Freshness::Missing, Freshness::Stale] {
        if statuses.contains(&status) {
            return status;
        }
    }
    if !statuses.is_empty() && statuses.iter().all(|s| *s == Freshness::Current) {
        Freshness::Current
    } else {
        Freshness::Unverified
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct InspectionEvidence(Value);
impl InspectionEvidence {
    pub fn parse(v: &Value) -> Result<Self, DomainError> {
        bounded_json(v)?;
        if v["version"] != 1 || !v["toolCallId"].is_string() || !v["scope"].is_object() {
            return Err(invalid("inspection evidence"));
        }
        SourceReference::parse(
            v["source"]
                .as_str()
                .ok_or_else(|| invalid("inspection source"))?,
        )?;
        let scope = &v["scope"];
        match v["kind"].as_str() {
            Some("read_range") => {
                let start = scope["startLine"]
                    .as_u64()
                    .ok_or_else(|| invalid("read range"))?;
                let end = scope["endLine"]
                    .as_u64()
                    .ok_or_else(|| invalid("read range"))?;
                if start == 0 || end < start {
                    return Err(invalid("read range"));
                }
            }
            Some("no_match") => {
                if scope["pattern"]
                    .as_str()
                    .is_none_or(|s| s.encode_utf16().count() > 1000)
                {
                    return Err(invalid("search scope"));
                }
            }
            _ => return Err(invalid("inspection kind")),
        }
        for k in ["literal", "ignoreCase"] {
            if scope.get(k).is_some() && !scope[k].is_boolean() {
                return Err(invalid("search flags"));
            }
        }
        if scope.get("glob").is_some() && !scope["glob"].is_string() {
            return Err(invalid("search glob"));
        }
        Ok(Self(v.clone()))
    }
    pub fn value(&self) -> &Value {
        &self.0
    }
}
pub fn inspection_history(journal: &[Value]) -> Vec<InspectionEvidence> {
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for e in journal.iter().rev() {
        if e["type"] != "custom" || e["customType"] != "inspection_evidence" {
            continue;
        }
        let Ok(e) = InspectionEvidence::parse(&e["data"]) else {
            continue;
        };
        let path = SourceReference::parse(e.0["source"].as_str().unwrap()).unwrap();
        let key = json!([e.0["kind"], path.path(), e.0["scope"]]).to_string();
        if seen.insert(key) {
            out.push(e);
            if out.len() == 20 {
                break;
            }
        }
    }
    out.reverse();
    out
}
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeObservation(Value);
impl RuntimeObservation {
    pub fn parse(v: &Value) -> Result<Self, DomainError> {
        bounded_json(v)?;
        let id = v["id"].as_str().ok_or_else(|| invalid("observation id"))?;
        if v["version"] != 1
            || id.len() != 16
            || !id.starts_with("obs_")
            || !id[4..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || !["pass", "fail", "partial", "blocked"]
                .iter()
                .any(|s| v["result"] == *s)
            || !["user", "agent", "verifier"]
                .iter()
                .any(|s| v["reporter"] == *s)
        {
            return Err(invalid("runtime observation"));
        }
        let mut out = v.clone();
        for (k, cap) in [
            ("feature", 240),
            ("environment", 240),
            ("revision", 240),
            ("reproduction", 1000),
            ("notes", 2000),
        ] {
            let t = text(
                v[k].as_str().ok_or_else(|| invalid("observation text"))?,
                cap,
            );
            if t.is_empty() {
                return Err(invalid("empty observation text"));
            }
            out[k] = json!(t);
        }
        let date = v["observedAt"]
            .as_str()
            .ok_or_else(|| invalid("observation time"))?;
        if chrono::DateTime::parse_from_rfc3339(date).is_err() {
            return Err(invalid("RFC3339 observation time"));
        }
        let sources = v["sources"]
            .as_array()
            .ok_or_else(|| invalid("observation sources"))?;
        let mut parsed = vec![];
        for source in sources {
            let t = text(
                source
                    .as_str()
                    .ok_or_else(|| invalid("observation source"))?,
                240,
            );
            if t.is_empty() {
                return Err(invalid("empty observation source"));
            }
            if parsed.len() < 8 {
                parsed.push(t);
            }
        }
        out["sources"] = json!(parsed);
        Ok(Self(out))
    }
    pub fn value(&self) -> &Value {
        &self.0
    }
    /// A claim is labelled by reporter and target revision; only current independent observation is proof.
    pub fn verified_for(
        &self,
        feature: &str,
        environment: &str,
        target_revision: &str,
        validate: impl Fn(&str) -> Freshness,
    ) -> bool {
        self.0["result"] == "pass"
            && self.0["feature"] == feature
            && self.0["environment"] == environment
            && self.0["revision"] == target_revision
            && self.0["reporter"] != "agent"
            && self.0["sources"].as_array().is_some_and(|a| {
                !a.is_empty()
                    && a.iter()
                        .all(|s| validate(s.as_str().unwrap()) == Freshness::Current)
            })
    }
}
/// Immutable source access hints. Every consumer still validates the referenced revision.
#[derive(Default)]
pub struct SourceAccess {
    sequence: u64,
    entries: BTreeMap<String, (u64, SourceReference)>,
}
impl SourceAccess {
    pub fn record(&mut self, source: SourceReference) -> Result<(), DomainError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("access sequence exhausted"))?;
        self.entries
            .insert(source.path().into(), (self.sequence, source));
        if self.entries.len() > 256 {
            let key = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.0)
                .unwrap()
                .0
                .clone();
            self.entries.remove(&key);
        }
        Ok(())
    }
    pub fn recent(
        &self,
        limit: usize,
        validate: impl Fn(&SourceReference) -> Freshness,
    ) -> Vec<(&SourceReference, Freshness)> {
        let mut rows: Vec<_> = self.entries.values().collect();
        rows.sort_by_key(|(seq, _)| std::cmp::Reverse(*seq));
        rows.into_iter()
            .take(limit.min(256))
            .map(|(_, r)| (r, validate(r)))
            .collect()
    }
}
#[derive(Clone, Debug)]
pub struct EvidenceRequest {
    pub requester: SessionId,
    pub origin: String,
    pub artifact: String,
}
#[derive(Clone, Debug)]
pub struct EvidencePage {
    pub bytes: Vec<u8>,
    pub offset: u64,
    pub next_offset: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct EvidenceHit {
    pub origin: String,
    pub artifact: String,
    pub line: u64,
    pub text: String,
}
/// Implementations authorize the requester for every read; a cursor is never authorization alone.
pub trait EvidencePort: Send + Sync {
    fn record(&self, request: &EvidenceRequest) -> Result<Value, DomainError>;
    fn page(
        &self,
        request: &EvidenceRequest,
        offset: u64,
        max_bytes: u32,
    ) -> Result<EvidencePage, DomainError>;
    fn read_cursor(&self, requester: &SessionId, cursor: &str)
    -> Result<EvidencePage, DomainError>;
    fn cursor(
        &self,
        request: &EvidenceRequest,
        offset: u64,
        max_bytes: u32,
    ) -> Result<String, DomainError>;
    fn search(
        &self,
        requester: &SessionId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<EvidenceHit>, DomainError>;
    fn lease(
        &self,
        request: &EvidenceRequest,
        reason: &str,
        ttl_ms: u64,
        release_on_verification: bool,
    ) -> Result<(), DomainError>;
}

pub enum ObservationRecord {
    Runtime(RuntimeObservation),
    Inspection(InspectionEvidence),
}
impl ObservationRecord {
    pub fn prepare(
        self,
        store: &impl crate::storage::StoragePort,
        session: SessionId,
        command: &str,
        entry_id: &str,
        now: &str,
    ) -> Result<crate::task::PreparedStateCommit, DomainError> {
        let journal = store.journal_snapshot(&session)?;
        let (kind, data) = match self {
            Self::Runtime(r) => ("runtime_observation", r.0),
            Self::Inspection(r) => ("inspection_evidence", r.0),
        };
        let record = json!({"type":"custom","id":entry_id,"parentId":journal.leaf,"timestamp":now,"customType":kind,"data":data});
        Ok(crate::task::PreparedStateCommit {
            session,
            command: command.into(),
            expected: journal.revision,
            records: vec![
                serde_json::to_vec(&record).map_err(|_| invalid("observation serialization"))?,
            ],
        })
    }
}
