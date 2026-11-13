use ag1_core::{
    context::projections,
    evidence::{Freshness, SourceValidation},
    task::TaskState,
};
use serde_json::json;

#[test]
fn unavailable_task_tool_preserves_constraints_and_stale_evidence_without_mutation() {
    let now = "2026-09-08T00:00:00Z";
    let source = format!("file:src/subject.rs@sha256:{}", "a".repeat(64));
    let task = TaskState::reset("Retained objective", now).unwrap()
        .set(&json!({"constraints":["Keep user data"],"latestInstruction":"Review the current change"}), now).unwrap();
    let (task, _) = task
        .record(
            &json!({"kind":"FACT","statement":"Historical source claim","sources":[source]}),
            "ts_00000001",
            now,
        )
        .unwrap();
    let before = task.value().clone();
    let projection = projections::task_with_tool(
        &task,
        |reference| {
            Some(SourceValidation {
                reference: reference.into(),
                status: Freshness::Stale,
                current: None,
            })
        },
        |_| false,
        false,
    )
    .unwrap();
    assert!(projection.contains("Keep user data"));
    assert!(projection.contains("Review the current change"));
    assert!(projection.contains("Historical source claim"));
    assert!(projection.contains("stale_source=true"));
    assert!(projection.contains("[STALE]"));
    assert!(!projection.contains("update with taskState"));
    assert_eq!(task.value(), &before);
}

#[test]
fn clipped_task_state_cannot_instruct_an_unavailable_show_action() {
    let now = "2026-09-08T00:00:00Z";
    let constraints: Vec<_> = (0..20)
        .map(|i| format!("{i}:{}", "x".repeat(490)))
        .collect();
    let task = TaskState::reset("Large retained task", now)
        .unwrap()
        .set(&json!({"constraints": constraints}), now)
        .unwrap();
    let readonly = projections::task_with_tool(&task, |_| None, |_| false, false).unwrap();
    assert!(
        readonly.ends_with("[recorded task state clipped; taskState is unavailable in this turn]")
    );
    assert!(!readonly.contains("taskState show"));
    let editable = projections::task(&task, |_| None, |_| false).unwrap();
    assert!(editable.ends_with("[task state clipped; use the current task-state tool with action=show for the complete state]"));
}

#[test]
fn preferred_task_projection_names_the_selected_work_tool() {
    let task = TaskState::reset("Current work", "2026-09-13T00:00:00Z").unwrap();
    let preferred = projections::task_with_control_tool(
        &task,
        |_| None,
        |_| false,
        Some(projections::TaskControlTool::Work),
    )
    .unwrap();
    assert!(preferred.contains("update with work"));
    assert!(!preferred.contains("taskState"));
    assert!(
        projections::task(&task, |_| None, |_| false)
            .unwrap()
            .contains("update with taskState")
    );
}
