//! Prospective checklist declarations, separately journalled from task claims.
use crate::{
    DomainError,
    task::{bounded_json, invalid, text},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub fn validate_items(input: &Value) -> Result<Vec<Value>, DomainError> {
    bounded_json(input)?;
    let rows = input.as_array().ok_or_else(|| invalid("todo array"))?;
    if rows.len() > 50 {
        return Err(invalid("todo cap"));
    }
    let mut ids = BTreeSet::new();
    let mut out = vec![];
    let mut active = 0;
    for (i, row) in rows.iter().enumerate() {
        let title = text(
            row["title"].as_str().ok_or_else(|| invalid("todo title"))?,
            usize::MAX,
        );
        if title.is_empty() {
            return Err(invalid("empty todo title"));
        }
        let status = if row["status"].is_null() {
            "pending"
        } else {
            row["status"]
                .as_str()
                .ok_or_else(|| invalid("todo status"))?
        };
        if !["pending", "in_progress", "completed"].contains(&status) {
            return Err(invalid("todo status"));
        }
        active += usize::from(status == "in_progress");
        let explicit = row["id"].as_str().unwrap_or("").trim();
        let id = if !explicit.is_empty() && !ids.contains(explicit) {
            explicit.into()
        } else {
            let mut n = i + 1;
            while ids.contains(&format!("t{n}")) {
                n += 1;
            }
            format!("t{n}")
        };
        ids.insert(id.clone());
        out.push(json!({"id":id,"title":title,"status":status}));
    }
    if active > 1 {
        return Err(invalid("only one active todo"));
    }
    Ok(out)
}
pub fn validate_lifecycle(current: &[Value], next: &[Value]) -> Result<(), DomainError> {
    if next.iter().any(|e| e["status"] != "completed")
        && next.iter().filter(|e| e["status"] == "in_progress").count() != 1
    {
        return Err(invalid("open plan needs one active step"));
    }
    if current.iter().all(|e| e["status"] == "completed") {
        if next.iter().any(|e| e["status"] == "completed") {
            return Err(invalid("new plans must be prospective"));
        }
        return Ok(());
    }
    for old in current {
        let new = next
            .iter()
            .find(|e| e["id"] == old["id"])
            .ok_or_else(|| invalid(&format!("Active todo {} disappeared. Query todo with no arguments to recover current IDs; retain all items, or explicitly archive and start a new plan.", old["id"])))?;
        if new["title"] != old["title"]
            || old["status"] == "completed" && new["status"] != "completed"
        {
            return Err(invalid(&format!(
                "Todo {} must retain its title and completed status. Query todo with no arguments and update only the intended statuses; use an explicit new-plan for new work.",
                old["id"]
            )));
        }
    }
    if next
        .iter()
        .any(|e| e["status"] == "completed" && !current.iter().any(|old| old["id"] == e["id"]))
    {
        return Err(invalid("new work cannot be retrospectively completed"));
    }
    Ok(())
}
pub fn replace(
    current: Option<&Value>,
    items: &Value,
    now: i64,
    plan_id: Option<&str>,
    phase: Option<&str>,
) -> Result<Value, DomainError> {
    let next = validate_items(items)?;
    let empty = vec![];
    let old = current
        .and_then(|c| c["items"].as_array())
        .unwrap_or(&empty);
    validate_lifecycle(old, &next)?;
    let mut out = json!({"version":1,"items":next,"updatedAt":now});
    for (key, new) in [("planId", plan_id), ("phase", phase)] {
        if let Some(v) = new.or_else(|| current.and_then(|c| c[key].as_str())) {
            if v.len() > 1024 {
                return Err(invalid("plan metadata cap"));
            }
            out[key] = json!(v);
        }
    }
    Ok(out)
}
pub fn archive(current: &Value, now: i64, new_plan: bool) -> Result<Value, DomainError> {
    validate_items(&current["items"])?;
    let mut out = json!({"version":1,"snapshot":current,"archivedAt":now,"reason":if new_plan{"new-plan"}else{"archive"}});
    for key in ["planId", "phase"] {
        if let Some(v) = current.get(key) {
            out[key] = v.clone();
        }
    }
    Ok(out)
}
pub fn calls_since_update(journal: &[Value]) -> usize {
    let Some(index) = journal.iter().rposition(|e| {
        e["type"] == "custom" && e["customType"] == "todo_list"
            || e["type"] == "message" && e["message"]["role"] == "user"
    }) else {
        return 0;
    };
    journal[index + 1..]
        .iter()
        .filter(|e| {
            e["type"] == "message"
                && e["message"]["role"] == "toolResult"
                && e["message"]["toolName"] != "todo"
                && e["message"]["isError"] != true
        })
        .count()
}

pub struct TodoStateService;
impl TodoStateService {
    pub fn prepare(
        store: &impl crate::storage::StoragePort,
        session: crate::SessionId,
        command: &str,
        entry_id: &str,
        now: i64,
        items: &Value,
    ) -> Result<crate::task::PreparedStateCommit, DomainError> {
        let journal = store.journal_snapshot(&session)?;
        let branch = crate::task::branch_values(&journal)?;
        let current = branch
            .iter()
            .rev()
            .find(|e| e["type"] == "custom" && e["customType"] == "todo_list")
            .map(|e| &e["data"]);
        let next = replace(current, items, now, None, None)?;
        let timestamp = chrono::DateTime::from_timestamp_millis(now)
            .ok_or_else(|| invalid("todo timestamp"))?
            .to_rfc3339();
        let record = json!({"type":"custom","id":entry_id,"parentId":journal.leaf,"timestamp":timestamp,"customType":"todo_list","data":next});
        Ok(crate::task::PreparedStateCommit {
            session,
            command: command.into(),
            expected: journal.revision,
            records: vec![serde_json::to_vec(&record).map_err(|_| invalid("todo serialization"))?],
        })
    }
}
