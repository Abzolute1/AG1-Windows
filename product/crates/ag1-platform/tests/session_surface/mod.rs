use super::*;
use ag1_platform::{
    artifact_tools::ArtifactTools, observation_tools::ObservationTools, process::ProcessSupervisor,
    search_tools::SearchService, session::adapters::NativeAdapters, task_tools::TaskTools,
};
async fn runner(f: &Fixture, names: &[&str]) -> Arc<FileRunner> {
    let spools = f._directory.path().join("surface-spools");
    std::fs::create_dir(&spools).unwrap();
    let processes =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let adapters = NativeAdapters {
        artifact: Some(Arc::new(ArtifactTools::new(
            f.workspace.clone(),
            f.store.clone(),
        ))),
        task: Some(Arc::new(TaskTools::new(f.workspace.clone()))),
        observation: Some(Arc::new(ObservationTools::new(f.workspace.clone()))),
        search: Some(Arc::new(
            SearchService::new(
                f.workspace.clone(),
                processes,
                "/usr/bin/rg".as_ref(),
                "/usr/bin/fd".as_ref(),
                &spools,
            )
            .unwrap(),
        )),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    let names: Vec<_> = names.iter().map(|n| n.to_string()).collect();
    registry
        .activate(&names, &adapters.available_names().into_iter().collect())
        .unwrap();
    let tools = registry
        .native_tools()
        .into_iter()
        .filter(|t| names.iter().any(|n| t["name"] == *n))
        .collect();
    f.host
        .select_tools("surface-tools", "surface-tools", NOW, tools, &ignore)
        .await
        .unwrap();
    Arc::new(FileRunner {
        adapters,
        registry: Arc::new(registry),
        ..(*f.files).clone()
    })
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preferred_calls_preserve_transcript_identity_and_project_one_observation() {
    let art = "art_00000000000000000000000000000009";
    let (origin, server) = server(vec![
        calls(vec![(
            "inspect-source",
            "inspect",
            json!({"path":"note.txt"}),
        )]),
        calls(vec![(
            "inspect-context",
            "inspect",
            json!({"query":"NEEDLE","path":"note.txt"}),
        )]),
        calls(vec![(
            "work-reset",
            "work",
            json!({"action":"reset","objective":"Verify owned fixture"}),
        )]),
        calls(vec![(
            "work-record",
            "work",
            json!({"action":"record","kind":"DECISION","statement":"Use the owned fixture","confidence":"high"}),
        )]),
        calls(vec![(
            "work-observe",
            "work",
            json!({"action":"observe","observation":{"feature":"Unrelated wording","result":"pass","environment":"owned fixture","revision":"workspace:fixture","reporter":"agent","reproduction":"Inspect source","notes":"Observed expected text"}}),
        )]),
        calls(vec![("work-show", "work", json!({"action":"show"}))]),
        calls(vec![(
            "evidence-read",
            "evidence",
            json!({"id":art,"maxBytes":2048}),
        )]),
        calls(vec![(
            "retain",
            "retainEvidence",
            json!({"id":art,"reason":"needed for check","verificationScope":"owned-check"}),
        )]),
        calls(vec![("release", "releaseEvidence", json!({"id":art}))]),
        calls(vec![(
            "permissions",
            "capability",
            json!({"action":"list"}),
        )]),
        calls(vec![(
            "schema",
            "capability",
            json!({"action":"schema","name":"editApply","arguments":{"path":"note.txt","edits":[{"oldText":"NEEDLE","newText":"changed"}]}}),
        )]),
        calls(vec![(
            "specialist",
            "capability",
            json!({"action":"run","name":"artifact","arguments":{"action":"leases"}}),
        )]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    std::fs::write(f.root.join("note.txt"), "before\nNEEDLE\nafter\n").unwrap();
    assert_eq!(retained_artifact(&f, 9, "DECISIVE_EVIDENCE", "text"), art);
    let runner = runner(
        &f,
        &[
            "inspect",
            "work",
            "evidence",
            "retainEvidence",
            "releaseEvidence",
            "capability",
        ],
    )
    .await;
    let mut turn = input();
    turn.max_generations = Some(16);
    let outcome = f
        .host
        .run(turn, &Resolver::new(false), runner, &ignore)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "run failed {e:?}; branch={:?}",
                ag1_core::task::branch_values(
                    &f.store
                        .journal_snapshot(&SessionId::new("session").unwrap())
                        .unwrap()
                )
                .unwrap()
            )
        });
    assert_eq!(outcome.generations, 13);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 13);
    assert!(requests[3]["messages"].to_string().contains("update with work"));
    let current = ag1_core::context::hygiene::content(requests[3]["messages"].as_array().unwrap().last().unwrap(), "\n");
    assert!(!current.split(ag1_core::runtime::context::WORKING_STATE).last().unwrap().contains("update with taskState"));
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let results: Vec<_> = branch
        .iter()
        .filter(|r| r["message"]["role"] == "toolResult")
        .map(|r| &r["message"])
        .collect();
    assert_eq!(results.len(), 12);
    for r in &results {
        assert_ne!(r["isError"], true, "{r}");
    }
    assert_eq!(results[0]["toolName"], "inspect");
    assert_eq!(results[4]["toolName"], "work");
    assert!(results[1].to_string().contains("before"));
    assert!(results[6].to_string().contains("DECISIVE_EVIDENCE"));
    let task = &results[5]["details"]["snapshot"];
    assert_eq!(task["entries"].as_array().unwrap().len(), 2);
    assert_eq!(
        branch
            .iter()
            .filter(|r| r["customType"] == "runtime_observation")
            .count(),
        1
    );
    assert!(results[4]["details"]["taskStateEntryId"].is_string());
    assert!(
        f.store
            .evidence_lease_records("session")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("note.txt")).unwrap(),
        "before\nNEEDLE\nafter\n"
    );
    let records = f
        .store
        .session_tool_records(f.workspace.id().as_str(), "session", None, 64)
        .unwrap();
    assert!(records.iter().any(|r| r.tool == "runtimeObservation"));
    assert!(records.iter().any(|r| r.tool == "read"));
    assert!(!records.iter().any(|r| r.tool == "work"));
    // Reopening the exact durable branch must still accept the original tool names.
    let reopened = SessionRuntime::resume(
        SessionId::new("session").unwrap(),
        2,
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
        f.host.configuration().unwrap(),
    )
    .unwrap();
    assert!(reopened.pending_tools().is_empty());
    assert!(f.host.settled().unwrap());
    assert_eq!(ag1_core::tools::surface::message_tool(results[0]), "read");
    assert_eq!(
        ag1_core::tools::surface::message_tool(results[6]),
        "artifact"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn specialist_calls_do_not_bypass_policy_or_unsupported_capabilities() {
    let (origin, server) = server(vec![
        calls(vec![(
            "blocked-edit",
            "capability",
            json!({"action":"run","name":"editApply","arguments":{"path":"note.txt","edits":[{"oldText":"before","newText":"after"}]}}),
        )]),
        calls(vec![(
            "blocked-shell",
            "capability",
            json!({"action":"run","name":"bash","arguments":{"command":"touch forbidden"}}),
        )]),
        stop(),
    ]);
    let f = fixture(&origin, "interactive");
    std::fs::write(f.root.join("note.txt"), "before").unwrap();
    let runner = runner(&f, &["capability"]).await;
    f.host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    server.join().unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("note.txt")).unwrap(),
        "before"
    );
    assert!(!f.root.join("forbidden").exists());
    let records = f
        .store
        .session_tool_records(f.workspace.id().as_str(), "session", None, 16)
        .unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.effect == ag1_core::tools::executor::Effect::None)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mixed_work_observation_rejects_ignored_decision_fields_and_recovers() {
    let (origin, server) = server(vec![
        calls(vec![(
            "reset",
            "work",
            json!({"action":"reset","objective":"Preserve revision evidence"}),
        )]),
        calls(vec![(
            "mixed",
            "work",
            json!({"action":"observe","kind":"DECISION","observation":{"feature":"Decision stored","result":"pass","environment":"fixture","revision":"workspace","reporter":"agent","reproduction":"Record a decision","notes":"Receipt applies only to its recorded revision"}}),
        )]),
        calls(vec![(
            "record",
            "work",
            json!({"action":"record","kind":"DECISION","statement":"Receipt applies only to its recorded revision","confidence":"high"}),
        )]),
        calls(vec![("show", "work", json!({"action":"show"}))]),
        stop(),
    ]);
    let f = fixture(&origin, "disabled");
    let files = runner(&f, &["work"]).await;
    let mut turn = input();
    turn.max_generations = Some(8);
    f.host
        .run(turn, &Resolver::new(false), files, &ignore)
        .await
        .unwrap();
    assert_eq!(server.join().unwrap().len(), 5);
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let results: Vec<_> = branch
        .iter()
        .filter(|r| r["message"]["role"] == "toolResult")
        .map(|r| &r["message"])
        .collect();
    assert_eq!(results.len(), 4);
    assert_eq!(
        results[1]["isError"], true,
        "Mixed fields must not become a silently altered observation: {}",
        results[1]
    );
    assert!(
        !branch
            .iter()
            .any(|r| r["customType"] == "runtime_observation")
    );
    assert_ne!(results[2]["isError"], true);
    let entries = results[3]["details"]["snapshot"]["entries"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "DECISION");
    assert_eq!(
        entries[0]["statement"],
        "Receipt applies only to its recorded revision"
    );
}
