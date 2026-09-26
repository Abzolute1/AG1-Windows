//! Host-owned before/after custody. A prepared receipt alone never proves an
//! effect happened, and neither result hooks nor assistant text can create one.
use crate::workspace::{MAX_READ, Workspace};
use ag1_core::{
    DomainError, FileRevision,
    storage::StoragePort,
    tools::registry::{Scope, ValidatedCall},
};
use ag1_storage::{ArtifactInput, Store, digest};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Revision {
    pub sha256: String,
    pub device: u64,
    pub inode: u64,
    pub size: u64,
    pub modified_ns: String,
    pub changed_ns: String,
    pub mode: u32,
}
impl From<&FileRevision> for Revision {
    fn from(value: &FileRevision) -> Self {
        Self {
            sha256: value.sha256.clone(),
            device: value.device,
            inode: value.inode,
            size: value.size,
            modified_ns: value.modified_ns.to_string(),
            changed_ns: value.changed_ns.to_string(),
            mode: value.mode,
        }
    }
}
impl TryFrom<&Revision> for FileRevision {
    type Error = DomainError;
    fn try_from(value: &Revision) -> Result<Self, DomainError> {
        Ok(Self {
            sha256: value.sha256.clone(),
            device: value.device,
            inode: value.inode,
            size: value.size,
            modified_ns: value
                .modified_ns
                .parse()
                .map_err(|_| DomainError::Conflict)?,
            changed_ns: value
                .changed_ns
                .parse()
                .map_err(|_| DomainError::Conflict)?,
            mode: value.mode,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub artifact: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeReceipt {
    pub version: u32,
    pub id: String,
    pub session: String,
    pub workspace: String,
    pub root: String,
    pub path: String,
    pub call_id: String,
    /// Immutable assistant journal entry observed at the native effect boundary.
    /// Older/unbound receipts remain diagnostic and never gain turn authority.
    #[serde(default)]
    pub call_entry: Option<String>,
    pub operation: String,
    pub tool: String,
    pub created_ms: u64,
    pub state: String,
    pub before: Option<Snapshot>,
    pub after: Snapshot,
    pub before_revision: Option<Revision>,
    pub after_revision: Option<Revision>,
    pub summary: Option<ag1_core::tools::diff::DiffSummary>,
}
pub struct PendingChange {
    pub record: ChangeReceipt,
    sha256: String,
}
fn json_error(_: serde_json::Error) -> DomainError {
    DomainError::InvalidInput("file change receipt JSON".into())
}

fn retain(
    store: &Store,
    session: &str,
    id: &str,
    side: &str,
    bytes: &[u8],
    now: u64,
) -> Result<Snapshot, DomainError> {
    // Binary-safe original bytes use an explicitly labelled base64 custody
    // envelope, not a pretend PNG or lossy UTF-8 conversion.
    let artifact = format!("art_{}", &digest(format!("{id}/{side}").as_bytes())[..32]);
    let content = STANDARD.encode(bytes).into_bytes();
    let record = json!({"version":1,"id":artifact,"sessionId":session,"kind":"native-change-bytes",
        "producer":{"type":"system","name":"native-file-effects"},"encoding":"utf8","mimeType":"text/plain",
        "sha256":digest(&content),"bytes":content.len(),"createdAt":chrono::DateTime::from_timestamp_millis(i64::try_from(now).map_err(|_|DomainError::Conflict)?).ok_or(DomainError::Conflict)?.to_rfc3339()});
    store.put_artifact(&ArtifactInput {
        origin: session.into(),
        session: session.into(),
        record: serde_json::to_vec(&record).map_err(json_error)?,
        bytes: content,
    })?;
    Ok(Snapshot {
        artifact,
        sha256: digest(bytes),
        bytes: bytes.len(),
    })
}

impl PendingChange {
    pub fn prepare(
        store: &Store,
        workspace: &Workspace,
        scope: &Scope,
        call: &ValidatedCall,
        path: &str,
        expected: Option<&FileRevision>,
        after: &[u8],
        now: u64,
    ) -> Result<Option<Self>, DomainError> {
        let before = match workspace.snapshot(path, MAX_READ) {
            Ok(snapshot) => Some(snapshot),
            Err(DomainError::NotFound) => None,
            Err(error) => return Err(error),
        };
        if before.as_ref().map(|s| &s.revision) != expected {
            return Err(DomainError::Conflict);
        }
        if before.as_ref().is_some_and(|s| s.bytes == after) {
            return Ok(None);
        }
        let id = format!(
            "change-{}",
            digest(
                json!([
                    scope.workspace,
                    scope.session,
                    call.operation(),
                    call.call_id()
                ])
                .to_string()
                .as_bytes()
            )
        );
        // Never replay a prepared/committed effect after lost acknowledgement.
        if store.load_metadata("native-file-change", &id)?.is_some() {
            return Err(DomainError::DurabilityUnknown);
        }
        let summary = if before.as_ref().map_or(0, |s| s.bytes.len()) + after.len() <= 256 * 1024 {
            std::str::from_utf8(before.as_ref().map_or(&[], |s| s.bytes.as_slice()))
                .ok()
                .zip(std::str::from_utf8(after).ok())
                .filter(|(a, b)| !a.contains('\0') && !b.contains('\0'))
                .and_then(|(a, b)| ag1_core::tools::diff::generate(a, b, 3).ok())
                .map(|d| d.summary)
        } else {
            None
        };
        let before_ref = before
            .as_ref()
            .map(|s| retain(store, &scope.session, &id, "before", &s.bytes, now))
            .transpose()?;
        let after_ref = retain(store, &scope.session, &id, "after", after, now)?;
        let record = ChangeReceipt {
            version: 1,
            id,
            session: scope.session.clone(),
            workspace: scope.workspace.clone(),
            root: workspace.root().to_string_lossy().into_owned(),
            path: path.into(),
            call_id: call.call_id().into(),
            call_entry: {
                let branch = store.branch(&scope.session, None)?;
                let records = branch
                    .iter()
                    .map(|entry| {
                        serde_json::from_slice::<Value>(&entry.normalized).map_err(json_error)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                unique_call_entry(&records, call.call_id(), call.name()).map(str::to_owned)
            },
            operation: call.operation().into(),
            tool: call.name().into(),
            created_ms: now,
            state: "prepared".into(),
            before: before_ref,
            after: after_ref,
            before_revision: expected.map(Revision::from),
            after_revision: None,
            summary,
        };
        let sha256 = store.commit_metadata(
            "native-file-change",
            &record.id,
            None,
            &serde_json::to_vec(&record).map_err(json_error)?,
        )?;
        Ok(Some(Self { record, sha256 }))
    }
    pub fn settle(
        mut self,
        store: &Store,
        published: &Result<FileRevision, DomainError>,
    ) -> Result<(), DomainError> {
        match published {
            Ok(revision) => {
                self.record.state = "committed".into();
                self.record.after_revision = Some(revision.into());
            }
            Err(DomainError::DurabilityUnknown) => self.record.state = "uncertain".into(),
            Err(_) => self.record.state = "not_committed".into(),
        }
        store.commit_metadata(
            "native-file-change",
            &self.record.id,
            Some(&self.sha256),
            &serde_json::to_vec(&self.record).map_err(json_error)?,
        )?;
        Ok(())
    }
}

pub fn read_snapshot(
    store: &Store,
    session: &str,
    snapshot: &Snapshot,
) -> Result<Vec<u8>, DomainError> {
    if snapshot.bytes > MAX_READ {
        return Err(DomainError::LimitExceeded("file change snapshot".into()));
    }
    let mut encoded = Vec::new();
    let mut offset = 0;
    loop {
        let page = store.page(session, session, &snapshot.artifact, offset, 32768)?;
        encoded.extend(page.bytes);
        if encoded.len() > MAX_READ.div_ceil(3) * 4 {
            return Err(DomainError::LimitExceeded("file change snapshot".into()));
        }
        match page.next_offset {
            Some(next) if next > offset => offset = next,
            None => break,
            _ => return Err(DomainError::Conflict),
        }
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| DomainError::Conflict)?;
    if bytes.len() != snapshot.bytes || digest(&bytes) != snapshot.sha256 {
        return Err(DomainError::Conflict);
    }
    Ok(bytes)
}

pub fn summary(value: &Value) -> Result<Value, DomainError> {
    let record: ChangeReceipt = serde_json::from_value(value.clone()).map_err(json_error)?;
    Ok(
        json!({"id":record.id,"path":record.path,"root":record.root,"call_id":record.call_id,"tool":record.tool,
        "state":record.state,"created_ms":record.created_ms,"operation":if record.before.is_some(){"edit"}else{"create"},
        "before_sha256":record.before.as_ref().map(|s| &s.sha256),"after_sha256":record.after.sha256,"summary":record.summary}),
    )
}

fn has_call(entry: &Value, call: &str, tool: &str) -> bool {
    entry["type"] == "message"
        && entry["message"]["role"] == "assistant"
        && entry["message"]["content"]
            .as_array()
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|b| b["type"] == "toolCall" && b["id"] == call && b["name"] == tool)
            })
}

pub fn unique_call_entry<'a>(records: &'a [Value], call: &str, tool: &str) -> Option<&'a str> {
    let mut matches = records.iter().filter(|r| has_call(r, call, tool));
    let entry = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    // Duplicate IDs within a single message are ambiguous too.
    if entry["message"]["content"]
        .as_array()?
        .iter()
        .filter(|b| b["type"] == "toolCall" && b["id"] == call)
        .count()
        != 1
    {
        return None;
    }
    entry["id"].as_str()
}

/// Resolve a native receipt against this exact active branch. Neither tool
/// result details nor a caller-supplied list of call IDs participates in trust.
pub fn verify_branch(
    record: &ChangeReceipt,
    records: &[Value],
    session: &str,
    workspace: &str,
) -> Result<(), DomainError> {
    if record.session != session
        || record.workspace != workspace
        || record.call_entry.is_none()
        || record.call_entry.as_deref() != unique_call_entry(records, &record.call_id, &record.tool)
    {
        return Err(DomainError::PermissionDenied);
    }
    Ok(())
}

pub fn turn_calls(records: &[Value], turn: &str) -> Result<Vec<String>, DomainError> {
    let start = records
        .iter()
        .position(|r| r["id"] == turn && r["type"] == "message" && r["message"]["role"] == "user")
        .ok_or(DomainError::NotFound)?;
    let mut calls = Vec::new();
    for entry in records
        .iter()
        .skip(start + 1)
        .take_while(|r| !(r["type"] == "message" && r["message"]["role"] == "user"))
    {
        if entry["message"]["role"] != "assistant" {
            continue;
        }
        for block in entry["message"]["content"].as_array().into_iter().flatten() {
            if block["type"] == "toolCall" {
                if let Some(call) = block["id"].as_str().filter(|id| id.len() <= 256) {
                    calls.push(call.to_owned());
                }
            }
        }
    }
    Ok(calls)
}
