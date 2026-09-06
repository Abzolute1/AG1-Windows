use super::{
    ContextArtifactPort,
    code::Relation,
    episode::valid_id,
    retrieval::{CapsuleDraft, CapsuleFile},
};
use crate::{
    DomainError, SessionId,
    evidence::SourceReference,
    storage::StoragePort,
    task::{PreparedStateCommit, bounded_json, branch_values, invalid, text, units},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapsuleRecord {
    version: u64,
    pub id: String,
    pub query: String,
    pub created_at: String,
    pub artifact_id: String,
    pub chars: usize,
    pub truncated: bool,
    pub files: Vec<CapsuleFile>,
    pub relations: Vec<Relation>,
}
fn safe_path(path: &str) -> Result<String, DomainError> {
    SourceReference::new(path, &"0".repeat(64)).map(|r| r.path().to_string())
}
impl CapsuleRecord {
    pub fn parse(value: &Value) -> Result<Self, DomainError> {
        bounded_json(value)?;
        let mut r: Self =
            serde_json::from_value(value.clone()).map_err(|_| invalid("capsule record"))?;
        if r.version != 1
            || !valid_id(&r.id, "cap_", 16)
            || !valid_id(&r.artifact_id, "art_", 32)
            || r.chars == 0
            || r.chars as u64 > 9_007_199_254_740_991
            || r.files.is_empty()
            || r.files.len() > 8
        {
            return Err(invalid("capsule bounds"));
        }
        chrono::DateTime::parse_from_rfc3339(&r.created_at)
            .map_err(|_| invalid("capsule timestamp"))?;
        r.query = text(&r.query, 2000);
        if r.query.is_empty() {
            return Err(invalid("capsule query"));
        }
        for f in &mut r.files {
            f.path = safe_path(&f.path)?;
            // A displayed path must name the file whose digest it carries.
            if SourceReference::parse(&f.reference)?.path() != f.path {
                return Err(invalid("capsule path/reference mismatch"));
            }
            if f.ranges.is_empty()
                || f.ranges.iter().any(|v| {
                    v.start == 0 || v.end < v.start || v.end as u64 > 9_007_199_254_740_991
                })
            {
                return Err(invalid("capsule ranges"));
            }
            f.ranges.truncate(8);
            for s in &mut f.symbols {
                s.name = text(&s.name, 200);
                s.kind = text(&s.kind, 80);
                if s.name.is_empty()
                    || s.kind.is_empty()
                    || s.line == 0
                    || s.line as u64 > 9_007_199_254_740_991
                {
                    return Err(invalid("capsule symbol"));
                }
            }
            f.symbols.truncate(24);
            if f.signals.iter().any(|s| {
                ![
                    "query-match",
                    "explicit",
                    "working-tree-change",
                    "recent-access",
                    "symbol-match",
                ]
                .contains(&s.as_str())
            }) {
                return Err(invalid("capsule signals"));
            }
            let mut seen = BTreeSet::new();
            f.signals.retain(|s| seen.insert(s.clone()));
        }
        for relation in &mut r.relations {
            relation.from = safe_path(&relation.from)?;
            relation.to = safe_path(&relation.to)?;
            if !["imports", "references"].contains(&relation.kind.as_str()) {
                return Err(invalid("capsule relation"));
            }
            relation.label = relation
                .label
                .as_ref()
                .map(|s| text(s, 200))
                .filter(|s| !s.is_empty());
        }
        r.relations.truncate(32);
        Ok(r)
    }
}
pub fn list(entries: &[Value], limit: usize) -> Vec<CapsuleRecord> {
    let mut seen = BTreeSet::new();
    entries
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == "context_capsule")
        .filter_map(|e| CapsuleRecord::parse(&e["data"]).ok())
        .filter(|r| seen.insert(r.id.clone()))
        .take(limit.clamp(1, 100))
        .collect()
}
pub fn find(entries: &[Value], id: &str) -> Option<CapsuleRecord> {
    if !valid_id(id, "cap_", 16) {
        return None;
    }
    entries
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == "context_capsule")
        .filter_map(|e| CapsuleRecord::parse(&e["data"]).ok())
        .find(|r| r.id == id)
}
pub struct PreparedCapsule {
    pub record: CapsuleRecord,
    pub commit: Option<PreparedStateCommit>,
}
/// New metadata follows an acknowledged immutable artifact. Existing pointers require byte custody.
pub fn prepare(
    store: &impl StoragePort,
    artifacts: &mut impl ContextArtifactPort,
    command: super::episode::EpisodeCommand<'_>,
    draft: &CapsuleDraft,
) -> Result<PreparedCapsule, DomainError> {
    let journal = store.journal_snapshot(&command.session)?;
    let entries = branch_values(&journal)?;
    let (record, created) =
        prepare_record(&entries, artifacts, command.now, command.command, draft)?;
    if !created {
        return Ok(PreparedCapsule {
            record,
            commit: None,
        });
    }
    let row = json!({"type":"custom","customType":"context_capsule","id":command.entry_id,"parentId":journal.leaf,"timestamp":command.now,"data":record});
    Ok(PreparedCapsule {
        record,
        commit: Some(PreparedStateCommit {
            session: SessionId::new(command.session.as_str())?,
            command: command.command.into(),
            expected: journal.revision,
            records: vec![row.to_string().into_bytes()],
        }),
    })
}

/// Retain immutable slices without writing session metadata. The native session
/// supplies its captured branch and commits a new record through its own owner.
pub fn prepare_record(
    entries: &[Value],
    artifacts: &mut impl ContextArtifactPort,
    now: &str,
    call_id: &str,
    draft: &CapsuleDraft,
) -> Result<(CapsuleRecord, bool), DomainError> {
    let suffix = format!("\ncapsule_id: {}", draft.id);
    let body = draft
        .body
        .strip_suffix(&suffix)
        .ok_or_else(|| invalid("capsule body/id"))?;
    if draft.id
        != format!(
            "cap_{}",
            &format!("{:x}", Sha256::digest(body.as_bytes()))[..16]
        )
        || draft.chars != units(&draft.body)
    {
        return Err(invalid("capsule body hash/count"));
    }
    if let Some(record) = find(entries, &draft.id) {
        let artifact = artifacts.get(&record.artifact_id)?;
        if artifact.kind != "source-capsule"
            || artifact.sha256 != format!("{:x}", Sha256::digest(draft.body.as_bytes()))
        {
            return Err(invalid("capsule artifact mismatch"));
        }
        return Ok((record, false));
    }
    let input = json!({"version":1,"id":draft.id,"query":draft.query,"createdAt":now,"artifactId":"art_00000000000000000000000000000000","chars":draft.chars,"truncated":draft.truncated,"files":draft.files,"relations":draft.relations});
    let mut record = CapsuleRecord::parse(&input)?;
    record.artifact_id = artifacts
        .put_text(&draft.body, "source-capsule", "sourceContext", call_id, &[])?
        .id;
    Ok((record, true))
}
