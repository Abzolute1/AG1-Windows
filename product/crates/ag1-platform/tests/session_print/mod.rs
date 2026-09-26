use super::*;
use ag1_platform::session::print::{self, Options};

fn oracle() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/print-oracle.json"
    ))
    .unwrap()
}
fn assert_source_guidance(fixture: &Fixture, case: &str, exit_code: u8) {
    let source = oracle();
    let row = source["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == case)
        .unwrap();
    assert_eq!(row["exitCode"], json!(exit_code));
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let actual: Vec<Value> = ag1_core::task::branch_values(&journal)
        .unwrap()
        .iter()
        .filter_map(|entry| {
            let message = &entry["message"];
            matches!(
                message["customType"].as_str(),
                Some("runtime_continuation_state" | "runtime_completion_gate")
            )
            .then(|| {
                let mut message = message.clone();
                let object = message.as_object_mut().unwrap();
                object.remove("role");
                object.remove("timestamp");
                message
            })
        })
        .collect();
    let expected: Vec<Value> = row["custom"]
        .as_array()
        .unwrap()
        .iter()
        .map(|custom| custom["message"].clone())
        .collect();
    assert_eq!(actual, expected, "{case}");
}
#[test]
fn print_mutation_partition_matches_seventeen_pinned_task_intents() {
    let source = oracle();
    let intents = source["intents"].as_array().unwrap();
    assert_eq!(intents.len(), 17);
    for row in intents {
        assert_eq!(
            ag1_core::runtime::completion::expects_mutation(row["prompt"].as_str().unwrap()),
            row["mutationExpected"].as_bool().unwrap(),
            "{row}"
        );
    }
}

fn prompt(text: &str) -> TurnInput {
    let mut input = input();
    input.message["content"] = json!(text);
    input
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_completion_uses_shared_source_intent_at_unicode_boundaries() {
    for (text, expected_generations, expected_exit) in [
        ("Do not\u{feff}edit files. Explain the fix.", 1, 0),
        ("修fix正", 4, 1),
    ] {
        let (origin, server) = server(vec![stop(); expected_generations]);
        let fixture = fixture(&origin, "disabled");
        let result = print::run(
            &fixture.host,
            prompt(text),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
            Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, expected_exit, "{text:?}");
        assert_eq!(result.turn.generations, expected_generations as u64);
        assert_eq!(result.assessment.is_some(), expected_exit == 1);
        assert_eq!(
            custom_count(&fixture, "runtime_completion_gate"),
            expected_generations - 1
        );
        assert_eq!(server.join().unwrap().len(), expected_generations);
        assert!(fixture.host.settled().unwrap());
    }
}
fn promise() -> Value {
    let mut reply = stop();
    reply["choices"][0]["delta"]["content"] = json!("Let me inspect it:");
    reply
}
fn custom_count(fixture: &Fixture, kind: &str) -> usize {
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    ag1_core::task::branch_values(&journal)
        .unwrap()
        .iter()
        .filter(|e| e["message"]["customType"] == kind)
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_completion_waits_for_actual_mutation_and_verifier_then_returns_success() {
    let (origin, server) = server(vec![
        stop(),
        calls(vec![(
            "answer",
            "write",
            json!({"path":"answer.txt","content":"42"}),
        )]),
        calls(vec![(
            "verify",
            "bash",
            json!({"command":"node --test verifier.test.js"}),
        )]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("verifier.test.js"),"const {test}=require('node:test'); const assert=require('node:assert/strict'); const fs=require('node:fs'); test('answer',()=>assert.equal(fs.readFileSync('answer.txt','utf8'),'42'));\n").unwrap();
    // V8 reserves a large virtual address range before running this tiny test.
    // Keep normal process/output/deadline custody; this fixture needs no AS cap.
    let runner = native_adapters_configured(&fixture, &["write", "bash"], None, None).await;
    let result = print::run(
        &fixture.host,
        prompt("Fix the output file and run tests."),
        &Resolver::new(false),
        runner,
        &ignore,
        Options::default(),
    )
    .await
    .unwrap_or_else(|error| panic!("{error:?}: {}", json!(session_tool_results(&fixture))));
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.turn.generations, 4);
    assert_eq!(result.completion_recoveries, 1);
    let assessment = result.assessment.unwrap();
    assert!(assessment.passed);
    assert_eq!(assessment.passing_verification_count, 1);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("answer.txt")).unwrap(),
        "42"
    );
    assert_eq!(custom_count(&fixture, "runtime_completion_gate"), 1);
    let requests = server.join().unwrap();
    assert!(requests[1]["messages"].to_string().contains(
        "canonical repository verifier exactly as recorded: node --test verifier.test.js"
    ));
    assert_eq!(requests[0]["messages"][0], requests[1]["messages"][0]);
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("<runtime-verification-contract>")
    );
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("node --test verifier.test.js")
    );
    assert!(fixture.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_unverified_coding_task_exhausts_three_recoveries_and_reports_failure() {
    let (origin, server) = server(vec![stop(), stop(), stop(), stop()]);
    let fixture = fixture(&origin, "disabled");
    let result = print::run(
        &fixture.host,
        prompt("Implement the requested change."),
        &Resolver::new(false),
        fixture.files.clone(),
        &ignore,
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.completion_recoveries, 3);
    assert_eq!(result.turn.generations, 4);
    assert!(!result.assessment.unwrap().passed);
    assert_eq!(custom_count(&fixture, "runtime_completion_gate"), 3);
    assert_source_guidance(&fixture, "unverified", result.exit_code);
    assert_eq!(server.join().unwrap().len(), 4);
    assert!(fixture.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_recovers_two_action_promises_and_preserves_non_mutating_intent() {
    let (origin, server) = server(vec![promise(), promise(), stop()]);
    let fixture = fixture(&origin, "disabled");
    let result = print::run(
        &fixture.host,
        prompt("Explain how to fix this; do not modify code files."),
        &Resolver::new(false),
        fixture.files.clone(),
        &ignore,
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.continuation_recoveries, 2);
    assert_eq!(result.completion_recoveries, 0);
    assert!(result.assessment.is_none());
    assert_eq!(custom_count(&fixture, "runtime_continuation_state"), 2);
    assert_source_guidance(&fixture, "promises", result.exit_code);
    let requests = server.join().unwrap();
    assert!(
        requests[2]["messages"]
            .to_string()
            .contains("Repeated planning-only continuations are not allowed")
    );
    assert!(fixture.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_cancel_between_terminal_and_recovery_prevents_another_request() {
    let (origin, server) = server(vec![promise()]);
    let fixture = fixture(&origin, "disabled");
    let observed = AtomicBool::new(false);
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(event) = event
            && event["type"] == "mode_recovery_preparing"
        {
            observed.store(!fixture.host.settled().unwrap(), Ordering::Release);
            fixture.host.cancel().unwrap();
        }
    };
    let result = print::run(
        &fixture.host,
        prompt("Explain the source."),
        &Resolver::new(false),
        fixture.files.clone(),
        &listener,
        Options::default(),
    )
    .await;
    assert!(matches!(result, Err(DomainError::Cancelled)));
    assert!(observed.load(Ordering::Acquire));
    assert_eq!(custom_count(&fixture, "runtime_continuation_state"), 0);
    assert_eq!(server.join().unwrap().len(), 1);
    assert!(fixture.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_owns_prompt_admission_between_followups() {
    let (origin, server) = server(vec![promise(), stop()]);
    let fixture = fixture(&origin, "disabled");
    let rejected = Mutex::new(None);
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(event) = event
            && event["type"] == "mode_recovery_preparing"
        {
            let resolver = Resolver::new(false);
            let mut input = prompt("Unrelated prompt");
            input.command = "unrelated".into();
            let mut run = Box::pin(fixture.host.run(
                input,
                &resolver,
                fixture.files.clone(),
                &ignore,
            ));
            // Admission rejects before I/O. Poll directly so a blocking fixture
            // callback cannot starve a spawned task in Tokio's local queue.
            if let std::task::Poll::Ready(result) = run
                .as_mut()
                .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
            {
                *rejected.lock().unwrap() = result.err();
            }
        }
    };
    let result = print::run(
        &fixture.host,
        prompt("Explain the source."),
        &Resolver::new(false),
        fixture.files.clone(),
        &listener,
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(matches!(
        *rejected.lock().unwrap(),
        Some(DomainError::Unavailable(_))
    ));
    assert!(fixture.store.command_result("unrelated").unwrap().is_none());
    assert_eq!(server.join().unwrap().len(), 2);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_recovery_preserves_the_explicit_callers_generation_budget() {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture(&origin, "disabled");
    let mut input = prompt("Implement the requested change.");
    input.max_generations = Some(1);
    let result = print::run(
        &fixture.host,
        input,
        &Resolver::new(false),
        fixture.files.clone(),
        &ignore,
        Options::default(),
    )
    .await;
    assert!(matches!(result, Err(DomainError::LimitExceeded(_))));
    assert_eq!(custom_count(&fixture, "runtime_completion_gate"), 0);
    assert_eq!(server.join().unwrap().len(), 1);
    assert!(fixture.host.settled().unwrap());
}

struct FinalReceiptGate {
    entered: Notify,
    accepted: std::sync::mpsc::Sender<()>,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}
impl FinalReceiptGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
impl ag1_storage::FaultInjector for FinalReceiptGate {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command == "print-control" && point == ag1_storage::Boundary::BeforeAck {
            self.accepted.send(()).unwrap();
            self.entered.notify_one();
            let (released, _) = self
                .changed
                .wait_timeout_while(
                    self.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(*released, "release print control acknowledgement");
        }
        Ok(())
    }
}
async fn print_final_control_receipt(abandon: bool) {
    let (origin, server) = server(vec![stop()]);
    let (accepted, receive) = std::sync::mpsc::channel();
    let gate = Arc::new(FinalReceiptGate {
        entered: Notify::new(),
        accepted,
        released: Mutex::new(false),
        changed: std::sync::Condvar::new(),
    });
    let fixture = fixture_configured(&origin, "disabled", vec![], Some(gate.clone()), None, 1);
    let control: Arc<Mutex<Option<ag1_core::PortFuture<'static, ()>>>> = Arc::new(Mutex::new(None));
    let started = Arc::new(AtomicBool::new(false));
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let model = fixture.model.clone();
    let handle = control.clone();
    let observed = started.clone();
    let receive = Mutex::new(receive);
    let mut run = tokio::spawn(async move {
        let listener = |event: &SessionEvent| {
            if matches!(event, SessionEvent::Committed { revision: 3, .. }) {
                let host = host.clone();
                let mut model = model.clone();
                model["id"] = json!("final-control");
                let mut control: ag1_core::PortFuture<'static, ()> = Box::pin(async move {
                    host.select_model("print-control", "print-control", NOW, model, &ignore)
                        .await
                });
                let pending = control
                    .as_mut()
                    .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
                    .is_pending();
                *handle.lock().unwrap() = Some(control);
                // Force the accepted append into the exact terminal callback gap.
                observed.store(
                    pending
                        && receive
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .is_ok(),
                    Ordering::Release,
                );
            }
        };
        print::run(
            &host,
            prompt("Explain the source."),
            &Resolver::new(false),
            files,
            &listener,
            Options::default(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), gate.entered.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(25), &mut run)
            .await
            .is_err()
    );
    assert!(started.load(Ordering::Acquire));
    assert!(!fixture.host.settled().unwrap());
    let control = control.lock().unwrap().take().unwrap();
    if abandon {
        drop(control);
        gate.release();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), run)
                .await
                .unwrap()
                .unwrap(),
            Err(DomainError::DurabilityUnknown)
        ));
        assert!(!fixture.host.persistence_healthy().unwrap());
    } else {
        gate.release();
        control.await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.exit_code, 0);
        assert_eq!(fixture.host.journal_revision().unwrap(), 4);
        assert!(fixture.host.persistence_healthy().unwrap());
        assert!(fixture.host.settled().unwrap());
    }
    assert_eq!(server.join().unwrap().len(), 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn print_finalization_waits_for_an_accepted_control_receipt() {
    print_final_control_receipt(false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn print_finalization_rejects_an_abandoned_control_receipt() {
    print_final_control_receipt(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_verifier_contract_is_request_local_and_expires_with_the_mode() {
    let (origin, server) = server(vec![stop(), stop(), stop(), stop(), stop()]);
    let fixture = fixture(&origin, "disabled");
    let result = print::run(
        &fixture.host,
        prompt("Fix the output."),
        &Resolver::new(false),
        fixture.files.clone(),
        &ignore,
        Options {
            recommended_verifier: Some("node --test output.test.js".into()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 1);
    let mut next = prompt("Explain the output.");
    next.command = "next-user".into();
    fixture
        .host
        .run(next, &Resolver::new(false), fixture.files.clone(), &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    let contract =
        ag1_core::context::projections::turn(ag1_core::context::projections::TurnAddenda {
            verification_command: Some("node --test output.test.js"),
            ..Default::default()
        })
        .unwrap();
    for request in &requests[..4] {
        assert!(
            request["messages"].as_array().unwrap().last().map(|message| ag1_core::context::hygiene::content(message, "\n")).unwrap()
                .contains(&contract)
        );
    }
    assert!(
        !requests[4]["messages"]
            .to_string()
            .contains("<runtime-verification-contract>")
    );
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    assert!(
        !json!(branch)
            .to_string()
            .contains("<runtime-verification-contract>")
    );
    let users: Vec<&Value> = branch
        .iter()
        .filter(|entry| entry["message"]["role"] == "user")
        .collect();
    assert_eq!(users[0]["message"]["content"], "Fix the output.");
    assert_eq!(users[1]["message"]["content"], "Explain the output.");
}
