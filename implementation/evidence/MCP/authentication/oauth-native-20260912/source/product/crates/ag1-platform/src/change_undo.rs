//! Native Undo intent custody. Receipt IDs select evidence, never arbitrary
//! destinations or bytes. A prepared/uncertain reversal is not replayable.
use crate::{
    change_receipts::{ChangeReceipt, read_snapshot, verify_branch},
    workspace::{MAX_READ, UndoStage, Workspace},
};
use ag1_core::{DomainError, FileRevision, storage::StoragePort};
use ag1_storage::Store;
use serde_json::{Value, json};
use std::path::Path;

pub fn execute(
    store: &Store,
    workspace: &Workspace,
    receipt: &ChangeReceipt,
    records: &[Value],
    session: &str,
    owner: &str,
    check: impl Fn(UndoStage) -> Result<(), DomainError>,
) -> Result<Value, DomainError> {
    verify_branch(receipt, records, session, owner)?;
    if receipt.state != "committed" || workspace.root() != Path::new(&receipt.root) {
        return Err(DomainError::PermissionDenied);
    }
    if store.native_file_undo(session, &receipt.id)?.is_some() {
        return Err(DomainError::Unavailable(
            "This Undo was already attempted. Review its retained outcome; it cannot be replayed."
                .into(),
        ));
    }
    let expected: FileRevision = receipt
        .after_revision
        .as_ref()
        .ok_or(DomainError::PermissionDenied)?
        .try_into()?;
    check(UndoStage::Prepare)?;
    if workspace.snapshot(&receipt.path, MAX_READ)?.revision != expected {
        return Err(DomainError::Unavailable("This file changed after the recorded operation. Undo would conflict with later work; review its current contents.".into()));
    }
    let original = receipt
        .before
        .as_ref()
        .map(|s| read_snapshot(store, session, s))
        .transpose()?;
    let mode = receipt.before_revision.as_ref().map_or(0o600, |r| r.mode);
    let backup = Path::new(&receipt.path)
        .parent()
        .unwrap_or(Path::new(""))
        .join(format!(
            ".ag1-undo-{}",
            &ag1_storage::digest(receipt.id.as_bytes())[..24]
        ))
        .to_string_lossy()
        .into_owned();
    let intent = json!({"version":1,"session":session,"workspace":owner,"receipt":receipt.id,"path":receipt.path,"root":receipt.root,"state":"prepared","recovery":backup,"replay_allowed":false});
    let sha = store.commit_metadata(
        "native-file-undo",
        &receipt.id,
        None,
        &serde_json::to_vec(&intent).map_err(|_| DomainError::Conflict)?,
    )?;
    let outcome = workspace.undo_file_checked(
        &receipt.path,
        &expected,
        original.as_deref().map(|b| (b, mode)),
        &backup,
        check,
    );
    let mut result = intent;
    match outcome {
        Ok(outcome) => {
            result["state"] = json!(outcome.state);
            result["recovery"] = json!(outcome.recovery);
            result["detail"] = json!(outcome.detail);
            result["revision"] = json!(
                outcome
                    .revision
                    .as_ref()
                    .map(crate::change_receipts::Revision::from)
            );
        }
        Err(error) => {
            result["state"] = json!("not_applied");
            result["detail"] = json!(format!("{error:?}"));
            result["recovery"] = Value::Null;
        }
    }
    // Failure here leaves an explicitly prepared intent for recovery inspection.
    store
        .commit_metadata(
            "native-file-undo",
            &receipt.id,
            Some(&sha),
            &serde_json::to_vec(&result).map_err(|_| DomainError::DurabilityUnknown)?,
        )
        .map_err(|_| DomainError::DurabilityUnknown)?;
    Ok(result)
}

/// Explicit recovery only puts the displaced postimage back into a currently
/// absent path. It never retries the interrupted Undo or replaces current work.
pub fn recover(
    store: &Store,
    workspace: &Workspace,
    receipt: &ChangeReceipt,
    records: &[Value],
    session: &str,
    owner: &str,
    check: impl Fn(UndoStage) -> Result<(), DomainError>,
) -> Result<Value, DomainError> {
    verify_branch(receipt, records, session, owner)?;
    if workspace.root() != Path::new(&receipt.root) {
        return Err(DomainError::PermissionDenied);
    }
    let mut record = store
        .native_file_undo(session, &receipt.id)?
        .ok_or(DomainError::NotFound)?;
    if !matches!(
        record["state"].as_str(),
        Some("prepared" | "recovery_required")
    ) {
        return Err(DomainError::PermissionDenied);
    }
    let backup = Path::new(&receipt.path)
        .parent()
        .unwrap_or(Path::new(""))
        .join(format!(
            ".ag1-undo-{}",
            &ag1_storage::digest(receipt.id.as_bytes())[..24]
        ))
        .to_string_lossy()
        .into_owned();
    if record["recovery"] != backup
        || record["receipt"] != receipt.id
        || record["root"] != receipt.root
        || record["path"] != receipt.path
    {
        return Err(DomainError::PermissionDenied);
    }
    check(UndoStage::Prepare)?;
    if workspace.exists(&receipt.path)? {
        return Err(DomainError::Unavailable("The destination contains a file. Recovery will not overwrite it; inspect the retained copy.".into()));
    }
    let copy = workspace.snapshot(&backup, MAX_READ)?;
    let expected: FileRevision = receipt
        .after_revision
        .as_ref()
        .ok_or(DomainError::PermissionDenied)?
        .try_into()?;
    let mut moved = copy.revision.clone();
    moved.changed_ns = expected.changed_ns;
    if moved != expected {
        return Err(DomainError::Unavailable(
            "The recovery copy changed. Review it before restoring bytes.".into(),
        ));
    }
    let saved = store
        .load_metadata("native-file-undo", &receipt.id)?
        .ok_or(DomainError::NotFound)?;
    record["state"] = json!("recovering");
    let sha = store.commit_metadata(
        "native-file-undo",
        &receipt.id,
        Some(&saved.sha256),
        &serde_json::to_vec(&record).map_err(|_| DomainError::Conflict)?,
    )?;
    let result = check(UndoStage::BeforeMove)
        .and_then(|_| workspace.rename(&backup, &receipt.path, &copy.revision))
        .and_then(|_| check(UndoStage::AfterPublish));
    match result {
        Ok(()) => {
            record["state"] = json!("restored_postimage");
            record["recovery"] = Value::Null;
            record["detail"] =
                json!("The displaced file was restored. The interrupted Undo was not replayed.");
        }
        Err(error) => {
            record["state"] = json!("recovery_required");
            record["detail"] = json!(format!("{error:?}"));
        }
    }
    store
        .commit_metadata(
            "native-file-undo",
            &receipt.id,
            Some(&sha),
            &serde_json::to_vec(&record).map_err(|_| DomainError::DurabilityUnknown)?,
        )
        .map_err(|_| DomainError::DurabilityUnknown)?;
    Ok(record)
}
