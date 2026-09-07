//! Request-local working state. These projections never become journal messages.
use crate::{
    DomainError,
    context::{ContextArtifactPort, episode_retrieval, hygiene, projections},
    evidence::{Freshness, SourceReference, SourceValidation, inspection_history},
    task::TaskState,
};
use serde_json::{Value, json};
pub const WORKING_STATE: &str =
    "[AG1 REQUEST-LOCAL WORKING STATE — current for this model call; not conversation history]\n";
pub fn latest_task(branch: &[Value]) -> Option<TaskState> {
    branch
        .iter()
        .rev()
        .filter(|entry| entry["type"] == "custom" && entry["customType"] == "task_state")
        .find_map(|entry| TaskState::parse(&entry["data"]).ok())
}
pub fn latest_query(messages: &[Value]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message["role"] == "user")
        .map(|message| hygiene::content(message, "\n"))
}
fn tagged(out: &mut String, tag: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        out.push_str(&format!("\n\n<{tag}>\n{value}\n</{tag}>"));
    }
}
pub fn working_state(
    branch: &[Value],
    messages: &[Value],
    artifacts: &impl ContextArtifactPort,
    mut validate: impl FnMut(&str) -> Option<SourceValidation>,
) -> Option<String> {
    let task = latest_task(branch);
    let task_text = task.as_ref().and_then(|task| {
        projections::task(
            task,
            |source| {
                let mut verdict = validate(source)?;
                // The platform retains a rehydratable current reference; the
                // AgentSession task addendum displays only its revision hash.
                if source.starts_with("file:") {
                    verdict.current = verdict.current.map(|current| {
                        SourceReference::parse(&current)
                            .map(|reference| reference.digest().to_owned())
                            .unwrap_or(current)
                    });
                }
                Some(verdict)
            },
            |id| task.checkpoint_verified(id, branch),
        )
    });
    let runtime = [
        projections::runtime(branch, 12),
        projections::task_plan(branch, task.as_ref()),
        projections::inspection(&inspection_history(branch), |source| {
            validate(source)
                .map(|v| v.status)
                .unwrap_or(Freshness::Unverified)
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n\n");
    let episodes = latest_query(messages).and_then(|query| {
        episode_retrieval::addendum(branch, &query, artifacts, |source| {
            validate(source)
                .map(|v| v.status)
                .unwrap_or(Freshness::Unverified)
        })
        .ok()
        .flatten()
    });
    let mut tail = String::new();
    tagged(&mut tail, "structured-task-state", task_text.as_deref());
    tagged(&mut tail, "runtime-observations", Some(&runtime));
    tagged(&mut tail, "episode-pointers", episodes.as_deref());
    let tail = tail.trim();
    if tail.is_empty() {
        None
    } else {
        Some(format!("{WORKING_STATE}{tail}"))
    }
}
/// Match the SDK's stable insertion point: augment the latest user message,
/// keeping all assistant/tool rounds after it untouched in the cached prefix.
pub fn append_working_state(
    messages: &[Value],
    tail: Option<&str>,
) -> Result<Vec<Value>, DomainError> {
    let mut result = messages.to_vec();
    let Some(tail) = tail else {
        return Ok(result);
    };
    if let Some(index) = result.iter().rposition(|message| message["role"] == "user") {
        let content = &mut result[index]["content"];
        match content {
            Value::String(text) => {
                text.push_str("\n\n");
                text.push_str(tail);
            }
            Value::Array(blocks) => blocks.push(json!({"type":"text","text":tail})),
            _ => return Err(DomainError::InvalidInput("user context content".into())),
        }
    } else {
        result.push(json!({"role":"custom","customType":"ag1-working-state","content":tail,"display":false,"timestamp":0}));
    }
    Ok(result)
}
