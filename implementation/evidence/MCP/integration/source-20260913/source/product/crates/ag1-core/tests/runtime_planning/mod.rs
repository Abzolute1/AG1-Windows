use ag1_core::runtime::planning::{self, Plan, Review, Status, Submission};
use serde_json::{Value, json};
const NOW: &str = "2026-09-06T00:00:00.000Z";

#[test]
fn markdown_plans_and_fallback_interviews_match_literal_source_test_inputs() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-markdown-oracle.json"
    ))
    .unwrap();
    assert!(oracle["rows"].as_array().unwrap().len() >= 15);
    let mut mismatches = Vec::new();
    for row in oracle["rows"].as_array().unwrap() {
        let input = &row["input"];
        let actual = Plan::from_markdown(
            "plan-a",
            input["goal"].as_str().unwrap(),
            input["assistantText"].as_str().unwrap(),
            None,
            None,
            NOW,
        )
        .unwrap();
        if json!(actual) != row["result"] {
            mismatches.push(json!({"line":row["line"],"actual":actual,"expected":row["result"]}));
        }
    }
    for row in oracle["fallback"].as_array().unwrap() {
        let goal = row["goal"].as_str().unwrap();
        assert_eq!(
            json!(planning::should_interview(goal)),
            row["interview"],
            "{goal}"
        );
        assert_eq!(
            json!(planning::allow_inspection(goal, false)),
            row["inspect"],
            "{goal}"
        );
        assert_eq!(
            json!(planning::allow_inspection(goal, true)),
            row["inspectPrevious"],
            "{goal}"
        );
        assert_eq!(
            json!(Plan::fallback("plan-a", goal, None, None, "Sure, let us plan.", NOW).unwrap()),
            row["result"],
            "{goal}"
        );
    }
    assert!(
        mismatches.is_empty(),
        "{}",
        serde_json::to_string_pretty(&mismatches).unwrap()
    );
}

#[test]
fn planning_client_prompts_preserve_saved_answers_progress_and_source_guidance() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-prompt-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 7);
    for case in oracle["rows"].as_array().unwrap() {
        let plan: Plan = serde_json::from_value(case["plan"].clone()).unwrap();
        assert_eq!(json!(plan.format()), case["formatted"]);
        assert_eq!(
            json!(plan.revision_prompt("  Keep it local  ")),
            case["revision"]
        );
        assert_eq!(
            json!(plan.clarification_prompt(" Native ")),
            case["clarification"]
        );
        assert_eq!(
            json!(plan.execution_prompt(None).unwrap()),
            case["execution"]
        );
        if !plan.steps.is_empty() {
            assert_eq!(json!(plan.execution_prompt(Some(0)).unwrap()), case["step"]);
        }
    }
    assert_eq!(
        include_str!("../../resources/runtime-planning-guidance.txt").trim(),
        oracle["guidance"].as_str().unwrap()
    );
}

#[test]
fn structured_plan_workflow_matches_source_and_completed_review_remains_readable() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-workflow-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["submissions"].as_array().unwrap().len(), 6);
    assert_eq!(oracle["reviews"].as_array().unwrap().len(), 8);
    for case in oracle["submissions"].as_array().unwrap() {
        let input = &case["input"];
        let submission: Submission = serde_json::from_value(input["submission"].clone()).unwrap();
        let previous: Option<Plan> = input
            .get("previous")
            .map(|v| serde_json::from_value(v.clone()).unwrap());
        let plan = Plan::from_submission(
            "plan-a",
            input["goal"].as_str().unwrap(),
            &submission,
            previous.as_ref(),
            input["feedback"].as_str(),
            NOW,
        )
        .unwrap();
        assert_eq!(json!(plan), case["result"], "{input}");
    }
    let executing: Plan = serde_json::from_value(oracle["executing"].clone()).unwrap();
    for case in oracle["reviews"].as_array().unwrap() {
        let parsed =
            Review::parse(case["text"].as_str().unwrap(), 0, &executing.steps[0], NOW).unwrap();
        assert_eq!(json!(parsed), case["result"], "{case}");
    }
    let marked = executing.mark_step(0, true, NOW).unwrap();
    assert_eq!(json!(marked), oracle["marked"]);
    let completed = marked.mark_step(1, true, NOW).unwrap();
    assert_eq!(json!(completed), oracle["completed"]);
    assert_eq!(
        json!(completed.mark_step(0, false, NOW).unwrap()),
        oracle["reopened"]
    );
    assert!(executing.mark_step(2, true, NOW).is_err());
    assert!(completed.mark_step(0, true, NOW).is_err());
    assert!(
        executing
            .record_review(
                1,
                "Execution Status:\ncompleted\nExecution Summary:\nDone",
                NOW
            )
            .is_err()
    );
    let reviewed = executing
        .record_review(0, oracle["reviews"][3]["text"].as_str().unwrap(), NOW)
        .unwrap()
        .unwrap();
    let mut corrected = oracle["reviewed"].clone();
    corrected["status"] = json!("accepted");
    assert_eq!(json!(reviewed), corrected);
    let entry = |p: &Plan| json!({"type":"custom","customType":planning::ARTIFACT,"data":p});
    let reread = planning::resolve(
        &[entry(&executing), entry(&reviewed)],
        &reviewed.reference(),
    )
    .unwrap();
    assert_eq!(reread.status, Status::Accepted);
    assert_eq!(reread.next_step(), Some(1));
    // Assistant review text cannot fabricate the completion subsystem's typed
    // verification lineage; this record stays inside planning workflow state.
    assert_eq!(reread.reviews()[0].verification, vec!["cargo test"]);
}
