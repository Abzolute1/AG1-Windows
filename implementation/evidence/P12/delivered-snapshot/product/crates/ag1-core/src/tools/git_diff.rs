//! Structured Git diff presentation, retaining the pinned hunk representation.
use crate::{DomainError, task::units};
use regex::Regex;
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::LazyLock;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Hunk {
    #[serde(skip_serializing_if = "Option::is_none")]
    from_line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_line: Option<usize>,
    context_start: usize,
    context_end: usize,
    added: Vec<String>,
    removed: Vec<String>,
}
#[derive(Serialize)]
struct Change {
    file: String,
    hunks: Vec<Hunk>,
}
#[derive(Serialize)]
struct Summary {
    files: Vec<String>,
    added: usize,
    removed: usize,
    truncated: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResultBody<'a> {
    summary: Summary,
    changes: Vec<Change>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_diff: Option<&'a str>,
}
pub fn message(text: &str) -> Value {
    json!({"content":[{"type":"text","text":text}],"details":{"files":[],"added":0,"removed":0,"truncated":false}})
}
pub fn project(raw: &str, max_changes: usize) -> Result<Value, DomainError> {
    if raw.len() > 5 * 1024 * 1024 || max_changes == 0 || max_changes > 100000 {
        return Err(DomainError::LimitExceeded("structured Git diff".into()));
    }
    if raw.trim().is_empty() {
        return Ok(message("No changes detected."));
    }
    static FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"a/(.*)\s+b/(.*)").unwrap());
    static HUNK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@").unwrap());
    let mut file = String::new();
    let mut changes: Vec<Change> = vec![];
    let mut active = false;
    let mut count = 0;
    for line in raw.split('\n') {
        if line.starts_with("diff --git") {
            active = false;
            file = FILE
                .captures(line)
                .map(|m| m[2].to_owned())
                .unwrap_or_default();
        } else if [
            "index",
            "new file",
            "old mode",
            "new mode",
            "deleted file",
            "similarity",
            "rename",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        {
            continue;
        } else if line.starts_with("@@") {
            active = false;
            if let Some(captures) = HUNK.captures(line) {
                let parse = |index| {
                    captures
                        .get(index)
                        .and_then(|v| v.as_str().parse::<usize>().ok())
                        .filter(|n| *n != 0)
                };
                changes.push(Change {
                    file: file.clone(),
                    hunks: vec![Hunk {
                        from_line: parse(1),
                        to_line: parse(3),
                        context_start: 0,
                        context_end: 0,
                        added: vec![],
                        removed: vec![],
                    }],
                });
                if changes.len() > 10000 {
                    return Err(DomainError::LimitExceeded("Git hunks".into()));
                }
                active = true;
            }
        } else if active {
            let hunk = &mut changes.last_mut().unwrap().hunks[0];
            if let Some(line) = line.strip_prefix('-') {
                hunk.removed.push(line.into());
                count += 1;
            } else if let Some(line) = line.strip_prefix('+') {
                hunk.added.push(line.into());
                count += 1;
            } else {
                hunk.context_end += 1;
            }
            if count >= max_changes {
                break;
            }
        }
    }
    let mut files = vec![];
    let mut added = 0;
    let mut removed = 0;
    for change in &mut changes {
        if !files.contains(&change.file) {
            files.push(change.file.clone());
        }
        for hunk in &mut change.hunks {
            added += hunk.added.len();
            removed += hunk.removed.len();
            if hunk.added.is_empty() && hunk.removed.is_empty() {
                hunk.context_end = 0;
            }
        }
    }
    // P09 correction: the reference enforces a global change limit but checks each
    // individual hunk to label truncation. A cut spanning two hunks must also be visible.
    let summary = Summary {
        files,
        added,
        removed,
        truncated: count >= max_changes,
    };
    let details = serde_json::to_value(&summary)
        .map_err(|_| DomainError::InvalidInput("Git summary".into()))?;
    let body = ResultBody {
        summary,
        changes,
        raw_diff: (units(raw) < 10240).then_some(raw),
    };
    let text = serde_json::to_string_pretty(&body)
        .map_err(|_| DomainError::InvalidInput("Git diff JSON".into()))?;
    Ok(json!({"content":[{"type":"text","text":text}],"details":details}))
}
