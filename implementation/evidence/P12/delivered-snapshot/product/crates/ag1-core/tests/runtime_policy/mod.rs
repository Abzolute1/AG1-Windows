use ag1_core::{
    DomainError,
    runtime::{
        completion,
        policy::{self, Intent, Mode, evidence},
        verification::Index,
    },
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[test]
fn controlled_task_policy_and_evidence_plans_match_pinned_sdk() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/policy-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["classifications"].as_array().unwrap().len(), 42);
    for row in oracle["classifications"].as_array().unwrap() {
        let prompt = row["prompt"].as_str().unwrap();
        let expected: Intent = serde_json::from_value(row["intent"].clone()).unwrap();
        assert_eq!(policy::classify(prompt), expected, "{prompt:?}");
        assert_eq!(
            completion::expects_mutation(prompt),
            expected.mutation_expected(),
            "{prompt:?}"
        );
    }
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 120);
    let mut corrected = 0;
    for row in rows {
        let input = &row["index"];
        let index = Index::captured(
            serde_json::from_value(input["files"].clone()).unwrap(),
            serde_json::from_value(input["packageScripts"].clone()).unwrap(),
            &serde_json::from_value::<BTreeMap<String, String>>(input["sources"].clone()).unwrap(),
        )
        .unwrap();
        let policy = policy::infer(
            row["prompt"].as_str().unwrap(),
            &index,
            serde_json::from_value(row["mode"].clone()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            json!(policy),
            row["policy"],
            "{} {} {}",
            row["id"],
            row["prompt"],
            row["mode"]
        );
        assert_eq!(json!(policy.format()), row["formatted"]);
        let plan = evidence::build(&policy).unwrap();
        assert_eq!(json!(plan), row["plan"]);
        assert_eq!(
            json!(plan.iter().map(evidence::Step::format).collect::<Vec<_>>()),
            row["steps"]
        );
        for next in row["next"].as_array().unwrap() {
            let state: evidence::State = serde_json::from_value(next["state"].clone()).unwrap();
            let actual = evidence::next(&policy, &plan, &state);
            if policy.intent == Intent::VerificationOnly
                && !state.mutation_since_verifier
                && !state.verifier_passed_after_mutation
            {
                // Recorded source defect: source selects final_answer despite
                // the required verifier still having no terminal observation.
                assert_eq!(next["step"]["action"], "final_answer");
                assert_eq!(actual.unwrap().action, evidence::Action::Verify);
                corrected += 1;
            } else {
                assert_eq!(
                    json!(actual),
                    next["step"],
                    "{} {}",
                    row["id"],
                    next["state"]
                );
            }
        }
    }
    assert_eq!(corrected, 9);
}

#[test]
fn verification_only_planner_requires_observation_and_does_not_invent_mutation() {
    let index =
        Index::captured(vec!["Cargo.toml".into()], BTreeMap::new(), &BTreeMap::new()).unwrap();
    let policy = policy::infer("Run tests only", &index, Mode::Controlled).unwrap();
    assert!(!policy.mutation_expected);
    let plan = evidence::build(&policy).unwrap();
    let mut state = evidence::State::default();
    assert_eq!(
        evidence::next(&policy, &plan, &state).unwrap().format(),
        "bash cargo test"
    );
    // Inspecting files is not evidence of verification.
    state.read_paths.insert("Cargo.toml".into());
    assert_eq!(
        evidence::next(&policy, &plan, &state).unwrap().format(),
        "bash cargo test"
    );
    state.verification_completed = true;
    assert_eq!(
        evidence::next(&policy, &plan, &state).unwrap().action,
        evidence::Action::FinalAnswer
    );
    assert!(!state.mutation_since_verifier);
    assert!(!state.verifier_passed_after_mutation);
}

