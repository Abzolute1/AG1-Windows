use super::*;
use ag1_core::runtime::policy::{self, Mode, TaskPolicy};
use ag1_platform::session::print::{self, Options};

fn policy(mode: Mode, mutation: bool, verifier: Option<&str>) -> TaskPolicy {
    TaskPolicy {
        mode,
        intent: if mutation {
            policy::Intent::DebugFix
        } else {
            policy::Intent::CodeExplanation
        },
        mutation_expected: mutation,
        web_relevant: false,
        allowed_paths: vec!["value.txt".into(), "tests/value.test.js".into()],
        expected_changed_paths: if mutation {
            vec!["value.txt".into()]
        } else {
            vec![]
        },
        likely_tests: vec!["tests/value.test.js".into()],
        verification: verifier.map(str::to_owned),
        max_reads_before_patch: 4,
        max_searches_before_patch: 1,
        max_patch_attempts: 2,
        stop_after_verifier_pass: verifier.is_some(),
    }
}
fn prompt() -> TurnInput {
    let mut input = input();
    input.message["content"] = json!("Explain the fixture result.");
    input
}
fn options(policy: TaskPolicy) -> Options {
    Options {
        controlled_policy: Some(policy),
        ..Options::default()
    }
}
fn write_call(id: &str, value: &str) -> Value {
    calls(vec![(
        id,
        "write",
        json!({"path":"value.txt","content":value}),
    )])
}
fn node_test(f: &Fixture) {
    std::fs::create_dir(f.root.join("tests")).unwrap();
    std::fs::write(f.root.join("tests/value.test.js"),"const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');test('value',()=>assert.equal(fs.readFileSync('value.txt','utf8'),'42'));\n").unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_policy_observes_acknowledged_mutation_and_full_verifier_then_expires() {
    let (origin, server) = server(vec![
        calls(vec![
            ("first", "write", json!({"path":"value.txt","content":"42"})),
            (
                "repeat",
                "write",
                json!({"path":"value.txt","content":"wrong"}),
            ),
        ]),
        calls(vec![(
            "verify",
            "bash",
            json!({"command":"node --test tests/value.test.js"}),
        )]),
        calls(vec![("after-pass", "read", json!({"path":"value.txt"}))]),
        stop(),
        calls(vec![("ordinary", "read", json!({"path":"value.txt"}))]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    node_test(&f);
    let runner = native_adapters_configured(&f, &["read", "write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner.clone(),
        &ignore,
        options({
            let mut selected = policy(Mode::StrictBenchmark, true, None);
            selected.stop_after_verifier_pass = true;
            selected
        }),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.turn.generations, 4);
    assert!(result.assessment.unwrap().passed);
    let outcomes = session_tool_results(&f);
    assert_eq!(outcomes.len(), 4);
    assert_eq!(outcomes[0]["isError"], false);
    assert_eq!(outcomes[1]["isError"], true);
    assert_eq!(outcomes[2]["isError"], false);
    assert_eq!(outcomes[3]["isError"], true);
    assert!(outcomes[1].to_string().contains("already edited"));
    assert!(outcomes[3].to_string().contains("verifier already passed"));
    assert_eq!(
        std::fs::read_to_string(f.root.join("value.txt")).unwrap(),
        "42"
    );
    let mut ordinary = prompt();
    ordinary.command = "ordinary-prompt".into();
    f.host
        .run(ordinary, &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(
        requests[0]["messages"][0]
            .to_string()
            .contains("Controlled execution policy")
    );
    assert!(
        requests[2]["messages"][0]
            .to_string()
            .contains("next required action: final_answer")
    );
    assert!(
        !requests[4]["messages"][0]
            .to_string()
            .contains("Controlled execution policy")
    );
    assert!(f.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_policy_does_not_confuse_echoed_verifier_with_execution() {
    let (origin, server) = server(vec![
        write_call("first", "bad"),
        calls(vec![(
            "echo",
            "bash",
            json!({"command":"echo 'node --test tests/value.test.js'"}),
        )]),
        calls(vec![(
            "read-after-echo",
            "read",
            json!({"path":"value.txt"}),
        )]),
        stop(),
        stop(),
        stop(),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    node_test(&f);
    let runner = native_adapters_configured(&f, &["read", "write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        options(policy(
            Mode::Controlled,
            true,
            Some("node --test tests/value.test.js"),
        )),
    )
    .await
    .unwrap();
    let outcomes = session_tool_results(&f);
    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[2]["isError"], false);
    assert_eq!(result.exit_code, 1);
    assert!(!result.assessment.unwrap().passed);
    assert_eq!(server.join().unwrap().len(), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_nonmutating_policy_blocks_native_mutation_aliases_before_preparation() {
    let (origin, server) = server(vec![
        calls(vec![
            (
                "append",
                "append",
                json!({"path":"value.txt","content":"changed"}),
            ),
            ("apply", "editApply", json!({"path":"value.txt","edits":[{"oldText":"original","newText":"changed"}]})),
            (
                "write",
                "write",
                json!({"path":"value.txt","content":"changed"}),
            ),
        ]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    std::fs::write(f.root.join("value.txt"), "original").unwrap();
    let runner = native_adapters(&f, &["append", "editApply", "write"], None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        options(policy(Mode::Controlled, false, None)),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        std::fs::read_to_string(f.root.join("value.txt")).unwrap(),
        "original"
    );
    for outcome in session_tool_results(&f) {
        assert_eq!(outcome["isError"], true);
        assert!(outcome.to_string().contains("classified as non-mutating"));
    }
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_batch_serialization_uses_selected_model_capability() {
    for window in [15000, 90000] {
        let (origin, server) = server(vec![
            calls(vec![
                ("first", "read", json!({"path":"value.txt","limit":1})),
                ("second", "read", json!({"path":"value.txt","limit":2})),
            ]),
            stop(),
        ]);
        let f = fixture(&origin, "disabled");
        std::fs::write(f.root.join("value.txt"), "one\ntwo\n").unwrap();
        let mut model = f.model.clone();
        model["contextWindow"] = json!(window);
        f.host
            .select_model("model", "model", NOW, model, &ignore)
            .await
            .unwrap();
        let result = print::run(
            &f.host,
            prompt(),
            &Resolver::new(false),
            f.files.clone(),
            &ignore,
            options(policy(Mode::Flexible, false, None)),
        )
        .await
        .unwrap();
        assert_eq!(result.exit_code, 0);
        let outcomes = session_tool_results(&f);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0]["isError"], false);
        assert_eq!(outcomes[1]["isError"], window < 16384);
        let requests = server.join().unwrap();
        assert_eq!(
            requests[0]["messages"][0]
                .to_string()
                .contains("emit exactly one tool call"),
            window < 16384
        );
    }
}

struct ResultFault(ag1_storage::Boundary);
impl ag1_storage::FaultInjector for ResultFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command.contains("-result-") && !command.ends_with("-inspection") && point == self.0 {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_result_ack_failure_prevents_later_effects_and_inference() {
    for boundary in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![calls(vec![
            ("first", "write", json!({"path":"value.txt","content":"42"})),
            (
                "later",
                "write",
                json!({"path":"other.txt","content":"never"}),
            ),
        ])]);
        let f = fixture_seeded_with_fault(
            &origin,
            "disabled",
            vec![],
            Some(Arc::new(ResultFault(boundary))),
        );
        let mut selected = policy(Mode::Controlled, true, None);
        selected.expected_changed_paths.push("other.txt".into());
        let result = print::run(
            &f.host,
            prompt(),
            &Resolver::new(false),
            f.files.clone(),
            &ignore,
            options(selected),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(f.root.join("value.txt")).unwrap(),
            "42"
        );
        assert!(!f.root.join("other.txt").exists());
        assert!(!f.host.persistence_healthy().unwrap());
        assert_eq!(server.join().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_verifier_exit_zero_with_no_tests_does_not_close_tools_or_complete() {
    let command = "node --test --test-reporter=tap --test-isolation=none --test-name-pattern=absent tests/value.test.js";
    let (origin, server) = server(vec![
        write_call("first", "42"),
        calls(vec![("empty-tests", "bash", json!({"command":command}))]),
        calls(vec![("after-empty", "read", json!({"path":"value.txt"}))]),
        stop(),
        stop(),
        stop(),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    node_test(&f);
    let runner = native_adapters_configured(&f, &["read", "write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        options(policy(Mode::Controlled, true, Some(command))),
    )
    .await
    .unwrap();
    let outcomes = session_tool_results(&f);
    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[1]["isError"], false);
    assert!(outcomes[1].to_string().contains("# tests 0"));
    assert_eq!(outcomes[2]["isError"], false);
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.assessment.unwrap().passing_verification_count, 0);
    assert_eq!(server.join().unwrap().len(), 7);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_verification_is_invalidated_by_a_later_shell_effect() {
    let command = "node --test tests/value.test.js";
    let (origin, server) = server(vec![
        write_call("first", "42"),
        calls(vec![("verify", "bash", json!({"command":command}))]),
        calls(vec![(
            "later-change",
            "bash",
            json!({"command":"printf changed > value.txt"}),
        )]),
        stop(),
        stop(),
        stop(),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    node_test(&f);
    let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
    let mut selected = policy(Mode::Controlled, true, Some(command));
    selected.stop_after_verifier_pass = false;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        options(selected),
    )
    .await
    .unwrap();
    let outcomes = session_tool_results(&f);
    assert_eq!(outcomes.len(), 3);
    assert_eq!(outcomes[1]["isError"], false);
    assert_eq!(outcomes[2]["isError"], false);
    assert_eq!(
        std::fs::read_to_string(f.root.join("value.txt")).unwrap(),
        "changed"
    );
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.assessment.unwrap().passing_verification_count, 0);
    assert_eq!(server.join().unwrap().len(), 7);
}
