//! Branch-local episode indexes are model-authored pointers, never verification evidence.
use super::{ContextArtifactPort, string};
use crate::{
    DomainError, SessionId,
    evidence::{Freshness, SourceReference},
    storage::StoragePort,
    task::{PreparedStateCommit, branch_values, invalid, text, units},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub fn valid_id(value: &str, prefix: &str, digits: usize) -> bool {
    value.strip_prefix(prefix).is_some_and(|v| {
        v.len() == digits
            && v.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
fn timestamp(v: &Value) -> Result<&str, DomainError> {
    let s = v.as_str().ok_or_else(|| invalid("episode timestamp"))?;
    chrono::DateTime::parse_from_rfc3339(s).map_err(|_| invalid("episode RFC3339 timestamp"))?;
    Ok(s)
}
fn normalized(v: &Value, cap: usize) -> Result<String, DomainError> {
    let s = text(v.as_str().ok_or_else(|| invalid("episode text"))?, cap);
    if s.is_empty() {
        return Err(invalid("empty episode text"));
    }
    Ok(s)
}
fn sources(v: &Value, cap: usize) -> Result<Vec<String>, DomainError> {
    let rows = v.as_array().ok_or_else(|| invalid("episode sources"))?;
    if rows.len() > cap {
        return Err(invalid("episode source limit"));
    }
    let mut result = vec![];
    for item in rows {
        let s = item
            .as_str()
            .ok_or_else(|| invalid("episode source"))?
            .trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if s.is_empty() || units(s) > 500 || s.contains(['\r', '\n', '\0']) {
            return Err(invalid("episode source"));
        }
        if !result.iter().any(|old| old == s) {
            result.push(s.to_string());
        }
    }
    Ok(result)
}
pub fn parse_claim(v: &Value) -> Result<Value, DomainError> {
    let kind = string(v, "kind");
    if ![
        "FACT",
        "OBSERVATION",
        "HYPOTHESIS",
        "DECISION",
        "CONSTRAINT",
        "ASSUMPTION",
        "RESULT",
        "FAILURE",
    ]
    .contains(&kind)
    {
        return Err(invalid("episode claim kind"));
    }
    let refs = sources(&v["sources"], 12)?;
    if ["FACT", "OBSERVATION", "CONSTRAINT", "RESULT", "FAILURE"].contains(&kind) && refs.is_empty()
    {
        return Err(invalid("factual episode claim requires sources"));
    }
    let mut claim =
        json!({"kind":kind,"statement":normalized(&v["statement"],1000)?,"sources":refs});
    if let Some(confidence) = v.get("confidence") {
        if !["low", "medium", "high"].iter().any(|s| confidence == s) {
            return Err(invalid("episode confidence"));
        }
        claim["confidence"] = confidence.clone();
    }
    Ok(claim)
}
fn claims(v: &Value) -> Result<Vec<Value>, DomainError> {
    let rows = v.as_array().ok_or_else(|| invalid("episode claims"))?;
    if rows.len() > 20 {
        return Err(invalid("episode claim cap"));
    }
    rows.iter().map(parse_claim).collect()
}
fn file_revisions(v: &Value) -> Result<Vec<String>, DomainError> {
    let refs = sources(v, 20)?;
    for source in &refs {
        SourceReference::parse(source)?;
    }
    Ok(refs)
}
#[derive(Clone, Debug)]
pub struct EpisodeRecord(Value);
impl EpisodeRecord {
    pub fn value(&self) -> &Value {
        &self.0
    }
    pub fn parse(v: &Value) -> Result<Self, DomainError> {
        if v["version"] != 1 || !valid_id(string(v, "id"), "ep_", 12) {
            return Err(invalid("episode version/id"));
        }
        let mut out = json!({"version":1,"event":v["event"],"id":v["id"],"goal":normalized(&v["goal"],2000)?});
        if let Ok(title) = normalized(&v["title"], 200) {
            out["title"] = title.into();
        }
        match string(v, "event") {
            "begin" => out["createdAt"] = timestamp(&v["createdAt"])?.into(),
            "closed" => {
                if v["synopsisAuthority"] != "model-authored-index"
                    || !valid_id(string(v, "trajectoryArtifactId"), "art_", 32)
                    || v["entryCount"]
                        .as_u64()
                        .is_none_or(|n| n == 0 || n > 9_007_199_254_740_991)
                    || !["completed", "partial", "failed", "abandoned"]
                        .contains(&string(v, "outcome"))
                    || string(v, "startEntryId").is_empty()
                    || string(v, "endEntryId").is_empty()
                {
                    return Err(invalid("episode close provenance"));
                }
                for key in ["openedAt", "closedAt"] {
                    out[key] = timestamp(&v[key])?.into();
                }
                for key in [
                    "startEntryId",
                    "endEntryId",
                    "entryCount",
                    "outcome",
                    "synopsisAuthority",
                    "trajectoryArtifactId",
                ] {
                    out[key] = v[key].clone();
                }
                out["synopsis"] = normalized(&v["synopsis"], 4000)?.into();
                out["claims"] = claims(&v["claims"])?.into();
                out["fileRevisions"] = file_revisions(&v["fileRevisions"])?.into();
                out["verificationSources"] = sources(&v["verificationSources"], 12)?.into();
            }
            _ => return Err(invalid("episode event")),
        }
        Ok(Self(out))
    }
}
pub fn entry_record(entry: &Value) -> Option<EpisodeRecord> {
    if entry["type"] != "custom" || entry["customType"] != "context_episode" {
        return None;
    }
    EpisodeRecord::parse(&entry["data"]).ok()
}
pub fn find_open(entries: &[Value]) -> Option<(EpisodeRecord, usize)> {
    let mut closed = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate().rev() {
        let Some(record) = entry_record(entry) else {
            continue;
        };
        if record.0["event"] == "closed" {
            closed.insert(string(&record.0, "id").to_string());
        } else if !closed.contains(string(&record.0, "id")) {
            return Some((record, index));
        }
    }
    None
}
pub fn list_closed(entries: &[Value], limit: usize) -> Vec<EpisodeRecord> {
    closed_records(entries)
        .into_iter()
        .take(limit.clamp(1, 100))
        .collect()
}
pub(super) fn closed_records(entries: &[Value]) -> Vec<EpisodeRecord> {
    let mut seen = BTreeSet::new();
    entries
        .iter()
        .rev()
        .filter_map(entry_record)
        .filter(|r| r.0["event"] == "closed" && seen.insert(string(&r.0, "id").to_string()))
        .collect()
}
pub fn health(
    record: &EpisodeRecord,
    store: &impl ContextArtifactPort,
    mut validate: impl FnMut(&str) -> Freshness,
) -> Value {
    let r = record.value();
    let available = store.get(string(r, "trajectoryArtifactId")).is_ok();
    let mut refs = vec![];
    for field in ["fileRevisions", "verificationSources"] {
        refs.extend(
            r[field]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    for claim in r["claims"].as_array().into_iter().flatten() {
        refs.extend(
            claim["sources"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    }
    let mut seen = BTreeSet::new();
    let rows: Vec<_> = refs
        .into_iter()
        .filter(|s| seen.insert(*s))
        .map(|source| {
            let id = source.strip_prefix("artifact:").unwrap_or(source);
            let status = if source.starts_with("file:") {
                if SourceReference::parse(source).is_ok() {
                    validate(source)
                } else {
                    Freshness::Invalid
                }
            } else if valid_id(id, "art_", 32) {
                if store.get(id).is_ok() {
                    Freshness::Current
                } else {
                    Freshness::Missing
                }
            } else {
                Freshness::Unverified
            };
            json!({"source":source,"status":status})
        })
        .collect();
    let status = if !available {
        "invalid"
    } else {
        ["invalid", "missing", "stale"]
            .into_iter()
            .find(|s| rows.iter().any(|r| r["status"] == *s))
            .unwrap_or(
                if !rows.is_empty() && rows.iter().all(|r| r["status"] == "current") {
                    "current"
                } else {
                    "unverified"
                },
            )
    };
    json!({"status":status,"trajectoryArtifact":if available {"available"} else {"missing"},"sources":rows})
}
pub fn parse_candidate(
    raw: &str,
    allowed: &BTreeSet<String>,
    store: &impl ContextArtifactPort,
) -> Result<Value, DomainError> {
    if raw.len() > 128 * 1024 {
        return Err(invalid("episode candidate size"));
    }
    let trimmed = raw.trim();
    let unfenced = if trimmed.starts_with("```") && trimmed.ends_with("```") {
        let inner = trimmed[3..trimmed.len() - 3].trim_start();
        if inner
            .get(..4)
            .is_some_and(|s| s.eq_ignore_ascii_case("json"))
        {
            &inner[4..]
        } else {
            inner
        }
    } else {
        trimmed
    };
    let v: Value = serde_json::from_str(unfenced).map_err(|_| invalid("episode candidate JSON"))?;
    if v["version"] != 1 || v["authority"] != "model-authored-candidate" {
        return Err(invalid("episode candidate authority"));
    }
    let claims = claims(&v["claims"])?;
    let files = file_revisions(&v["fileRevisions"])?;
    let verification = sources(&v["verificationSources"], 12)?;
    for source in
        files
            .iter()
            .chain(&verification)
            .map(String::as_str)
            .chain(claims.iter().flat_map(|c| {
                c["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
            }))
    {
        if !allowed.contains(source) {
            return Err(invalid("episode candidate invented source"));
        }
    }
    for source in &verification {
        if !valid_id(source, "art_", 32)
            || !["terminal-output", "test-output", "compiler-output"]
                .contains(&store.get(source)?.kind.as_str())
        {
            return Err(invalid("episode candidate mislabeled verification"));
        }
    }
    Ok(
        json!({"version":1,"authority":"model-authored-candidate","synopsis":normalized(&v["synopsis"],4000)?,"claims":claims,"fileRevisions":files,"verificationSources":verification}),
    )
}
pub struct EpisodeCommand<'a> {
    pub session: SessionId,
    pub command: &'a str,
    pub entry_id: &'a str,
    pub now: &'a str,
}
fn prepared(
    journal: &crate::storage::StoredJournal,
    command: EpisodeCommand<'_>,
    record: EpisodeRecord,
) -> Result<PreparedStateCommit, DomainError> {
    let entry = json!({"type":"custom","customType":"context_episode","id":command.entry_id,"parentId":journal.leaf,"timestamp":command.now,"data":record.0});
    Ok(PreparedStateCommit {
        session: command.session,
        command: command.command.into(),
        expected: journal.revision,
        records: vec![serde_json::to_vec(&entry).map_err(|_| invalid("episode JSON"))?],
    })
}
pub fn prepare_begin(
    store: &impl StoragePort,
    command: EpisodeCommand<'_>,
    id: &str,
    goal: &str,
    title: Option<&str>,
) -> Result<PreparedStateCommit, DomainError> {
    let journal = store.journal_snapshot(&command.session)?;
    let record = begin_record(&branch_values(&journal)?, command.now, id, goal, title)?;
    prepared(&journal, command, record)
}
/// Prepare metadata without acquiring a competing session writer.
pub fn begin_record(
    entries: &[Value],
    now: &str,
    id: &str,
    goal: &str,
    title: Option<&str>,
) -> Result<EpisodeRecord, DomainError> {
    if find_open(entries).is_some() {
        return Err(invalid("episode already open"));
    }
    EpisodeRecord::parse(
        &json!({"version":1,"event":"begin","id":id,"goal":goal,"title":title,"createdAt":now}),
    )
}
pub fn prepare_close(
    store: &impl StoragePort,
    artifacts: &mut impl ContextArtifactPort,
    command: EpisodeCommand<'_>,
    input: &Value,
) -> Result<PreparedStateCommit, DomainError> {
    let journal = store.journal_snapshot(&command.session)?;
    let record = close_record(&journal, artifacts, command.now, command.command, input)?;
    prepared(&journal, command, record)
}
/// Capture exact durable bytes; the caller still owns metadata publication.
pub fn close_record(
    journal: &crate::storage::StoredJournal,
    artifacts: &mut impl ContextArtifactPort,
    now: &str,
    call_id: &str,
    input: &Value,
) -> Result<EpisodeRecord, DomainError> {
    let entries = branch_values(journal)?;
    let (open, start) = find_open(&entries).ok_or_else(|| invalid("no open episode"))?;
    let selected = &entries[start..];
    let start_id = string(&entries[start], "id");
    let mut record = json!({"version":1,"event":"closed","id":open.0["id"],"goal":open.0["goal"],"title":open.0["title"],"openedAt":open.0["createdAt"],"closedAt":now,"startEntryId":start_id,"endEntryId":selected.last().unwrap()["id"],"entryCount":selected.len(),"outcome":input["outcome"],"synopsis":input["synopsis"],"synopsisAuthority":"model-authored-index","trajectoryArtifactId":"art_00000000000000000000000000000000","claims":input.get("claims").cloned().unwrap_or(json!([])),"fileRevisions":input.get("fileRevisions").cloned().unwrap_or(json!([])),"verificationSources":input.get("verificationSources").cloned().unwrap_or(json!([]))});
    EpisodeRecord::parse(&record)?;
    let mut trajectory = format!(
        "{{\"type\":\"ag1_episode_trajectory\",\"version\":1,\"episodeId\":{},\"startEntryId\":{},\"entryCount\":{}}}\n",
        open.0["id"],
        entries[start]["id"],
        selected.len()
    );
    // Retain the original physical bytes, including unknown fields, instead of reserializing projections.
    let raw: std::collections::BTreeMap<_, _> = journal
        .entries
        .iter()
        .map(|e| (e.id.as_str(), &e.raw))
        .collect();
    for entry in selected {
        let bytes = raw
            .get(string(entry, "id"))
            .ok_or_else(|| invalid("episode original entry missing"))?;
        trajectory
            .push_str(std::str::from_utf8(bytes).map_err(|_| invalid("episode original UTF-8"))?);
        if !trajectory.ends_with('\n') {
            trajectory.push('\n');
        }
        if trajectory.len() > 128 * 1024 * 1024 {
            return Err(invalid("episode trajectory limit"));
        }
    }
    let artifact = artifacts.put_evidence(
        &trajectory,
        "episode-trajectory",
        &json!({"type":"tool","name":"episode","callId":call_id}),
        &[],
        Some(start_id),
    )?;
    record["trajectoryArtifactId"] = artifact.id.into();
    EpisodeRecord::parse(&record)
}
