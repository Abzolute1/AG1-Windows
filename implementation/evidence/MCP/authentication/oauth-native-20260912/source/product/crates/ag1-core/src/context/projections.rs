//! Per-turn projections consume validated state and explicit evidence verdicts.
use super::{slice_utf16, string};
use crate::{
    evidence::{Freshness, InspectionEvidence, RuntimeObservation, SourceValidation},
    memory::{MemorySelection, context_line},
    task::TaskState,
    task::units,
};
use serde_json::Value;
use std::collections::BTreeSet;
fn clip(text: String, cap: usize, marker: &str) -> String {
    if units(&text) <= cap {
        text
    } else {
        format!("{}\n{marker}", slice_utf16(&text, cap))
    }
}
pub fn task(
    state: &TaskState,
    validate: impl FnMut(&str) -> Option<SourceValidation>,
    verify: impl FnMut(&str) -> bool,
) -> Option<String> {
    task_with_tool(state, validate, verify, true)
}
/// Keep recorded evidence visible when the selected tools cannot edit task state.
pub fn task_with_tool(
    state: &TaskState,
    mut validate: impl FnMut(&str) -> Option<SourceValidation>,
    mut verify: impl FnMut(&str) -> bool,
    task_tool_available: bool,
) -> Option<String> {
    let s = state.value();
    if string(s, "objective").is_empty() {
        return None;
    }
    let heading = if task_tool_available {
        "Current task control state (claims retain their evidence/confidence; update with taskState):"
    } else {
        "Recorded task state (read-only in this turn; claims retain their evidence/confidence):\nThe taskState tool is unavailable. Shell output cannot update this state. Its recorded status does not require bookkeeping before answering the latest user instruction."
    };
    let mut lines = vec![
        heading.into(),
        format!("objective: {}", string(s, "objective")),
        format!("status: {}", string(s, "status")),
    ];
    for (field, label) in [
        ("currentPhase", "phase"),
        ("latestInstruction", "latest_user_instruction"),
        ("currentSubgoal", "current_subgoal"),
    ] {
        if !string(s, field).is_empty() {
            lines.push(format!("{label}: {}", string(s, field)));
        }
    }
    lines.push(format!(
        "next_action: {}",
        s["nextAction"]
            .as_str()
            .unwrap_or(if task_tool_available {
                "not recorded; choose and record an evidence-bearing next action"
            } else {
                "not recorded; use the latest instruction and current evidence to decide whether more work is needed"
            })
    ));
    if !string(s, "blockerId").is_empty() {
        lines.push(format!("active_blocker: {}", string(s, "blockerId")));
    }
    let attempts: Vec<_> = s["attemptIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !attempts.is_empty() {
        lines.push(format!("attempt_links: {}", attempts.join(", ")));
    }
    let entries = s["entries"].as_array().unwrap();
    let checkpoint = string(s, "checkpointId");
    if !checkpoint.is_empty() {
        let stale = entries
            .iter()
            .find(|e| e["id"] == checkpoint)
            .is_some_and(|e| {
                e["sources"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|source| validate(source).is_some_and(|v| v.status != Freshness::Current))
            });
        lines.push(format!("checkpoint: {checkpoint} verification={} (not task completion; untracked external changes require rechecking)",if !stale && verify(checkpoint){"recorded_check_passed"}else{"unverified_or_stale"}));
    }
    for (field, label) in [
        ("constraints", "constraints"),
        ("openQuestions", "open_questions"),
    ] {
        let rows = s[field].as_array().unwrap();
        if !rows.is_empty() {
            lines.push(format!("{label}:"));
            lines.extend(
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(|v| format!("- {v}")),
            );
        }
    }
    let pinned: BTreeSet<_> = attempts
        .into_iter()
        .chain([string(s, "blockerId"), checkpoint])
        .collect();
    let active: Vec<_> = entries.iter().filter(|e| e["status"] == "active").collect();
    let selected: Vec<_> = active
        .iter()
        .filter(|e| pinned.contains(string(e, "id")))
        .chain(
            active
                .iter()
                .rev()
                .filter(|e| !pinned.contains(string(e, "id"))),
        )
        .take(30)
        .collect();
    if !selected.is_empty() {
        lines.push("knowledge:".into());
    }
    for e in selected {
        let checked: Vec<_> = e["sources"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(|source| (source, validate(source)))
            .collect();
        let stale = checked
            .iter()
            .any(|(_, v)| v.as_ref().is_some_and(|v| v.status != Freshness::Current));
        let confidence = e["confidence"]
            .as_str()
            .map(|s| format!(" confidence={s}"))
            .unwrap_or_default();
        lines.push(format!(
            "- {} [{}{confidence}{}] {}",
            string(e, "id"),
            string(e, "kind"),
            if stale { " stale_source=true" } else { "" },
            string(e, "statement")
        ));
        if !string(e, "reason").is_empty() {
            lines.push(format!("  reason: {}", string(e, "reason")));
        }
        if !checked.is_empty() {
            lines.push(format!(
                "  sources: {}",
                checked
                    .iter()
                    .map(|(source, v)| match v {
                        Some(v) if v.status != Freshness::Current => format!(
                            "{source} [{}{}]",
                            v.status.to_string().to_uppercase(),
                            v.current
                                .as_ref()
                                .map(|s| format!(" current={s}"))
                                .unwrap_or_default()
                        ),
                        _ => source.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Some(clip(
        lines.join("\n"),
        6000,
        if task_tool_available {
            "[task state clipped; use the current task-state tool with action=show for the complete state]"
        } else {
            "[recorded task state clipped; taskState is unavailable in this turn]"
        },
    ))
}
pub fn memory(selection: &MemorySelection) -> Option<String> {
    if selection.entries.is_empty() {
        return None;
    }
    let shown = selection.entries.len();
    let count = selection.stored_count;
    let header = if count > shown {
        format!(
            "Project memory ({count} stored, {shown} shown, {} older/not shown; cold memory remains searchable):",
            count - shown
        )
    } else {
        format!("Project memory ({count} stored — keep current via the memory tool):")
    };
    let warning = if selection.source_statuses.values().any(|s| s.historical()) {
        "Warning: historical stale/missing memory is not current evidence; rehydrate or inspect current source before acting.\n"
    } else {
        ""
    };
    let lines: Vec<_> = selection
        .entries
        .iter()
        .map(|e| {
            context_line(
                e,
                selection
                    .source_statuses
                    .get(string(e, "id"))
                    .copied()
                    .unwrap_or(Freshness::Unverified),
            )
        })
        .collect();
    Some(format!("{header}\n{warning}{}", lines.join("\n")))
}
pub fn inspection(
    evidence: &[InspectionEvidence],
    mut validate: impl FnMut(&str) -> Freshness,
) -> Option<String> {
    if evidence.is_empty() {
        return None;
    }
    let mut lines = vec![
        "Inspection evidence indexes (not source contents or proof about unsearched scopes):"
            .into(),
    ];
    for e in evidence.iter().rev() {
        let e = e.value();
        let source = string(e, "source");
        let row = format!(
            "- {} {} source={source} status={} result=tool-call:{}",
            string(e, "kind"),
            e["scope"],
            validate(source),
            string(e, "toolCallId")
        );
        if units(&lines.join("\n")) + units(&row) > 3000 {
            break;
        }
        lines.push(row);
    }
    lines.push("Stale/missing/unverified indexes require fresh inspection; no_match applies only to the recorded pattern, flags and file revision. A read range does not imply whole-file coverage.".into());
    Some(lines.join("\n"))
}
pub fn runtime(entries: &[Value], limit: usize) -> Option<String> {
    let rows: Vec<_> = entries
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == "runtime_observation")
        .filter_map(|e| RuntimeObservation::parse(&e["data"]).ok())
        .take(limit.clamp(1, 50))
        .collect();
    if rows.is_empty() {
        return None;
    }
    let mut lines=vec!["Runtime verification observations (structured evidence; separate from source/build status):".into()];
    for row in rows {
        let o = row.value();
        lines.extend([
            format!(
                "- {} [{}] {} — reporter={}, environment={}, revision={}",
                string(o, "id"),
                string(o, "result").to_uppercase(),
                string(o, "feature"),
                string(o, "reporter"),
                string(o, "environment"),
                string(o, "revision")
            ),
            format!("  reproduce: {}", string(o, "reproduction")),
            format!("  notes: {}", string(o, "notes")),
        ]);
    }
    Some(clip(
        lines.join("\n"),
        6000,
        "[runtime observations clipped; use runtimeObservation records for complete history]",
    ))
}
#[derive(Default)]
pub struct TurnAddenda<'a> {
    pub runtime_pending: Option<&'a str>,
    pub verification_command: Option<&'a str>,
    pub todo: Option<&'a str>,
    pub memory: Option<&'a str>,
    pub orientation: Option<&'a str>,
    pub finished_jobs: Option<&'a str>,
}
pub fn turn(input: TurnAddenda<'_>) -> Option<String> {
    let contract=input.verification_command.filter(|s|!s.is_empty()).map(|command|format!("<runtime-verification-contract>\n{{\"kind\":\"repository_verifier\",\"version\":1,\"command\":{},\"required_after_mutation\":true}}\nUse this canonical repository verifier after the final mutation. Alternative or focused tests may supplement it, but do not replace it.\n</runtime-verification-contract>",serde_json::to_string(command).unwrap()));
    let parts: Vec<_> = [
        input.runtime_pending,
        contract.as_deref(),
        input.todo,
        input.memory,
        input.orientation,
        input.finished_jobs,
    ]
    .into_iter()
    .flatten()
    .filter(|s| !s.trim().is_empty())
    .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// Checklist status is a declaration and cannot stand in for task verification.
pub fn task_plan(branch: &[Value], state: Option<&TaskState>) -> Option<String> {
    let (todo, items) = branch
        .iter()
        .rev()
        .filter(|e| e["type"] == "custom" && e["customType"] == "todo_list")
        .find_map(|e| {
            let d = &e["data"];
            if d["version"] != 1
                || !d["updatedAt"].is_number()
                || d.get("planId")
                    .is_some_and(|v| v.as_str().is_none_or(|s| s.trim().is_empty()))
                || d.get("phase").is_some_and(|v| !v.is_string())
            {
                return None;
            }
            crate::todo::validate_items(&d["items"])
                .ok()
                .map(|items| (d, items))
        })?;
    if items.is_empty() {
        return None;
    }
    let pending = items.iter().filter(|e| e["status"] != "completed").count();
    let phase = string(todo, "phase").trim();
    let mut lines = vec![
        "Checklist reconciliation (declared status, not verification):".into(),
        format!(
            "plan: {}{}",
            todo["planId"].as_str().map(str::trim).unwrap_or("legacy"),
            if phase.is_empty() {
                String::new()
            } else {
                format!(" phase={phase}")
            }
        ),
        format!("open_steps: {pending}/{}", items.len()),
    ];
    if let Some(active) = items.iter().find(|e| e["status"] == "in_progress") {
        lines.push(format!(
            "todo_focus: {} {}",
            string(active, "id"),
            string(active, "title")
        ));
    }
    let completed = state.is_some_and(|s| s.value()["status"] == "completed");
    if completed && pending > 0 {
        lines.push("CONFLICT: task says completed but checklist remains open; reconcile before claiming completion.".into());
    }
    if !completed && pending == 0 {
        lines.push(
            "Checklist complete; task completion still requires its own evidence and decision."
                .into(),
        );
    }
    if crate::todo::calls_since_update(branch) >= 6 {
        lines.push("Checklist has not been updated across recent tool results; review it against evidence, without inventing progress.".into());
    }
    Some(lines.join("\n"))
}
