//! Pinned task-tool metadata and bounded presentation; no process/model authority.
use super::checkpoint::{Checkpoint, Evidence};
use crate::{DomainError, task::bounded_json};
use serde_json::{Value, json};

pub fn definition() -> Result<Value, DomainError> {
    serde_json::from_str(include_str!("../../../resources/runtime-task-tool.json"))
        .map_err(|_| DomainError::InvalidInput("task tool metadata".into()))
}
fn commas(n: u64) -> String {
    let raw = n.to_string();
    raw.chars()
        .enumerate()
        .fold(String::new(), |mut out, (i, ch)| {
            if i > 0 && (raw.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(ch);
            out
        })
}
pub fn progress_line(turn: u64, calls: u64, tokens: u64, elapsed: u64, action: &str) -> String {
    format!(
        "turn {turn} · {calls} tool calls · ~{} tok · {elapsed}s · {action}",
        commas(tokens)
    )
}
/// Source presentation limits, retaining complete Unicode scalars at the edge.
pub fn action(name: &str, args: &Value) -> String {
    let value = match name {
        "bash" => args["command"].as_str(),
        "read" | "write" | "edit" => Some(
            args["path"]
                .as_str()
                .or_else(|| args["file_path"].as_str())
                .unwrap_or(""),
        ),
        "grep" | "find" => args["pattern"].as_str(),
        "webFetch" => args["url"].as_str(),
        "webSearch" => args["query"].as_str(),
        _ => None,
    };
    match value {
        None => name.into(),
        Some(value) => {
            let text = if value.encode_utf16().count() > 80 {
                let mut units = 0;
                let prefix = value
                    .chars()
                    .take_while(|ch| {
                        units += ch.len_utf16();
                        units <= 79
                    })
                    .collect::<String>();
                format!("{prefix}…")
            } else {
                value.into()
            };
            format!("{name}: {text}")
        }
    }
}
pub fn render(result: &Value) -> Result<String, DomainError> {
    bounded_json(result)?;
    let text = |key: &str| result[key].as_str().unwrap_or("");
    let mut parts = vec![format!(
        "Task {}: {} [{} · {} · {} turns · {}s · {} tok]",
        text("state"),
        text("description"),
        text("type"),
        text("model"),
        result["turns"].as_u64().unwrap_or(0),
        result["durationMs"]
            .as_u64()
            .unwrap_or(0)
            .saturating_add(500)
            / 1000,
        commas(result["usage"]["totalTokens"].as_u64().unwrap_or(0))
    )];
    if !text("error").is_empty() {
        parts.push(format!("Note: {}", text("error")));
    }
    if !text("checkpointId").is_empty() {
        parts.push(format!(
            "Checkpoint: {} (use resumeFrom to retry from this point)",
            text("checkpointId")
        ));
    }
    if result["partial"] == true {
        parts.push("Partial result — the task ended before completing.".into());
    }
    parts.push("Summary:".into());
    parts.push(
        if text("summary").trim().is_empty() {
            "(no output)"
        } else {
            text("summary")
        }
        .into(),
    );
    if let Some(evidence) = result["evidence"]
        .as_array()
        .filter(|entries| !entries.is_empty())
    {
        parts.push(format!("Evidence ({}):", evidence.len()));
        for e in evidence.iter().skip(evidence.len().saturating_sub(10)) {
            parts.push(format!(
                "- {}",
                match e["kind"].as_str() {
                    Some("bash") => format!(
                        "bash: `{}` → exit {}",
                        e["command"].as_str().unwrap_or(""),
                        e["exitCode"]
                            .as_i64()
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "killed".into())
                    ),
                    Some("file") => format!(
                        "{}: {}",
                        e["action"].as_str().unwrap_or(""),
                        e["path"].as_str().unwrap_or("")
                    ),
                    Some("web") => format!("fetch: {}", e["url"].as_str().unwrap_or("")),
                    _ => e["kind"].as_str().unwrap_or("").into(),
                }
            ));
        }
        if evidence.len() > 10 {
            parts.push(format!("- … {} more", evidence.len() - 10));
        }
    }
    Ok(parts.join("\n"))
}
pub fn result_set(checkpoint: &Checkpoint) -> Value {
    let mut reads = vec![];
    let mut writes = vec![];
    let mut bash = vec![];
    let mut web = vec![];
    for entry in &checkpoint.evidence {
        match &entry.evidence {
            Evidence::Inspect { tool, detail } if tool == "read" => {
                if !reads.contains(detail) {
                    reads.push(detail.clone());
                }
            }
            Evidence::File { path, .. } => {
                if !writes.contains(path) {
                    writes.push(path.clone());
                }
            }
            Evidence::Bash { command, exit_code } => {
                bash.push(json!({"command":command,"exitCode":exit_code}))
            }
            Evidence::Web { url } => web.push(url.clone()),
            _ => (),
        }
    }
    json!({"allEvidence":checkpoint.evidence.iter().map(|entry| &entry.evidence).collect::<Vec<_>>(),"filesRead":reads,"filesWritten":writes,"bashCommands":bash,"webFetches":web,"summaryFragments":checkpoint.summaries.iter().map(|s|&s.text).collect::<Vec<_>>()})
}
pub fn validate_rollup(data: &Value, args: &Value) -> Result<(), DomainError> {
    bounded_json(data)?;
    if data["description"] != args["description"]
        || data["type"] != args["type"]
        || !["completed", "failed", "timeout", "budget"]
            .contains(&data["state"].as_str().unwrap_or(""))
        || data["model"].as_str().is_none_or(str::is_empty)
        || data["turns"].as_u64().is_none()
        || data["durationMs"].as_u64().is_none()
        || ["input", "output", "cacheRead", "cacheWrite", "totalTokens"]
            .iter()
            .any(|key| data["usage"][key].as_u64().is_none())
        || data["usage"]["cost"]["total"]
            .as_f64()
            .is_none_or(|cost| !cost.is_finite() || cost < 0.)
        || data["evidence"]
            .as_array()
            .is_none_or(|entries| entries.len() > 20)
    {
        return Err(DomainError::InvalidInput("delegated usage rollup".into()));
    }
    Ok(())
}
