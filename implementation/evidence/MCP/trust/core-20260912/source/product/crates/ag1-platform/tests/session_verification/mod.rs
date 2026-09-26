use super::*;
use ag1_platform::session::{print, verification};

#[test]
fn generated_verifiers_preserve_literal_paths_and_do_not_interpret_shell_syntax() {
    use ag1_core::runtime::verification::Index;
    for (path, expected) in [
        (
            "tests/two  spaces.test.js",
            "node --test 'tests/two  spaces.test.js'",
        ),
        (
            "tests/name's.test.js",
            "node --test 'tests/name'\"'\"'s.test.js'",
        ),
        (
            "tests/$(touch injected).test.js",
            "node --test 'tests/$(touch injected).test.js'",
        ),
        ("--flag.test.js", "node --test ./--flag.test.js"),
        ("my  project/Cargo.toml", "cd 'my  project' && cargo test"),
    ] {
        let index =
            Index::captured(vec![path.into()], Default::default(), &Default::default()).unwrap();
        assert_eq!(
            index.verifier("Fix the code.").unwrap().as_deref(),
            Some(expected)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovered_quoted_node_test_runs_as_one_literal_path_through_normal_tools() {
    let command = "node --test 'tests/space ; touch injected ; value.test.js'";
    let (origin, server) = server(vec![
        calls(vec![(
            "write",
            "write",
            json!({"path":"answer.txt","content":"42"}),
        )]),
        calls(vec![("verify", "bash", json!({"command":command}))]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::create_dir(fixture.root.join("tests")).unwrap();
    std::fs::write(fixture.root.join("tests/space ; touch injected ; value.test.js"),"const {test}=require('node:test'); const assert=require('node:assert/strict'); const fs=require('node:fs'); test('answer',()=>assert.equal(fs.readFileSync('answer.txt','utf8'),'42'));\n").unwrap();
    let runner = native_adapters_configured(&fixture, &["write", "bash"], None, None).await;
    let mut input = input();
    input.message["content"] = json!("Fix the output and run the tests.");
    let result = print::run(
        &fixture.host,
        input,
        &Resolver::new(false),
        runner,
        &ignore,
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 0);
    assert!(result.assessment.unwrap().passed);
    assert_eq!(result.turn.generations, 3);
    assert!(!fixture.root.join("injected").exists());
    let requests = server.join().unwrap();
    assert!(
        requests[0]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains(command)
    );
}

#[test]
fn verifier_discovery_matches_pinned_workspace_indexes_and_commands() {
    let source: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/verifier-oracle.json"
    ))
    .unwrap();
    assert_eq!(source["rows"].as_array().unwrap().len(), 29);
    for row in source["rows"].as_array().unwrap() {
        let directory = tempfile::tempdir().unwrap();
        let input = &row["input"];
        let mut files = input["files"].as_object().unwrap().clone();
        for i in 0..input["bulk"].as_u64().unwrap_or(0) {
            files.insert(format!("aaa/file-{i:04}.txt"), json!(""));
        }
        for i in 0..input["testBulk"].as_u64().unwrap_or(0) {
            files.insert(
                format!("test/{i:02}.test.js"),
                json!("import '../src/target.js';"),
            );
        }
        for (name, bytes) in files {
            let path = directory.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes.as_str().unwrap()).unwrap();
        }
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(directory.path())
            .unwrap();
        let index = verification::capture(&workspace, &AtomicBool::new(false)).unwrap();
        assert_eq!(json!(index), row["index"], "{}", input["id"]);
        assert_eq!(
            json!(index.verifier(input["prompt"].as_str().unwrap()).unwrap()),
            row["verifier"],
            "{}",
            input["id"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_discovers_fresh_verifier_after_build_metadata_changes() {
    let (origin, server) = server(vec![stop(); 8]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("Cargo.toml"), "").unwrap();
    let mut first = input();
    first.message["content"] = json!("Fix the implementation.");
    let result = print::run(
        &fixture.host,
        first,
        &Resolver::new(false),
        fixture.files.clone(),
        &ignore,
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 1);
    std::fs::remove_file(fixture.root.join("Cargo.toml")).unwrap();
    std::fs::write(fixture.root.join("pyproject.toml"), "").unwrap();
    let mut second = input();
    second.command = "next-build-system".into();
    second.message["content"] = json!("Fix the next implementation.");
    print::run(
        &fixture.host,
        second,
        &Resolver::new(false),
        fixture.files.clone(),
        &ignore,
        Default::default(),
    )
    .await
    .unwrap();
    let requests = server.join().unwrap();
    for (request, expected) in requests
        .iter()
        .zip(["cargo test"; 4].into_iter().chain(["pytest"; 4]))
    {
        let contract =
            ag1_core::context::projections::turn(ag1_core::context::projections::TurnAddenda {
                verification_command: Some(expected),
                ..Default::default()
            })
            .unwrap();
        assert!(
            request["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains(&contract)
        );
    }
    assert!(
        !requests[4]["messages"][0]
            .to_string()
            .contains("cargo test")
    );
}

#[test]
fn verifier_discovery_does_not_follow_links_open_devices_or_return_partial_oversize_indexes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let outside = directory.path().join("outside");
    std::fs::write(&outside, "private marker").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("Cargo.toml")).unwrap();
    let status = std::process::Command::new("/usr/bin/mkfifo")
        .arg(root.join("pyproject.toml"))
        .status()
        .unwrap();
    assert!(status.success());
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let index = verification::capture(&workspace, &AtomicBool::new(false)).unwrap();
    assert!(index.files.is_empty());
    assert_eq!(index.verifier("Fix it").unwrap(), None);
    assert!(matches!(
        verification::capture(&workspace, &AtomicBool::new(true)),
        Err(DomainError::Cancelled)
    ));
    std::fs::write(root.join("huge.js"), vec![b'a'; 2 * 1024 * 1024 + 1]).unwrap();
    assert!(matches!(
        verification::capture(&workspace, &AtomicBool::new(false)),
        Err(DomainError::LimitExceeded(_))
    ));
    std::fs::remove_file(root.join("huge.js")).unwrap();
    std::fs::write(root.join("package.json"), vec![b' '; 1024 * 1024 + 1]).unwrap();
    assert!(matches!(
        verification::capture(&workspace, &AtomicBool::new(false)),
        Err(DomainError::LimitExceeded(_))
    ));
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "private marker");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unavailable_verifier_discovery_keeps_completion_evidence_required() {
    let (origin, server) = server(vec![stop(); 4]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(
        fixture.root.join("huge.js"),
        vec![b'a'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    let mut input = input();
    input.message["content"] = json!("Fix the implementation.");
    let failed = AtomicBool::new(false);
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(event) = event
            && event["type"] == "verifier_discovery_unavailable"
        {
            failed.store(true, Ordering::Release);
        }
    };
    let result = print::run(
        &fixture.host,
        input,
        &Resolver::new(false),
        fixture.files.clone(),
        &listener,
        Default::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_code, 1);
    assert!(!result.assessment.unwrap().passed);
    assert!(failed.load(Ordering::Acquire));
    let requests = server.join().unwrap();
    assert!(
        !requests[0]["messages"][0]
            .to_string()
            .contains("runtime-verification-contract")
    );
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("Discover and run the repository's canonical verifier")
    );
}