#[test]
fn task_policy_is_bounded_and_readonly_prompts_ignore_verifier_instructions() {
    let index = Index::captured(vec![], BTreeMap::new(), &BTreeMap::new()).unwrap();
    let prompt = format!("Fix it. Verification command is: {}", "x".repeat(4097));
    assert!(matches!(
        policy::infer(&prompt, &index, Mode::Controlled),
        Err(DomainError::LimitExceeded(_))
    ));
    assert!(matches!(
        policy::infer(&"x".repeat(4 * 1024 * 1024 + 1), &index, Mode::Controlled),
        Err(DomainError::LimitExceeded(_))
    ));
    let mut policy =
        policy::infer("Do not use tools. Explain the fix.", &index, Mode::Flexible).unwrap();
    assert!(!policy.mutation_expected);
    assert!(policy.verification.is_none());
    policy.allowed_paths = vec!["file.js".into(); 1001];
    assert!(matches!(
        evidence::build(&policy),
        Err(DomainError::LimitExceeded(_))
    ));
    policy.allowed_paths = vec!["bad\0path".into()];
    assert!(matches!(
        evidence::build(&policy),
        Err(DomainError::LimitExceeded(_))
    ));
}

#[test]
fn failure_repair_and_stop_controllers_match_pinned_source_traces() {
    use policy::{failure, repair, stop};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/policy-control-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["failures"].as_array().unwrap().len(), 60);
    for row in oracle["failures"].as_array().unwrap() {
        let relevant = serde_json::from_value::<Vec<String>>(row["relevantPaths"].clone()).unwrap();
        assert_eq!(
            json!(failure::classify(row["output"].as_str().unwrap(), &relevant).unwrap()),
            row["result"],
            "{}",
            row["output"]
        );
    }
    for row in oracle["repairs"].as_array().unwrap() {
        let options = &row["options"];
        let paths = |name| {
            serde_json::from_value::<Vec<String>>(options.get(name).cloned().unwrap_or(json!([])))
                .unwrap()
        };
        let mut controller = repair::Controller::new(
            paths("relevantPaths"),
            paths("expectedChangedPaths"),
            options["repeatedFailureLimit"].as_u64().unwrap_or(2) as u32,
        )
        .unwrap();
        for entry in row["observations"].as_array().unwrap() {
            let action = &entry["action"];
            match action[0].as_str().unwrap() {
                "failure" => assert_eq!(
                    json!(
                        controller
                            .note_failure(action[1].as_str().unwrap())
                            .unwrap()
                    ),
                    entry["result"]
                ),
                "mutation" => controller.note_mutation(),
                "success" => controller.note_success(),
                "initial" => (),
                other => panic!("unknown action {other}"),
            }
            assert_eq!(json!(controller.snapshot()), entry["snapshot"]);
            assert_eq!(json!(controller.evaluate_verifier()), entry["decision"]);
            assert_eq!(json!(controller.guidance()), entry["guidance"]);
            assert_eq!(json!(controller.next_repair_action()), entry["next"]);
        }
    }
    for row in oracle["decisions"].as_array().unwrap() {
        assert_eq!(
            json!(stop::decide(
                &serde_json::from_value(row["input"].clone()).unwrap()
            )),
            row["result"]
        );
    }
    for row in oracle["stops"].as_array().unwrap() {
        let mut controller = stop::Controller::new(row["stopAfterVerifierPass"].as_bool().unwrap());
        for entry in row["observations"].as_array().unwrap() {
            let action = &entry["action"];
            match action[0].as_str().unwrap() {
                "mutation" => controller.note_mutation(),
                "verify" => controller
                    .note_verification(action[1].as_bool().unwrap(), action[2].as_bool().unwrap()),
                "initial" => (),
                other => panic!("unknown action {other}"),
            }
            assert_eq!(json!(controller.snapshot()), entry["snapshot"]);
            assert_eq!(json!(controller.evaluate()), entry["decision"]);
        }
    }
}

