use super::*;
use ag1_platform::session::print::{self, Options};

fn prompt() -> TurnInput {
    let mut input = input();
    input.message["content"] = json!("Fix the value and verify the result.");
    input
}
fn write_value() -> Value {
    calls(vec![(
        "write",
        "write",
        json!({"path":"value.txt","content":"42"}),
    )])
}
fn verifier(f: &Fixture, extra: &str) {
    std::fs::write(f.root.join("verifier.test.js"),format!("const {{test}}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');test('value',()=>assert.equal(fs.readFileSync('value.txt','utf8'),'42'));{extra}\n")).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_canonical_receipt_recovers_false_completion_without_requiring_a_test_command_name()
{
    let (origin, server) = server(vec![
        stop(),
        write_value(),
        calls(vec![(
            "check",
            "bash",
            json!({"command":"node verify.mjs"}),
        )]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    std::fs::write(f.root.join("verify.mjs"), "import assert from 'node:assert/strict'; import fs from 'node:fs'; assert.equal(fs.readFileSync('value.txt','utf8'),'42'); console.log('value verifier passed');\n").unwrap();
    let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
    let mut input = prompt();
    input.max_generations = Some(4);
    let result = print::run(
        &f.host,
        input,
        &Resolver::new(false),
        runner,
        &ignore,
        Options {
            recommended_verifier: Some("node verify.mjs".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.completion_recoveries, 1);
    assert_eq!(result.turn.generations, 4);
    let assessment = result.assessment.unwrap();
    assert!(assessment.passed);
    assert_eq!(assessment.passing_verification_count, 1);
    assert_eq!(
        std::fs::read_to_string(f.root.join("value.txt")).unwrap(),
        "42"
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests[2]["messages"]
            .to_string()
            .contains("status: verification_required")
    );
    assert!(
        requests[3]["messages"]
            .to_string()
            .contains("status: passed_after_latest_observed_mutation")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_print_rejects_echoed_verifier_and_zero_test_success() {
    for command in [
        "echo 'node --test verifier.test.js'",
        "node --test --help",
        "node --test --test-isolation=none --test-name-pattern=absent verifier.test.js",
        "node --test --test-reporter=tap --test-isolation=none --test-name-pattern=absent verifier.test.js",
    ] {
        let (origin, server) = server(vec![
            write_value(),
            calls(vec![("check", "bash", json!({"command":command}))]),
            stop(),
            stop(),
            stop(),
            stop(),
        ]);
        let f = fixture(&origin, "disabled");
        verifier(&f, "");
        let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
        let result = print::run(
            &f.host,
            prompt(),
            &Resolver::new(false),
            runner,
            &ignore,
            Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(session_tool_results(&f)[1]["isError"], false);
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.assessment.unwrap().passing_verification_count, 0);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 6);
        assert!(
            !requests[2]["messages"]
                .to_string()
                .contains("status: passed_after_latest_observed_mutation")
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_print_reads_full_verifier_output_beyond_the_display_excerpt() {
    let (origin, server) = server(vec![
        write_value(),
        calls(vec![(
            "check",
            "bash",
            json!({"command":"node --test verifier.test.js"}),
        )]),
        stop(),
        stop(),
        stop(),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    verifier(
        &f,
        "console.log('a'.repeat(75000));console.log('1 failed');console.log('b'.repeat(75000));",
    );
    let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        Options::default(),
    )
    .await
    .unwrap();
    let outcomes = session_tool_results(&f);
    assert_eq!(outcomes[1]["isError"], false);
    assert!(
        !outcomes[1]["content"].to_string().contains("1 failed"),
        "failure deliberately outside display tail"
    );
    assert_eq!(result.exit_code, 1);
    assert!(!result.assessment.unwrap().passed);
    assert_eq!(server.join().unwrap().len(), 6);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_print_invalidates_verification_after_later_shell_mutation() {
    let (origin, server) = server(vec![
        write_value(),
        calls(vec![(
            "check",
            "bash",
            json!({"command":"node --test verifier.test.js"}),
        )]),
        calls(vec![(
            "later",
            "bash",
            json!({"command":"printf changed > value.txt"}),
        )]),
        stop(),
        stop(),
        stop(),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    verifier(&f, "");
    let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("value.txt")).unwrap(),
        "changed"
    );
    assert_eq!(result.exit_code, 1);
    assert_eq!(result.assessment.unwrap().passing_verification_count, 0);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 7);
    assert!(
        requests[2]["messages"]
            .to_string()
            .contains("status: passed_after_latest_observed_mutation")
    );
    assert!(
        requests[3]["messages"]
            .to_string()
            .contains("status: verification_required")
    );
    assert!(
        !requests[3]["messages"]
            .to_string()
            .contains("status: passed_after_latest_observed_mutation")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_print_can_verify_a_repository_created_after_initial_discovery() {
    let script = "const {test}=require('node:test');test('created',()=>{});";
    let (origin, server) = server(vec![
        calls(vec![(
            "create",
            "write",
            json!({"path":"created.test.js","content":script}),
        )]),
        calls(vec![(
            "check",
            "bash",
            json!({"command":"node --test created.test.js"}),
        )]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        Arc::new(ReturnedReceipts(runner)),
        &ignore,
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.assessment.unwrap().passed);
    assert_eq!(server.join().unwrap().len(), 3);
}

// A host runner may return durable P09 receipts without publishing parent
// messages itself. The common native fallback must collect evidence after ack.
struct ReturnedReceipts(Arc<dyn ag1_platform::session::ToolRunner>);
impl ag1_platform::session::ToolRunner for ReturnedReceipts {
    fn execute(
        &self,
        scope: &ag1_core::tools::registry::Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ag1_platform::session::ToolSelection,
        operation: &str,
        cancel: &AtomicBool,
        revision: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
        let detached = ag1_platform::session::ToolSelection {
            journal: None,
            names: selection.names.clone(),
            research_profile: selection.research_profile.clone(),
            loaded_memory: selection.loaded_memory.clone(),
        };
        self.0
            .execute(scope, calls, &detached, operation, cancel, revision)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn npm_version_header_does_not_reject_an_actual_passing_native_test() {
    let (origin, server) = server(vec![
        write_value(),
        calls(vec![(
            "check",
            "bash",
            json!({"command":"node --test verifier.test.js"}),
        )]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    verifier(&f, "console.log('> fixture@1.0.0 test');");
    let runner = native_adapters_configured(&f, &["write", "bash"], None, None).await;
    let result = print::run(
        &f.host,
        prompt(),
        &Resolver::new(false),
        runner,
        &ignore,
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.assessment.unwrap().passed);
    assert_eq!(server.join().unwrap().len(), 3);
}
