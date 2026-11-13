//! Stateless tool presentation plus the session-owned checklist lifecycle.
use crate::{
    DomainError,
    task::{bounded_json, invalid},
    todo,
};
use serde_json::{Value, json};

pub type TodoPlan = super::JournalToolPlan;
pub fn snapshot(value: &Value) -> Result<Value, DomainError> {
    bounded_json(value)?;
    if value["version"] != 1 || !value["updatedAt"].is_number() {
        return Err(invalid("todo snapshot"));
    }
    let items = todo::validate_items(&value["items"])?;
    let mut result = json!({"version":1,"items":items,"updatedAt":value["updatedAt"]});
    for key in ["planId", "phase"] {
        if let Some(value) = value.get(key) {
            let text = value
                .as_str()
                .ok_or_else(|| invalid("todo metadata"))?
                .trim();
            if text.len() > 1024 || key == "planId" && text.is_empty() {
                return Err(invalid("todo metadata"));
            }
            if !text.is_empty() {
                result[key] = json!(text);
            }
        }
    }
    Ok(result)
}
pub fn latest(branch: &[Value]) -> Option<Value> {
    branch
        .iter()
        .rev()
        .filter(|r| r["type"] == "custom" && r["customType"] == "todo_list")
        .find_map(|r| snapshot(&r["data"]).ok())
}
pub fn prepare(
    branch: &[Value],
    args: &Value,
    now: i64,
    new_plan_id: &str,
) -> Result<TodoPlan, DomainError> {
    bounded_json(args)?;
    let current = latest(branch);
    let empty = json!({"version":1,"items":[],"updatedAt":now});
    let action = args["planAction"].as_str();
    let clear = args["clear"] == true;
    let items = args.get("todos");
    let phase = args["phase"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let archive = matches!(action, Some("archive" | "new-plan"));
    let (next, text, details) = if action == Some("archive") {
        if items.is_some() || clear || args.get("phase").is_some() {
            return Err(invalid("archive cannot combine todos, clear or phase"));
        }
        (
            Some(empty),
            "Current todo plan archived.".into(),
            json!({"todos":null,"planAction":"archive"}),
        )
    } else if action == Some("new-plan") && (clear || items.is_none()) {
        return Err(invalid("new-plan needs prospective todos and cannot clear"));
    } else if clear {
        if items.is_some() {
            return Err(invalid("clear cannot combine todos"));
        }
        if current.as_ref().is_some_and(|c| {
            c["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["status"] != "completed")
        }) {
            return Err(invalid("cannot clear an active todo list"));
        }
        (
            Some(empty),
            "Todo list cleared.".into(),
            json!({"todos":[],"cleared":true}),
        )
    } else if let Some(items) = items {
        let next = todo::replace(
            if archive { None } else { current.as_ref() },
            items,
            now,
            if archive { Some(new_plan_id) } else { None },
            phase,
        )?;
        let normalized = next["items"].as_array().unwrap();
        let completed = normalized
            .iter()
            .filter(|i| i["status"] == "completed")
            .count();
        let mut text = format!(
            "Todo list updated: {completed}/{} completed.",
            normalized.len()
        );
        if let Some(active) = normalized.iter().find(|i| i["status"] == "in_progress") {
            text.push_str(&format!(
                " In progress: {}.",
                active["title"].as_str().unwrap()
            ));
        }
        text.push_str(" The harness stored this list. Keep working on the in_progress item; pass the full list again with updated statuses when you finish a step or the plan changes.");
        let mut details = json!({"todos":normalized});
        if let Some(action) = action {
            details["planAction"] = json!(action);
        }
        if let Some(phase) = phase {
            details["phase"] = json!(phase);
        }
        (Some(next), text, details)
    } else {
        let state = current.as_ref().unwrap_or(&empty);
        (
            None,
            format!(
                "Current todo list (reuse these IDs when updating): {}",
                state["items"]
            ),
            json!({"todos":state["items"],"planId":state["planId"],"phase":state["phase"]}),
        )
    };
    let mut records = vec![];
    if archive
        && let Some(current) = &current
        && !current["items"].as_array().unwrap().is_empty()
    {
        records.push(json!({"type":"custom","customType":"todo_archive","data":todo::archive(current, now, action == Some("new-plan"))?}));
    }
    if let Some(next) = next {
        records.push(json!({"type":"custom","customType":"todo_list","data":next}));
    }
    Ok(TodoPlan {
        records,
        result: json!({"content":[{"type":"text","text":text}],"details":details}),
    })
}
