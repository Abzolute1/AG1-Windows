//! Pure compaction preparation helpers. The runtime owns durable custody, epoch fencing and retries.
use super::{estimate_tokens, hygiene::content, slice_utf16, string};
use crate::{
    DomainError,
    task::{invalid, text, units},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
pub const PINNED_REQUEST_PREFIX: &str =
    "[Pinned original request — this is the definition of done. Do not rewrite or drop it.]";
fn window(value: u64) -> u64 {
    if value == 0 { 32768 } else { value }
}
pub fn reserve_tokens(context: u64) -> u64 {
    let w = window(context);
    ((w as f64 * 0.25).floor() as u64).clamp(
        1024.min((w as f64 * 0.15).floor() as u64),
        (w as f64 * 0.35).floor() as u64,
    )
}
pub fn keep_recent(context: u64, overhead: u64) -> u64 {
    let w = window(context);
    let keep = ((w as f64 * 0.35).floor() as u64).clamp(
        2048.min((w as f64 * 0.25).floor() as u64),
        (w as f64 * 0.45).floor() as u64,
    );
    if overhead == 0 {
        return keep;
    }
    let room = w.saturating_sub(reserve_tokens(w)).saturating_sub(overhead);
    keep.min(if room < 1024 { 1024 } else { room })
}
pub fn pin(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.starts_with(PINNED_REQUEST_PREFIX) {
        value.into()
    } else {
        format!("{PINNED_REQUEST_PREFIX}\n\n{value}")
    }
}
pub fn unpin(value: &str) -> &str {
    value
        .trim()
        .strip_prefix(PINNED_REQUEST_PREFIX)
        .unwrap_or(value.trim())
        .trim()
}
pub fn pinned_request(entries: &[Value], previous: Option<&Value>) -> Option<String> {
    previous
        .and_then(|v| v["details"]["pinnedUserRequest"].as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().into())
        .or_else(|| {
            entries.iter().rev().find_map(|e| {
                e["details"]["pinnedUserRequest"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().into())
            })
        })
        .or_else(|| {
            entries
                .iter()
                .filter(|e| e["type"] == "message" && e["message"]["role"] == "user")
                .map(|e| unpin(&content(&e["message"], "\n")).to_string())
                .find(|s| !s.is_empty())
        })
}
pub fn horizon_handoff(modified: &[String], read: &[String]) -> String {
    let mut seen = BTreeSet::new();
    let hot: Vec<_> = modified
        .iter()
        .chain(read.iter().skip(read.len().saturating_sub(12)))
        .filter(|s| seen.insert(*s))
        .map(|s| format!("- {s}"))
        .collect();
    if hot.is_empty() {
        String::new()
    } else {
        format!(
            "## Re-read before editing\nThe repository is the source of truth. Transcript bodies of these files may have been compacted.\nRe-read a file before editing it:\n{}",
            hot.join("\n")
        )
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SummaryState {
    pub pinned_user_request: Option<String>,
    pub goals: Vec<String>,
    pub constraints: Vec<String>,
    pub read_files: Vec<String>,
    pub modified_files: Vec<String>,
    pub failed_attempts: Vec<String>,
    pub verification: Vec<Verification>,
    pub next_steps: Vec<String>,
    pub open_questions: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    pub command: String,
    pub status: String,
    pub detail: Option<String>,
}
fn bullets(items: &[String], empty: &str) -> Vec<String> {
    if items.is_empty() {
        vec![empty.into()]
    } else {
        items.iter().map(|s| format!("- {s}")).collect()
    }
}
pub fn deterministic_summary(state: &SummaryState) -> String {
    let goal = state
        .goals
        .first()
        .filter(|s| !s.is_empty())
        .or(state.pinned_user_request.as_ref())
        .map(|s| {
            let line = s
                .split('\n')
                .map(|s| s.trim_start_matches('#').trim())
                .map(unpin)
                .find(|s| !s.is_empty())
                .unwrap_or(s);
            let clean = text(line, usize::MAX);
            if units(&clean) <= 220 {
                clean
            } else {
                format!("{}...", slice_utf16(&clean, 217))
            }
        })
        .filter(|s| !s.is_empty())
        .unwrap_or("(continue the current task)".into());
    let mut lines = vec![
        "## Goal".into(),
        goal,
        "".into(),
        "## Constraints & Preferences".into(),
    ];
    lines.extend(bullets(&state.constraints, "- (none)"));
    lines.extend(["".into(), "## Files & Evidence".into()]);
    let files: Vec<_> = state
        .modified_files
        .iter()
        .map(|s| format!("- modified: {s}"))
        .chain(state.read_files.iter().map(|s| format!("- read: {s}")))
        .take(80)
        .collect();
    if files.is_empty() {
        lines.push("- (none)".into());
    } else {
        lines.extend(files);
    }
    lines.extend(["".into(), "## Verification".into()]);
    if state.verification.is_empty() {
        lines.push("- (not run)".into());
    } else {
        lines.extend(state.verification.iter().map(|v| {
            format!(
                "- {}: {}{}",
                v.status,
                v.command,
                v.detail
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!(" — {s}"))
                    .unwrap_or_default()
            )
        }));
    }
    lines.extend(["".into(), "## Failed attempts".into()]);
    lines.extend(bullets(&state.failed_attempts, "- (none)"));
    lines.extend(["".into(), "## Next Steps".into()]);
    if state.next_steps.is_empty() {
        lines.push("1. Re-read the hot files and continue the pinned request.".into());
    } else {
        lines.extend(
            state
                .next_steps
                .iter()
                .enumerate()
                .map(|(i, s)| format!("{}. {s}", i + 1)),
        );
    }
    lines.extend(["".into(), "## Open Questions".into()]);
    lines.extend(bullets(&state.open_questions, "- (none)"));
    lines.push("".into());
    lines.push(horizon_handoff(&state.modified_files, &state.read_files));
    lines.join("\n").trim().into()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutPoint {
    pub first_kept_entry_index: usize,
    pub turn_start_index: i64,
    pub is_split_turn: bool,
}
fn starts_turn(e: &Value) -> bool {
    e["type"] == "branch_summary"
        || e["type"] == "custom_message"
        || e["type"] == "message"
            && ["user", "bashExecution"].contains(&string(&e["message"], "role"))
}
pub fn cut_point(
    entries: &[Value],
    start: usize,
    end: usize,
    keep: u64,
) -> Result<CutPoint, DomainError> {
    if start > end || end > entries.len() {
        return Err(invalid("compaction entry range"));
    }
    let points: Vec<_> = (start..end)
        .filter(|i| {
            let e = &entries[*i];
            e["type"] == "branch_summary"
                || e["type"] == "custom_message"
                || e["type"] == "message"
                    && [
                        "bashExecution",
                        "custom",
                        "branchSummary",
                        "compactionSummary",
                        "user",
                        "assistant",
                    ]
                    .contains(&string(&e["message"], "role"))
        })
        .collect();
    let Some(first) = points.first() else {
        return Ok(CutPoint {
            first_kept_entry_index: start,
            turn_start_index: -1,
            is_split_turn: false,
        });
    };
    let mut cut = *first;
    let mut accumulated = 0;
    for i in (start..end).rev() {
        if entries[i]["type"] != "message" {
            continue;
        }
        accumulated += estimate_tokens(&entries[i]["message"]);
        if accumulated >= keep {
            if let Some(point) = points.iter().find(|p| **p >= i) {
                cut = *point;
            }
            break;
        }
    }
    while cut > start
        && entries[cut - 1]["type"] != "compaction"
        && entries[cut - 1]["type"] != "message"
    {
        cut -= 1;
    }
    let user = entries[cut]["type"] == "message" && entries[cut]["message"]["role"] == "user";
    let turn = if user {
        None
    } else {
        (start..=cut).rev().find(|i| starts_turn(&entries[*i]))
    };
    Ok(CutPoint {
        first_kept_entry_index: cut,
        turn_start_index: turn.map_or(-1, |i| i as i64),
        is_split_turn: !user && turn.is_some(),
    })
}
/// Complete pairs can span custom records. Move the cut back to their originating call.
/// This correction preserves the phase's pairing invariant for malformed/non-contiguous histories.
pub fn close_pair_boundary(entries: &[Value], mut cut: usize) -> usize {
    let mut changed = true;
    while changed {
        changed = false;
        let calls: std::collections::BTreeMap<_, _> = entries[..cut]
            .iter()
            .enumerate()
            .filter(|(_, e)| e["type"] == "message" && e["message"]["role"] == "assistant")
            .flat_map(|(i, e)| {
                super::blocks(&e["message"]["content"])
                    .iter()
                    .filter(|b| b["type"] == "toolCall")
                    .map(move |b| (string(b, "id"), i))
            })
            .collect();
        for e in &entries[cut..] {
            if e["type"] == "message"
                && e["message"]["role"] == "toolResult"
                && let Some(i) = calls.get(string(&e["message"], "toolCallId"))
                && *i < cut
            {
                cut = *i;
                changed = true;
                break;
            }
        }
    }
    cut
}
pub fn entry_message(entry: &Value) -> Option<Value> {
    let timestamp = entry["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0);
    match string(entry, "type") {
        "message" => Some(entry["message"].clone()),
        "custom_message" => {
            let mut v = json!({"role":"custom","customType":entry["customType"],"content":entry["content"],"display":entry["display"],"timestamp":timestamp});
            if let Some(d) = entry.get("details") {
                v["details"] = d.clone();
            }
            Some(v)
        }
        "branch_summary" => Some(
            json!({"role":"branchSummary","summary":entry["summary"],"fromId":entry["fromId"],"timestamp":timestamp}),
        ),
        _ => None,
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryPreparation {
    pub first_kept_entry_id: String,
    pub messages_to_summarize: Vec<Value>,
    pub turn_prefix_messages: Vec<Value>,
    pub retained_entries: Vec<Value>,
    pub is_split_turn: bool,
    pub previous_summary: Option<String>,
    pub pinned_user_request: Option<String>,
    pub pair_boundary_adjusted: bool,
}
pub fn prepare(entries: &[Value], keep: u64) -> Result<Option<SummaryPreparation>, DomainError> {
    if entries.last().is_some_and(|e| e["type"] == "compaction") {
        return Ok(None);
    }
    if entries.len() > 100000 {
        return Err(invalid("compaction entry limit"));
    }
    let previous = entries.iter().rposition(|e| e["type"] == "compaction");
    let start = previous.map_or(0, |i| {
        entries
            .iter()
            .position(|e| e["id"] == entries[i]["firstKeptEntryId"])
            .unwrap_or(i + 1)
    });
    let cut = cut_point(entries, start, entries.len(), keep)?;
    if cut.first_kept_entry_index >= entries.len()
        || string(&entries[cut.first_kept_entry_index], "id").is_empty()
    {
        return Ok(None);
    }
    let point = close_pair_boundary(entries, cut.first_kept_entry_index);
    let adjusted = point != cut.first_kept_entry_index;
    let split = cut.is_split_turn && !adjusted;
    let history_end = if split {
        cut.turn_start_index as usize
    } else {
        point
    };
    Ok(Some(SummaryPreparation {
        first_kept_entry_id: string(&entries[point], "id").into(),
        messages_to_summarize: entries[start.min(history_end)..history_end]
            .iter()
            .filter_map(entry_message)
            .collect(),
        turn_prefix_messages: if split {
            entries[cut.turn_start_index as usize..point]
                .iter()
                .filter_map(entry_message)
                .collect()
        } else {
            vec![]
        },
        retained_entries: entries[point..].to_vec(),
        is_split_turn: split,
        previous_summary: previous.and_then(|i| entries[i]["summary"].as_str().map(str::to_string)),
        pinned_user_request: pinned_request(entries, previous.map(|i| &entries[i])),
        pair_boundary_adjusted: adjusted,
    }))
}

pub fn context_messages(entries: &[Value]) -> Vec<Value> {
    let Some(index) = entries.iter().rposition(|e| e["type"] == "compaction") else {
        return entries.iter().filter_map(entry_message).collect();
    };
    let c = &entries[index];
    let timestamp = c["timestamp"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.timestamp_millis())
        .unwrap_or(0);
    let mut messages = vec![
        json!({"role":"compactionSummary","summary":c["summary"],"tokensBefore":c["tokensBefore"],"timestamp":timestamp}),
    ];
    if let Some(first) = entries[..index]
        .iter()
        .position(|e| e["id"] == c["firstKeptEntryId"])
    {
        messages.extend(entries[first..index].iter().filter_map(entry_message));
    }
    messages.extend(entries[index + 1..].iter().filter_map(entry_message));
    messages
}
pub fn usage_estimate(messages: &[Value], hygiene: bool) -> Result<Value, DomainError> {
    let clean = if hygiene {
        super::hygiene::copy(messages, &Default::default())?
    } else {
        messages.to_vec()
    };
    let last = clean.iter().rposition(|m| {
        m["role"] == "assistant"
            && m["usage"].is_object()
            && !["aborted", "error"].contains(&string(m, "stopReason"))
    });
    let (usage, start) = if let Some(i) = last {
        let u = &clean[i]["usage"];
        let total = u["totalTokens"]
            .as_u64()
            .filter(|n| *n > 0)
            .unwrap_or_else(|| {
                ["input", "output", "cacheRead", "cacheWrite"]
                    .iter()
                    .map(|k| u[k].as_u64().unwrap_or(0))
                    .sum()
            });
        (total, i + 1)
    } else {
        (0, 0)
    };
    let trailing: u64 = clean[start..].iter().map(estimate_tokens).sum();
    Ok(
        json!({"tokens":usage+trailing,"usageTokens":usage,"trailingTokens":trailing,"lastUsageIndex":last}),
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_tokens: 40000,
            keep_recent_tokens: 20000,
        }
    }
}
pub fn trigger(context: u64, reserve: u64, virtualized: bool) -> Option<u64> {
    if context == 0 {
        return None;
    }
    let w = context as f64;
    let ceiling = if context >= 200000 { 220000 } else { 75000 };
    Some(if virtualized {
        ceiling.min((w * 0.84).floor() as u64).max(1)
    } else {
        ((w * 0.7).floor() as u64)
            .min(context - reserve.min((w * 0.45).floor() as u64))
            .min(ceiling)
            .max(1)
    })
}
pub fn should_compact(tokens: u64, context: u64, settings: &Settings, virtualized: bool) -> bool {
    settings.enabled
        && trigger(context, settings.reserve_tokens, virtualized).is_some_and(|t| tokens >= t)
}
/// Whole summary input on an explicitly selected branch. Returns no commit or model authority.
pub fn prepare_full(entries: &[Value], settings: &Settings) -> Result<Option<Value>, DomainError> {
    let Some(prep) = prepare(entries, settings.keep_recent_tokens)? else {
        return Ok(None);
    };
    let previous = entries.iter().rfind(|e| e["type"] == "compaction");
    let old = previous
        .filter(|e| e["fromHook"] != true)
        .map(|e| &e["details"]);
    let mut read: Vec<String> = vec![];
    let mut written: Vec<String> = vec![];
    let mut edited: Vec<String> = vec![];
    let add = |list: &mut Vec<String>, value: &str| {
        if !value.is_empty() && !list.iter().any(|s| s == value) {
            list.push(value.into());
        }
    };
    if let Some(old) = old {
        for v in old["readFiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            add(&mut read, v);
        }
        for v in old["modifiedFiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            add(&mut edited, v);
        }
    }
    let all: Vec<_> = prep
        .messages_to_summarize
        .iter()
        .chain(&prep.turn_prefix_messages)
        .cloned()
        .collect();
    for m in &all {
        if m["role"] != "assistant" {
            continue;
        }
        for b in super::blocks(&m["content"]) {
            if b["type"] != "toolCall" {
                continue;
            }
            let path = string(&b["arguments"], "path");
            match string(b, "name") {
                "read" => add(&mut read, path),
                "write" => add(&mut written, path),
                "edit" => add(&mut edited, path),
                _ => {}
            }
        }
    }
    let modified: BTreeSet<_> = edited.iter().chain(&written).cloned().collect();
    let read_only: BTreeSet<_> = read
        .iter()
        .filter(|s| !modified.contains(*s))
        .cloned()
        .collect();
    let task = entries
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == "task_state")
        .find_map(|e| crate::task::TaskState::parse(&e["data"]).ok());
    let inspections = crate::evidence::inspection_history(entries);
    let next_todo = entries
        .iter()
        .rev()
        .find(|e| e["type"] == "custom" && e["customType"] == "todo_list")
        .and_then(|e| e["data"]["items"].as_array())
        .and_then(|items| items.iter().find(|i| i["status"] == "in_progress"))
        .and_then(|i| i["title"].as_str());
    let memory = super::working_memory::extract(
        &all,
        super::working_memory::MemoryInputs {
            previous: old
                .and_then(|v| v.get("workingMemory"))
                .filter(|v| super::working_memory::validate(v).is_ok()),
            read_files: &read_only.into_iter().collect::<Vec<_>>(),
            modified_files: &modified.into_iter().collect::<Vec<_>>(),
            task: task.as_ref(),
            inspection: &inspections,
            next_todo,
        },
    )?;
    let mut value = json!({"firstKeptEntryId":prep.first_kept_entry_id,"messagesToSummarize":prep.messages_to_summarize,"turnPrefixMessages":prep.turn_prefix_messages,"isSplitTurn":prep.is_split_turn,"tokensBefore":usage_estimate(&context_messages(entries),true)?["tokens"],"fileOps":{"read":read,"written":written,"edited":edited},"workingMemory":memory,"settings":settings});
    if let Some(previous) = prep.previous_summary {
        value["previousSummary"] = previous.into();
    }
    if prep.pair_boundary_adjusted {
        value["pairBoundaryAdjusted"] = true.into();
    }
    Ok(Some(value))
}

/// Assemble the reference's deterministic state/file lists around model-authored
/// summary text. An automatic compaction may apply its documented size guard.
/// Reference coordinator's persistent checkpoint, used only after raw custody.
/// It deliberately makes no inferred claim about the unexamined history.
pub fn emergency_checkpoint(preparation: &Value, cause: &str) -> Result<Value, DomainError> {
    crate::task::bounded_json(preparation)?;
    let list = |key: &str| -> BTreeSet<String> {
        preparation["fileOps"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    let read = list("read");
    let modified = list("edited");
    let mut lines = vec![
        "[Semantic summary unavailable; deterministic recovery checkpoint]".into(),
        format!("cause: {}", slice_utf16(cause, 500)),
        "No historical semantic claim was inferred. Use the structured task state for current work and rehydrate the raw trajectory for earlier exact evidence.".into(),
    ];
    for (label, paths) in [("read_files", &read), ("modified_files", &modified)] {
        if !paths.is_empty() {
            lines.push(format!(
                "{label}: {}",
                paths.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(
        json!({"summary":lines.join("\n"),"firstKeptEntryId":preparation["firstKeptEntryId"],"tokensBefore":preparation["tokensBefore"],"details":{"method":"persistent_emergency_checkpoint","readFiles":read,"modifiedFiles":modified,"workingMemory":preparation["workingMemory"]}}),
    )
}

pub fn attach_provenance(
    result: &mut Value,
    artifact: &super::ContextArtifact,
) -> Result<(), DomainError> {
    let summary = result["summary"]
        .as_str()
        .ok_or_else(|| invalid("compaction summary"))?;
    let summary = format!(
        "[COMPACTION WORKING INDEX — MODEL-AUTHORED; NOT PRIMARY EVIDENCE]\nraw_trajectory_artifact: {}\nraw_trajectory_sha256: {}\nUse artifact grep/read to rehydrate original evidence when exactness, conflict resolution, or a material code change depends on this history.\n\n{summary}",
        artifact.id, artifact.sha256
    );
    if !result["details"].is_object() {
        result["details"] = result
            .get("details")
            .map(|value| json!({"priorDetails":value}))
            .unwrap_or(json!({}));
    }
    result["summary"] = summary.into();
    result["details"]["trajectoryArtifactId"] = json!(artifact.id);
    result["details"]["trajectorySha256"] = json!(artifact.sha256);
    result["details"]["summaryAuthority"] = json!("model-authored-index");
    Ok(())
}

pub fn finish_summary(
    preparation: &Value,
    history: &str,
    prefix: Option<&str>,
    automatic_window: Option<u64>,
) -> Result<Value, DomainError> {
    crate::task::bounded_json(preparation)?;
    if history.len() + prefix.map_or(0, str::len) > 16 * 1024 * 1024 {
        return Err(invalid("summary output budget"));
    }
    let mut summary = match prefix {
        Some(prefix) => format!("{history}\n\n---\n\n**Turn Context (split turn):**\n\n{prefix}"),
        None => history.into(),
    };
    let primary = super::working_memory::primary(&preparation["workingMemory"])?;
    if !primary.is_empty() {
        summary = format!("{primary}\n\n---\n\n{summary}");
    }
    let files = &preparation["fileOps"];
    let list = |key: &str| -> BTreeSet<String> {
        files[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect()
    };
    let modified: BTreeSet<_> = list("written").union(&list("edited")).cloned().collect();
    let read: BTreeSet<_> = list("read").difference(&modified).cloned().collect();
    for (name, paths) in [("read-files", &read), ("modified-files", &modified)] {
        if !paths.is_empty() {
            summary.push_str(&format!(
                "\n\n<{name}>\n{}\n</{name}>",
                paths.iter().cloned().collect::<Vec<_>>().join("\n")
            ));
        }
    }
    let mut result = json!({"summary":summary,"firstKeptEntryId":preparation["firstKeptEntryId"],"tokensBefore":preparation["tokensBefore"],"details":{"readFiles":read,"modifiedFiles":modified,"workingMemory":preparation["workingMemory"]}});
    if let Some(window) = automatic_window {
        guard_summary(&mut result, window)?;
    }
    Ok(result)
}

/// Applied after provenance is attached, matching the coordinator's final guard.
pub fn guard_summary(result: &mut Value, window: u64) -> Result<(), DomainError> {
    if window == 0 || window > 16_000_000 {
        return Err(invalid("summary context window"));
    }
    let summary = result["summary"]
        .as_str()
        .ok_or_else(|| invalid("compaction summary"))?;
    let estimate = (crate::task::units(summary) as u64).div_ceil(4);
    if estimate > window / 2 {
        let target = window * 4 / 10;
        result["summary"] = format!(
            "{}\n\n[Summary truncated from ~{} to ~{} tokens to fit the context window]",
            slice_utf16(summary, (target * 4) as usize),
            super::commas(estimate),
            super::commas(target)
        )
        .into();
    }
    Ok(())
}
