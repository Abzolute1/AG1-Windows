use ag1_core::{
    runtime::subagent::{
        self, Profile,
        checkpoint::{Checkpoint, Evidence, State, Usage},
    },
    tools::policy::Policy,
};
use serde_json::{Value, json};
const NOW: &str = "2026-09-06T00:00:00.000Z";

#[test]
fn task_metadata_and_twelve_parent_result_formats_match_pinned_source() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/task-tool-oracle.json"
    ))
    .unwrap();
    assert_eq!(subagent::tool::definition().unwrap(), oracle["metadata"]);
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 12);
    for row in oracle["rows"].as_array().unwrap() {
        assert_eq!(
            subagent::tool::render(&row["input"]).unwrap(),
            row["output"].as_str().unwrap()
        );
    }
}

#[test]
fn delegated_model_routing_matches_twenty_pinned_choices_and_errors() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/subagent-loop-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["routing"].as_array().unwrap().len(), 20);
    for row in oracle["routing"].as_array().unwrap() {
        let actual = subagent::routing::select(
            serde_json::from_value(row["type"].clone()).unwrap(),
            row["explicit"].as_str(),
            &row["parent"],
            row["candidates"].as_array().unwrap(),
        );
        if row["selected"].is_null() {
            assert!(actual.is_err());
        } else {
            assert_eq!(actual.unwrap(), row["selected"]);
        }
    }
}

#[test]
fn subagent_profiles_and_parent_policy_match_the_pinned_runner() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/subagent-oracle.json"
    ))
    .unwrap();
    assert_eq!(
        oracle["defaults"],
        json!({"maxTurns":subagent::DEFAULT_MAX_TURNS,"maxTurnsCap":subagent::MAX_TURNS,"timeout":subagent::DEFAULT_TIMEOUT_SECONDS,"tokenBudget":subagent::DEFAULT_TOKEN_BUDGET})
    );
    for row in oracle["profiles"].as_array().unwrap() {
        let profile: Profile = serde_json::from_value(row["type"].clone()).unwrap();
        assert_eq!(json!(profile.tools()), row["tools"]);
        assert!(!profile.tools().contains(&"task"));
        for case in row["policies"].as_array().unwrap() {
            let policy: Policy = serde_json::from_value(json!({"mode":case["mode"],"cwd":"/fixture","trustedTools":["read"],"extensionTools":["explicit"]})).unwrap();
            let resolved = profile.policy(&policy);
            assert_eq!(json!(resolved.mode), case["output"]["mode"]);
            assert_eq!(resolved.cwd, "/fixture");
            assert_eq!(resolved.trusted_tools, policy.trusted_tools);
            assert_eq!(resolved.extension_tools, policy.extension_tools);
        }
    }
    assert_eq!(subagent::max_turns(None), 12);
    assert_eq!(subagent::max_turns(Some(0)), 12);
    assert_eq!(subagent::max_turns(Some(100)), 25);
}

#[test]
fn checkpoint_state_matches_source_and_resume_marks_prior_findings_as_historical() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/subagent-oracle.json"
    ))
    .unwrap();
    let expected = &oracle["checkpointRows"][0];
    let mut checkpoint = Checkpoint::new(
        "ckpt_fixture".into(),
        Profile::Explore,
        "Inspect the fixture".into(),
        NOW.into(),
    )
    .unwrap();
    checkpoint
        .add_evidence(
            Evidence::Inspect {
                tool: "read".into(),
                detail: "src/main.rs".into(),
            },
            0,
            NOW,
        )
        .unwrap();
    checkpoint
        .add_summary("Found the entry point", 0, NOW)
        .unwrap();
    checkpoint.usage = Usage {
        input: 5,
        output: 3,
        total_tokens: 8,
        cost: 0.0,
    };
    checkpoint.last_turn = 1;
    checkpoint.final_state = Some(State::Failed);
    let mut value = json!(checkpoint);
    value["id"] = json!("<checkpoint>");
    // JSON number semantics agree; JavaScript spells integral costs without .0.
    assert_eq!(
        value["usage"]["cost"].as_f64(),
        expected["state"]["usage"]["cost"].as_f64()
    );
    value["usage"]["cost"] = expected["state"]["usage"]["cost"].clone();
    assert_eq!(value, expected["state"]);
    let resume = checkpoint.resume_prompt("Continue inspection").unwrap();
    let source = expected["resume"].as_str().unwrap();
    assert_eq!(
        resume.rsplit_once('\n').unwrap().0,
        source.rsplit_once('\n').unwrap().0
    );
    assert!(resume.ends_with(
        "Prior findings are historical; revalidate source-dependent claims before acting."
    ));
    assert_eq!(
        oracle["defects"],
        json!({"silentWriteFailure":true,"checkpointTraversal":true,"readonlyAccepted":true,"readonlyMutated":true})
    );
}

#[test]
fn checkpoint_identity_limits_and_rejected_updates_preserve_prior_state() {
    for id in [
        "../outside",
        "ckpt_../outside",
        "ckpt_",
        "ckpt_a/b",
        "ckpt_a\\b",
    ] {
        assert!(subagent::checkpoint_id(id).is_err());
    }
    let mut checkpoint = Checkpoint::new(
        "ckpt_limits".into(),
        Profile::Plan,
        "Plan".into(),
        NOW.into(),
    )
    .unwrap();
    for i in 0..105 {
        checkpoint
            .add_evidence(
                Evidence::Inspect {
                    tool: "read".into(),
                    detail: format!("src/{i}.rs"),
                },
                i,
                NOW,
            )
            .unwrap();
        checkpoint.add_summary(&"😀".repeat(1500), i, NOW).unwrap();
    }
    assert_eq!(checkpoint.evidence.len(), 100);
    assert_eq!(checkpoint.summaries.len(), 20);
    assert_eq!(checkpoint.tool_calls, 105);
    assert_eq!(checkpoint.summaries[0].text.encode_utf16().count(), 2000);
    let before = checkpoint.clone();
    assert!(
        checkpoint
            .add_evidence(
                Evidence::Web {
                    url: "x".repeat(1024 * 1024 + 1)
                },
                106,
                NOW
            )
            .is_err()
    );
    assert_eq!(checkpoint, before);
    assert!(
        checkpoint
            .add_summary("Invalid timestamp", 106, "bad")
            .is_err()
    );
    assert_eq!(checkpoint, before);
}
