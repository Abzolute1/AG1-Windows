//! Source-compatible resolved labels with a corrected continuous branch. The
//! reference drops labels without repairing message parents, losing history.
use crate::{
    JournalEntry, JournalSnapshot, StorageError, StorageResult, Store, digest, journal::entry,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn records(
    snapshot: &JournalSnapshot,
    leaf: Option<&str>,
    command: &str,
) -> StorageResult<Vec<JournalEntry>> {
    let Some(leaf) = leaf else {
        return Ok(vec![]);
    };
    if snapshot.entries.len() > 100000
        || snapshot
            .entries
            .iter()
            .try_fold(0usize, |n, row| n.checked_add(row.normalized.len()))
            .is_none_or(|n| n > 128 * 1024 * 1024)
    {
        return Err(StorageError::Limit("Runtime fork source".into()));
    }
    let branch = Store::branch_snapshot(snapshot, Some(leaf))?;
    let mut result = Vec::<JournalEntry>::new();
    for mut row in branch {
        let mut value: Value = serde_json::from_slice(&row.normalized)?;
        if value["type"] == "label" {
            continue;
        }
        let parent = result.last().map(|row| row.id.clone());
        let ordinal = result.len() as u64 + 1;
        if row.parent != parent {
            value["parentId"] = json!(parent);
            row = entry(&serde_json::to_vec(&value)?, ordinal)?;
        } else {
            row.ordinal = ordinal;
        }
        result.push(row);
    }
    // JavaScript Map keeps insertion order on update; deletion and re-addition
    // move the target to the end. Labels resolve across all physical branches.
    let mut positions = BTreeMap::<String, usize>::new();
    let mut labels = Vec::<Option<Value>>::new();
    for row in &snapshot.entries {
        let value: Value = serde_json::from_slice(&row.normalized)?;
        if value["type"] != "label" {
            continue;
        }
        let target = value["targetId"]
            .as_str()
            .ok_or_else(|| StorageError::Corrupt("Fork label target".into()))?
            .to_owned();
        let label = match &value["label"] {
            Value::Null => "",
            Value::String(text) => text.as_str(),
            _ => return Err(StorageError::Corrupt("Fork label value".into())),
        };
        if label.is_empty() {
            if let Some(index) = positions.remove(&target) {
                labels[index] = None;
            }
        } else if let Some(index) = positions.get(&target) {
            labels[*index] = Some(value);
        } else {
            positions.insert(target, labels.len());
            labels.push(Some(value));
        }
    }
    let targets = result.iter().map(|e| e.id.clone()).collect::<BTreeSet<_>>();
    let mut occupied = snapshot
        .entries
        .iter()
        .map(|e| e.id.clone())
        .collect::<BTreeSet<_>>();
    let prefix = digest(command.as_bytes());
    let mut counter = 0u64;
    for label in labels.into_iter().flatten() {
        if !targets.contains(label["targetId"].as_str().unwrap()) {
            continue;
        }
        let id = loop {
            counter += 1;
            let id = format!("fork-label-{prefix}-{counter}");
            if occupied.insert(id.clone()) {
                break id;
            }
        };
        let value = json!({"type":"label","id":id,"parentId":result.last().map(|e|&e.id),
            "timestamp":label["timestamp"],"targetId":label["targetId"],"label":label["label"]});
        result.push(entry(
            &serde_json::to_vec(&value)?,
            result.len() as u64 + 1,
        )?);
    }
    if result.len() > 100000
        || result
            .iter()
            .try_fold(0usize, |n, row| n.checked_add(row.raw.len()))
            .is_none_or(|n| n > 128 * 1024 * 1024)
    {
        return Err(StorageError::Limit("Runtime fork branch".into()));
    }
    Ok(result)
}
