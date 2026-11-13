use super::*;

#[test]
fn failure_working_state_matches_eleven_unchanged_sdk_traces() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/failure-oracle.json"
    ))
    .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        let mut tracker = ag1_core::tools::recovery::FailureTracker::default();
        let mut outcomes = Vec::new();
        for step in row["steps"].as_array().unwrap() {
            match step["kind"].as_str().unwrap() {
                "reset" => tracker.reset(),
                "observe" => {
                    let guidance = tracker
                        .observe(
                            step["tool"].as_str().unwrap(),
                            step["error"].as_bool().unwrap(),
                            step["text"].as_str().unwrap(),
                        )
                        .unwrap();
                    outcomes.push(json!({"kind":"observe","guidance":guidance}));
                }
                "prompt" => {
                    let names =
                        serde_json::from_value::<Vec<String>>(step["active"].clone()).unwrap();
                    let (message, warnings) = match tracker.prepare_prompt(&names).unwrap() {
                        None => (Value::Null, json!([])),
                        Some(warning) => (
                            json!({"role":"custom","customType":"tool_failure_warning","content":warning.content,"display":true,"details":warning.details}),
                            json!(warning.warnings),
                        ),
                    };
                    outcomes.push(json!({"kind":"prompt","message":message,"warnings":warnings}));
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(json!(outcomes), row["outcomes"], "{}", row["id"]);
    }
}
fn missing(id: &str) -> Value {
    calls(vec![(
        id,
        "bash",
        json!({"command":"ag1_fixture_missing_command_for_recovery"}),
    )])
}
fn warnings(fixture: &Fixture) -> Vec<Value> {
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    ag1_core::task::branch_values(&journal)
        .unwrap()
        .into_iter()
        .filter(|entry| entry["message"]["customType"] == "tool_failure_warning")
        .collect()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_missing_shell_recovery_preserves_raw_results_and_warns_once_on_next_prompt() {
    use ag1_core::{context::features::Features, tools::executor::ExecutionArtifacts};
    let (origin, server) = server(vec![
        missing("one"),
        missing("two"),
        missing("three"),
        stop(),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_adapters(&fixture, &["bash"], None).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner.clone(), &ignore)
        .await
        .unwrap();
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 3);
    assert!(
        !results[0]["content"]
            .to_string()
            .contains("Shell recovery:")
    );
    let artifacts = ag1_storage::context::ContextStore::new(
        &fixture.store,
        SessionId::new("session").unwrap(),
        "session",
        "inspect-recovery",
        "recovery",
        Features::default(),
    )
    .unwrap();
    for result in &results[1..] {
        assert!(result["content"].to_string().contains("Shell recovery:"));
        let artifact =
            serde_json::from_value(result["details"]["nativeResultArtifact"].clone()).unwrap();
        let raw = artifacts.read_json(&artifact).unwrap();
        assert!(!raw.to_string().contains("Shell recovery:"));
        assert!(raw["content"].to_string().contains("command not found"));
    }
    assert!(warnings(&fixture).is_empty());
    let events = Mutex::new(Vec::new());
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(value) = event {
            events.lock().unwrap().push(value.clone());
        }
    };
    for command in ["next-user", "third-user"] {
        let mut next = input();
        next.command = command.into();
        fixture
            .host
            .run(next, &Resolver::new(false), runner.clone(), &listener)
            .await
            .unwrap();
    }
    assert_eq!(warnings(&fixture).len(), 1);
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["type"] == "tool_failure_warning")
            .count(),
        1
    );
    let requests = server.join().unwrap();
    assert!(
        requests[3]["messages"]
            .to_string()
            .contains("<tool_failure_warning>")
    );
    assert_eq!(requests[3]["messages"][0], requests[0]["messages"][0]);
    assert!(fixture.host.settled().unwrap());
    assert!(fixture.host.persistence_healthy().unwrap());
}

struct WarningFault(ag1_storage::Boundary);
impl ag1_storage::FaultInjector for WarningFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command.ends_with("-tool-failure-warning") && point == self.0 {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_warning_receipt_blocks_next_request_without_publishing_warning_event() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![missing("one"), missing("two"), missing("three")]);
        let fixture = fixture_seeded_with_fault(
            &origin,
            "disabled",
            vec![],
            Some(Arc::new(WarningFault(point))),
        );
        let runner = native_adapters(&fixture, &["bash"], None).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), runner.clone(), &ignore)
            .await
            .unwrap();
        let events = Mutex::new(Vec::new());
        let listener = |event: &SessionEvent| {
            if let SessionEvent::Recovery(value) = event {
                events.lock().unwrap().push(value.clone());
            }
        };
        let mut next = input();
        next.command = "next-user".into();
        assert!(
            fixture
                .host
                .run(next, &Resolver::new(false), runner, &listener)
                .await
                .is_err()
        );
        assert!(!fixture.host.persistence_healthy().unwrap());
        assert!(
            !events
                .lock()
                .unwrap()
                .iter()
                .any(|v| v["type"] == "tool_failure_warning")
        );
        assert_eq!(
            warnings(&fixture).len(),
            usize::from(point == ag1_storage::Boundary::BeforeAck)
        );
        assert_eq!(server.join().unwrap().len(), 3);
    }
}
