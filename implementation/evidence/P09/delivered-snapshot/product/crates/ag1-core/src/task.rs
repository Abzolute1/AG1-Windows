//! Validated task reducers. Declared progress is distinct from compilation and runtime evidence.
use crate::{DomainError, SessionId, storage::StoragePort};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const KINDS: &[&str] = &[
    "FACT",
    "OBSERVATION",
    "HYPOTHESIS",
    "DECISION",
    "CONSTRAINT",
    "ASSUMPTION",
    "RESULT",
    "FAILURE",
];
const REQUIRED: &[&str] = &["FACT", "OBSERVATION", "CONSTRAINT", "RESULT", "FAILURE"];
pub(crate) fn invalid(message: &str) -> DomainError {
    DomainError::InvalidInput(message.into())
}
pub(crate) fn text(value: &str, cap: usize) -> String {
    let normalized = value
        .split(|c: char| c.is_whitespace() || c == '\u{feff}')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    // Respect the reference UTF-16 budget without creating invalid surrogate strings.
    let mut units = 0;
    normalized
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= cap
        })
        .collect()
}
pub(crate) fn units(value: &str) -> usize {
    if value.is_ascii() {
        value.len()
    } else {
        value.encode_utf16().count()
    }
}
pub(crate) fn list(value: &Value, cap: usize) -> Result<Vec<String>, DomainError> {
    let rows = value
        .as_array()
        .ok_or_else(|| invalid("string list required"))?;
    if rows.len() > 100_000 {
        return Err(DomainError::LimitExceeded("input list".into()));
    }
    let mut out = vec![];
    for item in rows {
        if let Some(s) = item.as_str() {
            let s = text(s, 500);
            if !s.is_empty() && !out.contains(&s) {
                out.push(s);
                if out.len() == cap {
                    break;
                }
            }
        }
    }
    Ok(out)
}
fn member(v: &Value, choices: &[&str]) -> bool {
    v.as_str().is_some_and(|s| choices.contains(&s))
}
pub(crate) fn bounded_json(v: &Value) -> Result<(), DomainError> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("state size"))?;
            if self.0 > 4 * 1024 * 1024 {
                return Err(std::io::Error::other("state size"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Counter(0), v)
        .map_err(|_| DomainError::LimitExceeded("state document".into()))
}
fn id_valid(id: &str) -> bool {
    id.len() == 11
        && id.starts_with("ts_")
        && id[3..]
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn normalize_entry(v: &Value, new: bool) -> Result<Value, DomainError> {
    if !v.is_object() || !member(&v["kind"], KINDS) {
        return Err(invalid("epistemic kind"));
    }
    let mut e = v.clone();
    let statement = text(
        v["statement"]
            .as_str()
            .ok_or_else(|| invalid("statement"))?,
        1000,
    );
    if statement.is_empty() {
        return Err(invalid("empty statement"));
    }
    e["statement"] = json!(statement);
    if v.get("confidence").is_some() && !member(&v["confidence"], &["low", "medium", "high"]) {
        return Err(invalid("confidence"));
    }
    let sources = list(v.get("sources").unwrap_or(&json!([])), 10)?;
    if new && REQUIRED.contains(&v["kind"].as_str().unwrap()) && sources.is_empty() {
        return Err(invalid("claim requires provenance"));
    }
    e["sources"] = json!(sources);
    if let Some(reason) = v.get("reason") {
        let reason = text(reason.as_str().ok_or_else(|| invalid("reason"))?, 1000);
        if reason.is_empty() {
            e.as_object_mut().unwrap().remove("reason");
        } else {
            e["reason"] = json!(reason);
        }
    }
    Ok(e)
}
#[derive(Clone, Debug, PartialEq)]
pub struct TaskState(Value);
impl TaskState {
    pub fn empty(now: &str) -> Self {
        Self(
            json!({"version":1,"status":"active","constraints":[],"openQuestions":[],"entries":[],"updatedAt":now}),
        )
    }
    pub fn value(&self) -> &Value {
        &self.0
    }
    pub fn parse(v: &Value) -> Result<Self, DomainError> {
        bounded_json(v)?;
        if v["version"] != 1
            || !member(&v["status"], &["active", "completed", "blocked"])
            || !v["updatedAt"].is_string()
        {
            return Err(invalid("task snapshot"));
        }
        let mut base = Self::empty(v["updatedAt"].as_str().unwrap());
        let entries = v["entries"].as_array().ok_or_else(|| invalid("entries"))?;
        let mut ids = BTreeSet::new();
        let mut parsed = vec![];
        for entry in entries {
            let id = entry["id"].as_str().ok_or_else(|| invalid("entry id"))?;
            if !id_valid(id)
                || !ids.insert(id)
                || !member(&entry["status"], &["active", "resolved", "superseded"])
                || !entry["createdAt"].is_string()
                || !entry["updatedAt"].is_string()
            {
                return Err(invalid("entry shape or duplicate identity"));
            }
            parsed.push(normalize_entry(entry, false)?);
        }
        if parsed.len() > 50 {
            parsed.drain(..parsed.len() - 50);
        }
        base.0["entries"] = json!(parsed);
        let mut patch = v.clone();
        patch
            .as_object_mut()
            .ok_or_else(|| invalid("task object"))?
            .remove("entries");
        // Historical dangling links cannot become active authority after bounded migration.
        for (key, kind) in [("blockerId", "FAILURE"), ("checkpointId", "RESULT")] {
            if !base
                .entries()
                .iter()
                .any(|e| e["id"] == v[key] && e["kind"] == kind && e["status"] == "active")
            {
                patch.as_object_mut().unwrap().remove(key);
            }
        }
        if let Some(a) = v["attemptIds"].as_array() {
            patch["attemptIds"] = json!(
                a.iter()
                    .filter(|id| base.entries().iter().any(|e| e["id"] == **id))
                    .rev()
                    .take(10)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
            );
        }
        base.set(&patch, v["updatedAt"].as_str().unwrap())
    }
    fn entries(&self) -> &Vec<Value> {
        self.0["entries"].as_array().unwrap()
    }
    pub fn set(&self, patch: &Value, now: &str) -> Result<Self, DomainError> {
        bounded_json(patch)?;
        if !patch.is_object() {
            return Err(invalid("task patch"));
        }
        let mut out = self.0.clone();
        if let Some(status) = patch.get("status") {
            if !member(status, &["active", "completed", "blocked"]) {
                return Err(invalid("task status"));
            }
            out["status"] = status.clone();
        }
        for (key, cap) in [
            ("objective", 2000),
            ("currentPhase", 500),
            ("currentSubgoal", 500),
            ("nextAction", 500),
            ("latestInstruction", 2000),
        ] {
            if let Some(v) = patch.get(key) {
                let v = text(v.as_str().ok_or_else(|| invalid("task text"))?, cap);
                if v.is_empty() {
                    out.as_object_mut().unwrap().remove(key);
                } else {
                    out[key] = json!(v);
                }
            }
        }
        for key in ["constraints", "openQuestions"] {
            if let Some(v) = patch.get(key) {
                out[key] = json!(list(v, 20)?);
            }
        }
        for (key, kind) in [("blockerId", "FAILURE"), ("checkpointId", "RESULT")] {
            if let Some(v) = patch.get(key) {
                let id = v.as_str().ok_or_else(|| invalid("executive link"))?;
                if id.is_empty() {
                    out.as_object_mut().unwrap().remove(key);
                } else if self
                    .entries()
                    .iter()
                    .any(|e| e["id"] == id && e["kind"] == kind && e["status"] == "active")
                {
                    out[key] = json!(id);
                } else {
                    return Err(invalid("executive link needs active matching entry"));
                }
            }
        }
        if let Some(a) = patch.get("attemptIds") {
            let a = a.as_array().ok_or_else(|| invalid("attempt ids"))?;
            let mut ids = vec![];
            for id in a {
                if !self.entries().iter().any(|e| e["id"] == *id) {
                    return Err(invalid("unknown attempt"));
                }
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
            if ids.len() > 10 {
                ids.drain(..ids.len() - 10);
            }
            out["attemptIds"] = json!(ids);
        }
        if out["status"] == "completed" && out.get("blockerId").is_some() {
            return Err(invalid("resolve blocker before completion"));
        }
        out["updatedAt"] = json!(now);
        bounded_json(&out)?;
        Ok(Self(out))
    }
    pub fn reset(objective: &str, now: &str) -> Result<Self, DomainError> {
        if text(objective, 2000).is_empty() {
            return Err(invalid("empty objective"));
        }
        Self::empty(now).set(&json!({"objective":objective}), now)
    }
    pub fn record(
        &self,
        input: &Value,
        id: &str,
        now: &str,
    ) -> Result<(Self, String), DomainError> {
        bounded_json(input)?;
        if input.as_object().is_none_or(|o| {
            o.keys().any(|k| {
                !["kind", "statement", "confidence", "reason", "sources"].contains(&k.as_str())
            })
        }) {
            return Err(invalid("task entry input fields"));
        }
        let mut entry = normalize_entry(input, true)?;
        if let Some(old) = self.entries().iter().find(|e| {
            e["status"] == "active"
                && e["kind"] == entry["kind"]
                && e["statement"] == entry["statement"]
                && e["sources"] == entry["sources"]
        }) {
            return Ok((self.clone(), old["id"].as_str().unwrap().into()));
        }
        if !id_valid(id) || self.entries().iter().any(|e| e["id"] == id) {
            return Err(invalid("unique task entry id required"));
        }
        entry["id"] = json!(id);
        entry["status"] = json!("active");
        entry["createdAt"] = json!(now);
        entry["updatedAt"] = json!(now);
        let mut out = self.0.clone();
        let mut rows = self.entries().clone();
        rows.push(entry);
        while rows.len() > 50 {
            let index = rows
                .iter()
                .position(|e| !self.linked(&e["id"]))
                .ok_or_else(|| invalid("all entries pinned"))?;
            rows.remove(index);
        }
        out["entries"] = json!(rows);
        out["updatedAt"] = json!(now);
        bounded_json(&out)?;
        Ok((Self(out), id.into()))
    }
    fn linked(&self, id: &Value) -> bool {
        self.0["blockerId"] == *id
            || self.0["checkpointId"] == *id
            || self.0["attemptIds"]
                .as_array()
                .is_some_and(|a| a.contains(id))
    }
    pub fn update(&self, id: &str, patch: &Value, now: &str) -> Result<Self, DomainError> {
        bounded_json(patch)?;
        let index = self
            .entries()
            .iter()
            .position(|e| e["id"] == id)
            .ok_or(DomainError::NotFound)?;
        let mut e = self.entries()[index].clone();
        for (k, v) in patch.as_object().ok_or_else(|| invalid("entry patch"))? {
            if ![
                "kind",
                "statement",
                "status",
                "confidence",
                "reason",
                "sources",
            ]
            .contains(&k.as_str())
            {
                return Err(invalid("unknown entry mutation"));
            }
            e[k] = v.clone();
        }
        if !member(&e["status"], &["active", "resolved", "superseded"]) {
            return Err(invalid("entry status"));
        }
        e = normalize_entry(&e, true)?;
        e["updatedAt"] = json!(now);
        let mut out = self.0.clone();
        out["entries"][index] = e.clone();
        out["updatedAt"] = json!(now);
        for (key, kind) in [("blockerId", "FAILURE"), ("checkpointId", "RESULT")] {
            if out[key] == id && (e["status"] != "active" || e["kind"] != kind) {
                out.as_object_mut().unwrap().remove(key);
                if key == "blockerId" && out["status"] == "blocked" {
                    out["status"] = json!("active");
                }
            }
        }
        bounded_json(&out)?;
        Ok(Self(out))
    }
    pub fn remove(&self, id: &str, now: &str) -> Result<Self, DomainError> {
        if self.linked(&json!(id)) {
            return Err(invalid("clear executive links before removal"));
        }
        if !self.entries().iter().any(|e| e["id"] == id) {
            return Err(DomainError::NotFound);
        }
        let mut out = self.0.clone();
        out["entries"] = json!(
            self.entries()
                .iter()
                .filter(|e| e["id"] != id)
                .collect::<Vec<_>>()
        );
        out["updatedAt"] = json!(now);
        bounded_json(&out)?;
        Ok(Self(out))
    }
    pub fn capture_instruction(
        &self,
        instruction: &str,
        prior_users: &[String],
        now: &str,
    ) -> Result<Self, DomainError> {
        let Some(instruction) = canonical_task(instruction) else {
            return Ok(self.clone());
        };
        if self.0["latestInstruction"] == instruction {
            return Ok(self.clone());
        }
        let mut patch = json!({"latestInstruction":instruction});
        if self.0.get("objective").is_none() {
            patch["objective"] = json!(
                prior_users
                    .iter()
                    .find_map(|s| canonical_task(s))
                    .unwrap_or(instruction)
            );
        }
        self.set(&patch, now)
    }
    pub fn checkpoint_verified(&self, id: &str, journal: &[Value]) -> bool {
        self.entries()
            .iter()
            .find(|e| e["id"] == id && e["kind"] == "RESULT" && e["status"] == "active")
            .is_some_and(|e| passing_check(e["sources"].as_array().unwrap(), journal))
    }
}
pub fn canonical_task(s: &str) -> Option<String> {
    let mut s = s.trim();
    while s.starts_with("<skill ") {
        let end = s.find("</skill>")?;
        s = s[end + 8..].trim();
    }
    let s = text(s, 2000);
    (!s.is_empty()).then_some(s)
}
pub fn passing_check(sources: &[Value], journal: &[Value]) -> bool {
    for entry in journal.iter().rev() {
        if entry["type"] != "message" || entry["message"]["role"] != "toolResult" {
            continue;
        }
        let m = &entry["message"];
        if ["write", "edit", "bash"]
            .iter()
            .any(|s| m["toolName"] == *s)
        {
            return false;
        }
        if m["toolName"] == "check" {
            return m["isError"] != true
                && m["details"]["passed"] == true
                && sources.iter().any(|s| {
                    s == &json!(format!(
                        "tool-call:{}",
                        m["toolCallId"].as_str().unwrap_or("")
                    )) || s
                        == &json!(format!(
                            "session-entry:{}",
                            entry["id"].as_str().unwrap_or("")
                        ))
                });
        }
    }
    false
}
/// Storage owns the transaction and idempotency record; core never acknowledges before commit.
pub trait JournalWritePort: StoragePort {
    fn append_records(
        &self,
        session: &SessionId,
        command: &str,
        expected: u64,
        records: &[Vec<u8>],
    ) -> Result<(u64, u64), DomainError>;
}
pub struct PreparedStateCommit {
    pub session: SessionId,
    pub command: String,
    pub expected: u64,
    pub records: Vec<Vec<u8>>,
}
impl PreparedStateCommit {
    pub fn commit(&self, store: &impl JournalWritePort) -> Result<(u64, u64), DomainError> {
        store.append_records(&self.session, &self.command, self.expected, &self.records)
    }
}
pub fn branch_values(journal: &crate::storage::StoredJournal) -> Result<Vec<Value>, DomainError> {
    let map: BTreeMap<_, _> = journal.entries.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut seen = BTreeSet::new();
    let mut rows = vec![];
    let mut leaf = journal.leaf.as_deref();
    while let Some(id) = leaf {
        if !seen.insert(id) {
            return Err(invalid("journal cycle"));
        }
        let entry = map
            .get(id)
            .ok_or_else(|| invalid("missing branch parent"))?;
        rows.push(serde_json::from_slice(&entry.normalized).map_err(|_| invalid("journal JSON"))?);
        leaf = entry.parent.as_deref();
    }
    rows.reverse();
    Ok(rows)
}
pub struct TaskStateService;
impl TaskStateService {
    pub fn prepare(
        store: &impl StoragePort,
        session: SessionId,
        command: &str,
        entry_id: &str,
        now: &str,
        reduce: impl FnOnce(TaskState) -> Result<TaskState, DomainError>,
    ) -> Result<PreparedStateCommit, DomainError> {
        let journal = store.journal_snapshot(&session)?;
        let rows = branch_values(&journal)?;
        let mut current = TaskState::empty(now);
        for e in rows.iter().rev() {
            if e["type"] == "custom" && e["customType"] == "task_state" {
                current = TaskState::parse(&e["data"])?;
                break;
            }
        }
        let next = reduce(current)?;
        let record = json!({"type":"custom","id":entry_id,"parentId":journal.leaf,"timestamp":now,"customType":"task_state","data":next.value()});
        Ok(PreparedStateCommit {
            session,
            command: command.into(),
            expected: journal.revision,
            records: vec![serde_json::to_vec(&record).map_err(|_| invalid("task serialization"))?],
        })
    }
}
