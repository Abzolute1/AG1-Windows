//! Deterministic historical indexes. Inferred command outcomes are never current verification.
use super::{blocks, hygiene::content, projections, string};
use crate::{
    DomainError,
    evidence::{Freshness, InspectionEvidence},
    task::{TaskState, bounded_json, canonical_task, invalid, text},
};
use regex::Regex;
use serde_json::{Value, json};
use std::sync::LazyLock;
const LISTS: &[&str] = &[
    "goals",
    "constraints",
    "readFiles",
    "modifiedFiles",
    "commandsRun",
    "failedAttempts",
    "nextSteps",
    "openQuestions",
];
static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
    r"(?i)(?-u:\b)(must|should|need(?:s)? to|required|requirement|constraint|only|never|do not|don't|avoid|stay inside|verify|acceptance|definition of done)(?-u:\b)",
    r"(?i)(?-u:\b)(next|remaining|todo|follow[- ]?up|still need|need(?:s)? to|should now|continue by)(?-u:\b)",
    r"(?i)(?-u:\b)(error|failed|failure|exception|traceback|cannot|can't|enoent|exit code [1-9]|timed out|timeout)(?-u:\b)",
    r"(?i)(?-u:\b)(pass(?:ed|ing)?|success|ok|exit code 0|0 failed|all tests passed)(?-u:\b)",
    r"(?i)(?-u:\b)(?:npm|pnpm|yarn|bun)\s+(?:test|run\s+(?:test|lint|typecheck|build|check)|lint|typecheck|build|check)(?-u:\b)|(?-u:\b)(?:node\s+--test|vitest|jest|pytest|cargo test|go test|playwright test|eslint|tsc|make\s+(?:test|lint|build|check))(?-u:\b)",
    r"(?i)(?-u:\b)(unknown|unclear|blocked|blocker)(?-u:\b)",
].iter().map(|p|Regex::new(p).unwrap()).collect()
});
pub fn empty() -> Value {
    let mut v = json!({"verification":[]});
    for k in LISTS {
        v[k] = json!([]);
    }
    v
}
pub fn validate(v: &Value) -> Result<(), DomainError> {
    bounded_json(v)?;
    for k in LISTS {
        if !v[k]
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string))
        {
            return Err(invalid("working memory lists"));
        }
    }
    let verification = v["verification"]
        .as_array()
        .ok_or_else(|| invalid("working memory verification"))?;
    if verification.iter().any(|r| {
        !r["command"].is_string()
            || !["passed", "failed", "unknown"].contains(&string(r, "status"))
            || r.get("detail").is_some_and(|d| !d.is_string())
    }) {
        return Err(invalid("working memory verification row"));
    }
    if let Some(t) = v.get("taskState") {
        TaskState::parse(t)?;
    }
    if let Some(e) = v.get("inspectionEvidence") {
        for item in e
            .as_array()
            .ok_or_else(|| invalid("working memory inspection"))?
        {
            InspectionEvidence::parse(item)?;
        }
    }
    Ok(())
}
fn add(v: &mut Value, key: &str, value: String) {
    if value.is_empty() {
        return;
    }
    let list = v[key].as_array_mut().unwrap();
    if !list.iter().any(|s| s == &value) {
        list.push(value.into());
    }
}
fn fragments(s: &str) -> Vec<String> {
    let mut parts = vec![];
    let mut current = String::new();
    let mut after_punctuation = false;
    for c in s.chars() {
        if c == '\n' || c.is_whitespace() && after_punctuation {
            if !current.trim().is_empty() {
                parts.push(current.trim().into());
            }
            current.clear();
            after_punctuation = false;
        } else {
            current.push(c);
            after_punctuation = ".!?".contains(c);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().into());
    }
    parts
}
fn verification(v: &mut Value, command: &str, status: &str, detail: Option<String>) {
    let rows = v["verification"].as_array_mut().unwrap();
    if let Some(row) = rows.iter_mut().find(|r| r["command"] == command) {
        row["status"] = status.into();
        if let Some(detail) = detail.filter(|s| !s.is_empty()) {
            row["detail"] = detail.into();
        }
    } else {
        let mut row = json!({"command":command,"status":status});
        if let Some(detail) = detail.filter(|s| !s.is_empty()) {
            row["detail"] = detail.into();
        }
        rows.push(row);
    }
}
#[derive(Default)]
pub struct MemoryInputs<'a> {
    pub previous: Option<&'a Value>,
    pub read_files: &'a [String],
    pub modified_files: &'a [String],
    pub task: Option<&'a TaskState>,
    pub inspection: &'a [InspectionEvidence],
    pub next_todo: Option<&'a str>,
}
pub fn extract(messages: &[Value], input: MemoryInputs<'_>) -> Result<Value, DomainError> {
    super::validate_history(messages)?;
    let mut memory = input.previous.cloned().unwrap_or_else(empty);
    validate(&memory)?;
    let mut pending: Option<String> = None;
    for m in messages {
        match string(m, "role") {
            "user" => {
                let Some(s) = canonical_task(&content(m, "\n")) else {
                    continue;
                };
                let goal = s
                    .lines()
                    .map(str::trim)
                    .find(|s| !s.is_empty())
                    .unwrap_or(s.trim());
                add(&mut memory, "goals", text(goal, 220));
                for f in fragments(&s) {
                    if PATTERNS[0].is_match(&f) {
                        add(&mut memory, "constraints", text(&f, 220));
                    }
                    if f.contains('?') {
                        add(&mut memory, "openQuestions", text(&f, 220));
                    }
                }
            }
            "assistant" => {
                for b in blocks(&m["content"]) {
                    if b["type"] == "text" {
                        for f in fragments(string(b, "text")) {
                            if PATTERNS[1].is_match(&f) {
                                add(&mut memory, "nextSteps", text(&f, 220));
                            }
                            if f.contains('?') || PATTERNS[5].is_match(&f) {
                                add(&mut memory, "openQuestions", text(&f, 220));
                            }
                        }
                    } else if b["type"] == "toolCall" {
                        let args = &b["arguments"];
                        let path = string(args, "path");
                        match string(b, "name") {
                            "read" if !path.is_empty() => {
                                add(&mut memory, "readFiles", path.into())
                            }
                            "edit" | "write" if !path.is_empty() => {
                                add(&mut memory, "modifiedFiles", path.into())
                            }
                            "bash" if args["command"].is_string() => {
                                let command = text(string(args, "command"), 220);
                                add(&mut memory, "commandsRun", command.clone());
                                if PATTERNS[4].is_match(&command) {
                                    verification(&mut memory, &command, "unknown", None);
                                }
                                pending = Some(command);
                            }
                            _ => {}
                        }
                    }
                }
            }
            "toolResult" => {
                let s = content(m, "\n");
                if let Some(command) = pending.take() {
                    if !command.is_empty() && PATTERNS[2].is_match(&s) {
                        add(
                            &mut memory,
                            "failedAttempts",
                            format!("{command}: {}", text(&s, 220)),
                        );
                        if PATTERNS[4].is_match(&command) {
                            verification(&mut memory, &command, "failed", Some(text(&s, 220)));
                        }
                    } else if !command.is_empty()
                        && PATTERNS[4].is_match(&command)
                        && PATTERNS[3].is_match(&s)
                    {
                        verification(&mut memory, &command, "passed", Some(text(&s, 220)));
                    }
                }
            }
            _ => {}
        }
    }
    for f in input.read_files {
        add(&mut memory, "readFiles", f.clone());
    }
    for f in input.modified_files {
        add(&mut memory, "modifiedFiles", f.clone());
    }
    for k in LISTS.iter().copied().chain(["verification"]) {
        let cap = if k == "readFiles" { 24 } else { 12 };
        let list = memory[k].as_array_mut().unwrap();
        if k != "verification" {
            list.retain(|v| v.as_str().is_some_and(|s| !s.is_empty()));
        }
        if list.len() > cap {
            list.drain(..list.len() - cap);
        }
    }
    if let Some(task) = input.task {
        let t = task.value();
        memory["taskState"] = t.clone();
        memory["goals"] = if string(t, "objective").is_empty() {
            json!([])
        } else {
            json!([t["objective"]])
        };
        memory["constraints"] = t["constraints"].clone();
        memory["openQuestions"] = t["openQuestions"].clone();
        memory["nextSteps"] = if t["status"] == "completed" {
            json!([])
        } else {
            json!(
                t["nextAction"]
                    .as_str()
                    .or(input.next_todo)
                    .filter(|s| !s.is_empty())
                    .into_iter()
                    .collect::<Vec<_>>()
            )
        };
    }
    if !input.inspection.is_empty() {
        memory["inspectionEvidence"] = input
            .inspection
            .iter()
            .map(|e| e.value().clone())
            .collect::<Vec<_>>()
            .into();
    }
    Ok(memory)
}
pub fn format(memory: &Value) -> Result<String, DomainError> {
    validate(memory)?;
    let mut sections = vec![];
    if let Some(task) = memory.get("taskState") {
        sections.push(
            projections::task(&TaskState::parse(task)?, |_| None, |_| false).unwrap_or_default(),
        );
    }
    if let Some(e) = memory["inspectionEvidence"].as_array()
        && !e.is_empty()
    {
        let evidence = e
            .iter()
            .map(InspectionEvidence::parse)
            .collect::<Result<Vec<_>, _>>()?;
        sections.push(
            projections::inspection(&evidence, |_| Freshness::Unverified).unwrap_or_default(),
        );
    }
    for (key, label) in [
        ("goals", "goals"),
        ("constraints", "constraints"),
        ("readFiles", "read-files"),
        ("modifiedFiles", "modified-files"),
        ("commandsRun", "commands-run"),
        ("failedAttempts", "failed-attempts"),
        ("verification", "verification"),
        ("nextSteps", "next-steps"),
        ("openQuestions", "open-questions"),
    ] {
        let rows = memory[key].as_array().unwrap();
        if rows.is_empty() {
            continue;
        }
        let lines: Vec<_> = rows
            .iter()
            .map(|r| {
                if key == "verification" {
                    format!(
                        "- {}: {}{}",
                        string(r, "status"),
                        string(r, "command"),
                        r["detail"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!(" — {s}"))
                            .unwrap_or_default()
                    )
                } else {
                    format!("- {}", r.as_str().unwrap())
                }
            })
            .collect();
        sections.push(format!("{label}:\n{}", lines.join("\n")));
    }
    Ok(if sections.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n<working-memory>\n{}\n</working-memory>",
            sections.join("\n\n")
        )
    })
}
pub fn primary(memory: &Value) -> Result<String, DomainError> {
    let s = format(memory)?;
    let s = s.trim();
    Ok(if s.is_empty() {
        String::new()
    } else {
        format!(
            "## Deterministic Working Memory\n\n{s}\n\nThe current request-local structured task state supersedes older summaries and resolved blockers. Commands, failed attempts and verification inferred from prose are historical indexes, not current outcomes. Evidence indexes must be revision-validated before reuse."
        )
    })
}
