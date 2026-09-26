//! Runtime feedback remains a labelled observation, independently of build status.
use super::JournalToolPlan;
use crate::{DomainError, evidence::RuntimeObservation, runtime::context::latest_task};
use serde_json::{Value, json};
pub struct ObservationIdentity<'a> {
    pub observation: &'a str,
    pub entry: &'a str,
    pub task_entry: &'a str,
    pub call: &'a str,
}
fn related(feature: &str, task: &Value) -> bool {
    if task["objective"].as_str().is_none_or(str::is_empty) {
        return false;
    }
    let mut text = format!(
        "{} {}",
        task["objective"].as_str().unwrap(),
        task["currentPhase"].as_str().unwrap_or("")
    );
    for question in task["openQuestions"].as_array().into_iter().flatten() {
        text.push(' ');
        text.push_str(question.as_str().unwrap_or(""));
    }
    for entry in task["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["status"] == "active")
    {
        text.push(' ');
        text.push_str(entry["statement"].as_str().unwrap_or(""));
    }
    let text = text.to_lowercase();
    feature
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || "_$-".contains(c)))
        .any(|term| term.len() >= 4 && text.contains(term))
}
pub fn prepare(
    branch: &[Value],
    args: &Value,
    now: &str,
    ids: ObservationIdentity<'_>,
) -> Result<JournalToolPlan, DomainError> {
    let current = latest_task(branch);
    if let Some(objective) = args.get("taskObjective") {
        if objective.as_str().is_none_or(str::is_empty) || current.as_ref().is_none_or(|task| task.value()["objective"] != *objective) {
            return Err(DomainError::InvalidInput("Runtime observation taskObjective does not match the current task. Inspect taskState show before associating this event.".into()));
        }
    }
    let sources = args
        .get("sources")
        .cloned()
        .unwrap_or_else(|| json!([ids.call]));
    let record = RuntimeObservation::parse(
        &json!({"version":1,"id":ids.observation,"feature":args["feature"],"result":args["result"],"environment":args["environment"],"revision":args["revision"],"reporter":args["reporter"],"reproduction":args["reproduction"],"notes":args["notes"],"sources":sources,"observedAt":now}),
    )?;
    let feature = args["feature"].as_str().unwrap();
    let result = args["result"].as_str().unwrap();
    let mut records =
        vec![json!({"type":"custom","customType":"runtime_observation","data":record.value()})];
    let mut task_entry = None;
    if let Some(current) = current
        && args["updateTaskState"].as_bool().unwrap_or(true)
        && (args.get("taskObjective").is_some() || related(feature, current.value()))
    {
        let statement = format!(
            "{feature}: {result} in {}. {}",
            args["environment"].as_str().unwrap(),
            args["notes"].as_str().unwrap()
        );
        let (mut next, id) = current.record(&json!({"kind":"OBSERVATION","statement":statement,"confidence":if result == "pass" { "medium" } else { "low" },"sources":[format!("session-entry:{}", ids.entry)]}), ids.task_entry, now)?;
        if result != "pass" && current.value()["status"] == "completed" {
            next = next.set(&json!({"status":"active"}), now)?;
        }
        records.push(json!({"type":"custom","customType":"task_state","data":next.value()}));
        task_entry = Some(id);
    }
    let mut details =
        json!({"id":ids.observation,"entryId":ids.entry,"result":result,"feature":feature,"verification":"recorded_claim","sourceReference":format!("session-entry:{}",ids.entry)});
    let suffix = if let Some(id) = task_entry {
        details["taskStateEntryId"] = json!(id);
        format!(" (task state {id} updated)")
    } else {
        String::new()
    };
    Ok(JournalToolPlan {
        records,
        result: json!({"content":[{"type":"text","text":format!("Recorded runtime observation {}: {} — {feature}{suffix}. Recording this claim does not independently verify its result; cite a matching check receipt for verification.", ids.observation, result.to_uppercase())}],"details":details}),
    })
}