#[test]
fn failure_diagnosis_limits_preserve_existing_repair_state_and_unicode() {
    use policy::{failure, repair};
    let mut controller = repair::Controller::new(vec![], vec!["src/value.js".into()], 2).unwrap();
    controller
        .note_failure("AssertionError: expected 1, got 2")
        .unwrap();
    let before = json!(controller.snapshot());
    assert!(matches!(
        controller.note_failure(&"x".repeat(4 * 1024 * 1024 + 1)),
        Err(DomainError::LimitExceeded(_))
    ));
    assert_eq!(json!(controller.snapshot()), before);
    let paths = (0..4097)
        .map(|i| format!("src/{i}.js"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(matches!(
        controller.note_failure(&paths),
        Err(DomainError::LimitExceeded(_))
    ));
    assert_eq!(json!(controller.snapshot()), before);
    // Do not emit the reference's lone UTF-16 surrogate when a preview ends
    // halfway through an emoji. The diagnostic stays within the same budget.
    let diagnostic = failure::classify(&format!("Error: {}😀 tail", "x".repeat(492)), &[]).unwrap();
    assert_eq!(diagnostic.summary, format!("Error: {}", "x".repeat(492)));
    assert!(diagnostic.summary.encode_utf16().count() <= 500);
    assert!(diagnostic.signature.encode_utf16().count() <= 240);
    assert!(repair::Controller::new(vec![], vec![], 0).is_err());
}

#[test]
fn controlled_orchestration_matches_source_decisions_and_guidance() {
    use policy::orchestrator::{Controller, Observation, Verification, mutation_tool};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/orchestrator-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 11);
    for row in oracle["rows"].as_array().unwrap() {
        let mut controller =
            Controller::new(serde_json::from_value(row["policy"].clone()).unwrap()).unwrap();
        for action in row["actions"].as_array().unwrap() {
            let input = &action["input"];
            let name = input["name"].as_str().unwrap();
            let args = &input["args"];
            let path = args["path"].as_str();
            if input["kind"] == "evaluate" {
                assert_eq!(
                    json!(controller.evaluate(name, args, path).unwrap()),
                    action["decision"],
                    "{} {input}",
                    row["id"]
                );
            } else {
                let error = input["isError"].as_bool().unwrap();
                let verified = name == "bash"
                    && args["command"]
                        .as_str()
                        .is_some_and(|c| controller.verifier_matches(c));
                controller
                    .observe(Observation {
                        name,
                        path,
                        is_error: error,
                        mutation_committed: mutation_tool(name) && !error,
                        verification: verified.then(|| Verification {
                            success: !error,
                            output: input["output"].as_str().unwrap(),
                        }),
                    })
                    .unwrap();
            }
            assert_eq!(
                json!(controller.guidance()),
                action["hint"],
                "{} {input}",
                row["id"]
            );
        }
    }
}

#[test]
fn controlled_verifier_identity_and_native_mutation_aliases_cannot_bypass_policy() {
    use policy::orchestrator::{Controller, Observation};
    let index =
        Index::captured(vec!["Cargo.toml".into()], BTreeMap::new(), &BTreeMap::new()).unwrap();
    let mut policy = policy::infer("Fix it", &index, Mode::StrictBenchmark).unwrap();
    policy.expected_changed_paths = vec!["src/value.rs".into()];
    let mut controller = Controller::new(policy.clone()).unwrap();
    for command in [
        "echo cargo test",
        "cargo test; echo passed",
        "cargo  test",
        "false || cargo test",
    ] {
        assert!(!controller.verifier_matches(command));
    }
    assert!(controller.verifier_matches("cargo test"));
    let decision = controller
        .evaluate("bash", &json!({"command":"cargo test"}), None)
        .unwrap();
    assert!(!decision.allow);
    controller
        .observe(Observation {
            name: "append",
            path: Some("src/value.rs"),
            is_error: false,
            mutation_committed: true,
            verification: None,
        })
        .unwrap();
    assert!(
        controller
            .evaluate("bash", &json!({"command":"cargo test"}), None)
            .unwrap()
            .allow
    );
    assert!(
        !controller
            .evaluate("editApply", &json!({}), Some("src/value.rs"))
            .unwrap()
            .allow
    );
    policy.mutation_expected = false;
    let controller = Controller::new(policy).unwrap();
    for name in ["write", "edit", "append", "editApply"] {
        assert!(
            !controller
                .evaluate(name, &json!({}), Some("src/value.rs"))
                .unwrap()
                .allow
        );
    }
}
