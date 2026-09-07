//! Pure task tool reduction. Publication belongs to the active session owner.
use crate::{DomainError, context::projections, task::TaskState};
use serde_json::{Value, json};
pub struct TaskPlan {
    pub next: Option<TaskState>,
    pub result: Value,
}
fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, DomainError> {
    args[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DomainError::InvalidInput(format!("taskState requires {key}")))
}
pub fn prepare(
    branch: &[Value],
    args: &Value,
    now: &str,
    new_id: &str,
) -> Result<TaskPlan, DomainError> {
    let current = branch
        .iter()
        .rev()
        .find(|entry| entry["type"] == "custom" && entry["customType"] == "task_state")
        .map(|entry| TaskState::parse(&entry["data"]))
        .transpose()?;
    let action = required(args, "action")?;
    let result = |text: String, snapshot: &TaskState, entry: Option<&str>| {
        let mut details = json!({"action":action,"snapshot":snapshot.value()});
        if let Some(id) = entry {
            details["entryId"] = json!(id);
        }
        json!({"content":[{"type":"text","text":text}],"details":details})
    };
    if action == "show" {
        let text = if let Some(current) = &current {
            serde_json::to_string_pretty(current.value())
                .map_err(|_| DomainError::InvalidInput("task JSON".into()))?
        } else {
            "No task state recorded.".into()
        };
        return Ok(TaskPlan {
            next: None,
            result: result(
                text,
                &current.unwrap_or_else(|| TaskState::empty(now)),
                None,
            ),
        });
    }
    let mut patch = json!({});
    for key in [
        "objective",
        "currentPhase",
        "currentSubgoal",
        "nextAction",
        "blockerId",
        "checkpointId",
        "attemptIds",
        "constraints",
        "openQuestions",
    ] {
        if let Some(value) = args.get(key) {
            patch[key] = value.clone();
        }
    }
    if let Some(value) = args.get("taskStatus") {
        patch["status"] = value.clone();
    }
    let format = |snapshot: &TaskState| {
        projections::task(snapshot, |_| None, |_| false).unwrap_or_else(|| {
            "No active task state. Use taskState reset with an objective.".into()
        })
    };
    let (next, text, entry) = if action == "reset"
        || action == "set"
            && current.is_none()
            && args["objective"].as_str().is_some_and(|s| !s.is_empty())
    {
        let next = TaskState::reset(required(args, "objective")?, now)?.set(&patch, now)?;
        let text = format!(
            "Task state {}.\n{}",
            if action == "reset" {
                "reset"
            } else {
                "initialized"
            },
            format(&next)
        );
        (next, text, None)
    } else {
        let current = current.as_ref().ok_or_else(|| {
            DomainError::InvalidInput(
                "No task state exists. Use reset first, or set with an objective.".into(),
            )
        })?;
        match action {
            "set" => {
                if patch.as_object().unwrap().is_empty() {
                    return Err(DomainError::InvalidInput(
                        "set requires at least one task-level field".into(),
                    ));
                }
                let next = current.set(&patch, now)?;
                let text = format!("Task state updated.\n{}", format(&next));
                (next, text, None)
            }
            "record" => {
                required(args, "kind")?;
                required(args, "statement")?;
                let mut input = json!({});
                for key in ["kind", "statement", "confidence", "reason", "sources"] {
                    if let Some(value) = args.get(key) {
                        input[key] = value.clone();
                    }
                }
                let (next, id) = current.record(&input, new_id, now)?;
                let kind = next.value()["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["id"] == id)
                    .unwrap()["kind"]
                    .as_str()
                    .unwrap();
                let text = format!("Recorded {id} [{kind}].");
                (next, text, Some(id))
            }
            "remove" => {
                let id = required(args, "id")?;
                (
                    current.remove(id, now)?,
                    format!("Removed {id}."),
                    Some(id.into()),
                )
            }
            "update" => {
                let id = required(args, "id")?;
                let mut input = json!({});
                for key in ["kind", "statement", "confidence", "reason", "sources"] {
                    if let Some(value) = args.get(key) {
                        input[key] = value.clone();
                    }
                }
                if let Some(value) = args.get("entryStatus") {
                    input["status"] = value.clone();
                }
                (
                    current.update(id, &input, now)?,
                    format!("Updated {id}."),
                    Some(id.into()),
                )
            }
            _ => return Err(DomainError::InvalidInput("taskState action".into())),
        }
    };
    if let Some(checkpoint) = next.value()["checkpointId"].as_str()
        && current.as_ref().map(|s| &s.value()["checkpointId"])
            != Some(&next.value()["checkpointId"])
        && !next.checkpoint_verified(checkpoint, branch)
    {
        return Err(DomainError::InvalidInput("Checkpoint must cite a successful check result using tool-call:<id>; shell success is not verification.".into()));
    }
    let result = result(text, &next, entry.as_deref());
    Ok(TaskPlan {
        next: Some(next),
        result,
    })
}
