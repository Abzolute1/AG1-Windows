use ag1_core::runtime::completion::{self, Ledger, VerificationAuthority};
use serde_json::{Value, json};

#[test]
fn completion_classifiers_and_lineage_match_eighty_seven_unchanged_sdk_cases() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/completion-oracle.json"
    ))
    .unwrap();
    for row in oracle["commands"].as_array().unwrap() {
        let command = row["command"].as_str().unwrap();
        assert_eq!(
            completion::is_test_command(command),
            row["recognized"],
            "{command}"
        );
        assert_eq!(
            completion::normalize_test_command(command),
            row["normalized"],
            "{command}"
        );
    }
    for row in oracle["classifications"].as_array().unwrap() {
        let input = &row["input"];
        let actual = completion::classify(
            input["command"].as_str().unwrap(),
            input["output"].as_str().unwrap(),
            input["isError"].as_bool().unwrap(),
        )
        .unwrap();
        assert_eq!(json!(actual), row["result"], "{input}");
    }
    for row in oracle["continuations"].as_array().unwrap() {
        assert_eq!(
            completion::needs_continuation(&row["message"]),
            row["result"],
            "{}",
            row["message"]
        );
    }
    for row in oracle["assessments"].as_array().unwrap() {
        let input = &row["input"];
        let untracked = input
            .get("untrackedPaths")
            .map(|v| serde_json::from_value::<Vec<String>>(v.clone()).unwrap())
            .unwrap_or_default();
        let modified = input
            .get("modifiedTestPaths")
            .map(|v| serde_json::from_value::<Vec<String>>(v.clone()).unwrap())
            .unwrap_or_default();
        let authority = if input["verificationAuthority"] == "typed-ledger" {
            VerificationAuthority::TypedLedger
        } else {
            VerificationAuthority::MessageHistory
        };
        let actual = completion::assess(
            input["messages"].as_array().unwrap(),
            &untracked,
            &modified,
            authority,
        )
        .unwrap();
        assert_eq!(json!(actual), row["result"], "{input}");
    }
    for row in oracle["ledgers"].as_array().unwrap() {
        let mut ledger = Ledger::default();
        let mut outcomes = Vec::new();
        for step in row["steps"].as_array().unwrap() {
            let patch = step["patchSha256"].as_str().unwrap();
            match step["kind"].as_str().unwrap() {
                "candidate" => ledger.note_candidate(patch).unwrap(),
                "verify" => {
                    let result = ledger
                        .note_verification(
                            patch,
                            step["command"].as_str().unwrap(),
                            step["output"].as_str().unwrap(),
                            step["isError"].as_bool().unwrap(),
                            step["baselineProbe"].as_bool(),
                        )
                        .unwrap();
                    outcomes.push(json!({"kind":"verify","result":result}));
                }
                "assess" => outcomes.push(json!({"kind":"assess","result":ledger.assess(patch)})),
                _ => unreachable!(),
            }
        }
        assert_eq!(json!(outcomes), row["outcomes"], "{}", row["id"]);
    }
}

#[test]
fn full_verification_working_state_keeps_prior_evidence_and_rejects_atomically() {
    let mut ledger = Ledger::default();
    let command = format!("pytest {}", "x".repeat(65000));
    let mut accepted = 0;
    for _ in 0..300 {
        match ledger.note_verification(
            "candidate",
            &command,
            "FAILED test_a::bad\n1 failed",
            true,
            None,
        ) {
            Ok(_) => accepted += 1,
            Err(ag1_core::DomainError::LimitExceeded(_)) => break,
            Err(error) => panic!("{error:?}"),
        }
    }
    assert!(accepted > 0 && accepted < 300);
    let before = ledger.assess("candidate");
    assert!(!before.passed);
    assert_eq!(before.observations.len(), accepted);
    let sequence = before.observations.last().unwrap().sequence;
    assert!(
        ledger
            .note_verification("unaccepted-child", &command, "1 passed", false, None)
            .is_err()
    );
    let after = ledger.assess("candidate");
    assert_eq!(after.observations.len(), accepted);
    assert_eq!(after.observations.last().unwrap().sequence, sequence);
    assert_eq!(after.unresolved_failures.len(), 1);
    assert!(!after.passed);
    assert!(
        completion::classify(
            "pytest tests",
            &format!("FAILED {}", "x".repeat(4097)),
            true
        )
        .is_err()
    );
}
