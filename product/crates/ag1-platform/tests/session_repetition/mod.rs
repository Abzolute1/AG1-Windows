use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_edit_variants_pause_without_effects_and_a_fresh_user_can_repair() {
    let mut replies = (0..3).map(|i| calls(vec![(
        &format!("stale-{i}"), "edit", json!({"path":if i==1 {"./source.txt"} else {"source.txt"},"edits":[{"oldText":format!("obsolete variant {i}"),"newText":format!("new narration {i}")} ]})
    )])).collect::<Vec<_>>();
    replies.push(calls(vec![(
        "corrected",
        "edit",
        json!({"path":"source.txt","edits":[{"oldText":"actual","newText":"repaired"}]}),
    )]));
    replies.push(stop());
    let (origin, server) = server(replies);
    let f = fixture_configured_tools(
        &origin,
        "disabled",
        vec![],
        None,
        None,
        3,
        &["read", "write", "edit"],
    );
    std::fs::write(f.root.join("source.txt"), "actual").unwrap();
    let first = f
        .host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(first.generations, 3);
    assert_eq!(first.assistant["nativePolicy"], "repetition_guard");
    assert_eq!(
        std::fs::read_to_string(f.root.join("source.txt")).unwrap(),
        "actual"
    );
    let results = session_tool_results(&f);
    assert_eq!(results.len(), 3);
    for result in results {
        assert_eq!(result["isError"], true);
        assert!(result.to_string().contains("Read a small current range"));
    }
    let mut next = input();
    next.command = "recover-user".into();
    f.host
        .run(next, &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("source.txt")).unwrap(),
        "repaired"
    );
    assert_eq!(server.join().unwrap().len(), 5);
    assert!(f.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_stale_edit_can_recover_and_changed_source_resets_failure_history() {
    let (origin, server) = server(vec![
        calls(vec![(
            "stale-a",
            "edit",
            json!({"path":"source.txt","edits":[{"oldText":"missing","newText":"x"}]}),
        )]),
        calls(vec![(
            "read-current",
            "read",
            json!({"path":"source.txt","offset":1,"limit":3}),
        )]),
        calls(vec![(
            "corrected",
            "edit",
            json!({"path":"source.txt","edits":[{"oldText":"actual","newText":"changed"}]}),
        )]),
        calls(vec![(
            "stale-b",
            "edit",
            json!({"path":"source.txt","edits":[{"oldText":"missing","newText":"x"}]}),
        )]),
        calls(vec![(
            "stale-c",
            "edit",
            json!({"path":"source.txt","edits":[{"oldText":"missing again","newText":"x"}]}),
        )]),
        stop(),
    ]);
    let f = fixture_configured_tools(
        &origin,
        "disabled",
        vec![],
        None,
        None,
        3,
        &["read", "write", "edit"],
    );
    std::fs::write(f.root.join("source.txt"), "actual").unwrap();
    let result = f
        .host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 6);
    assert!(result.assistant["nativePolicy"].is_null());
    assert_eq!(
        std::fs::read_to_string(f.root.join("source.txt")).unwrap(),
        "changed"
    );
    assert_eq!(server.join().unwrap().len(), 6);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checklist_queries_do_not_hide_repeated_conflicts_or_redundant_updates() {
    for redundant in [false, true] {
        let initial = json!({"todos":[{"id":"t1","title":"Build","status":"in_progress"},{"id":"t2","title":"Verify","status":"pending"}]});
        let mut replies = vec![calls(vec![("initial", "todo", initial.clone())])];
        for i in 0..3 {
            replies.push(calls(vec![(&format!("query-{i}"), "todo", json!({}))]));
            let args = if redundant {
                initial.clone()
            } else {
                json!({"todos":[{"id":"t1","title":format!("Renamed {i}"),"status":"in_progress"},{"id":"t2","title":"Verify","status":"pending"}]})
            };
            replies.push(calls(vec![(&format!("retry-{i}"), "todo", args)]));
        }
        let (origin, server) = server(replies);
        let f = fixture(&origin, "disabled");
        let runner = native_todo_runner(&f).await;
        let result = f
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await
            .unwrap();
        assert_eq!(result.generations, 7);
        assert_eq!(result.assistant["nativePolicy"], "repetition_guard");
        let results = session_tool_results(&f);
        for i in [2, 4, 6] {
            assert_eq!(results[i]["isError"] == true, !redundant);
        }
        let branch = f.host.client_records().unwrap();
        let current = ag1_core::tools::todo::latest(&branch).unwrap();
        assert_eq!(current["items"], initial["todos"]);
        assert_eq!(server.join().unwrap().len(), 7);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn semantic_pause_blocks_later_batch_mutations_and_requires_durable_ack() {
    for fault in [
        None,
        Some(ag1_storage::Boundary::BeforeCommit),
        Some(ag1_storage::Boundary::BeforeAck),
    ] {
        let mut responses = vec![];
        for i in 0..3 {
            let mut batch = vec![(
                format!("stale-{i}"),
                "edit",
                json!({"path":"source.txt","edits":[{"oldText":"missing","newText":"wrong"}]}),
            )];
            if i == 2 {
                batch.push((
                    "later-write".into(),
                    "write",
                    json!({"path":"must-not-exist.txt","content":"unwanted"}),
                ));
            }
            responses.push(calls(
                batch
                    .iter()
                    .map(|(id, tool, args)| (id.as_str(), *tool, args.clone()))
                    .collect(),
            ));
        }
        let (origin, server) = server(responses);
        let injector =
            fault.map(|point| Arc::new(PauseFault(point)) as Arc<dyn ag1_storage::FaultInjector>);
        let f = fixture_configured_tools(
            &origin,
            "disabled",
            vec![],
            injector,
            None,
            3,
            &["edit", "write"],
        );
        std::fs::write(f.root.join("source.txt"), "actual").unwrap();
        let result = f
            .host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
            .await;
        assert_eq!(result.is_err(), fault.is_some());
        assert_eq!(f.host.persistence_healthy().unwrap(), fault.is_none());
        assert_eq!(
            std::fs::read_to_string(f.root.join("source.txt")).unwrap(),
            "actual"
        );
        assert!(!f.root.join("must-not-exist.txt").exists());
        let results = session_tool_results(&f);
        assert_eq!(results.len(), 4);
        assert!(results[3].to_string().contains("task is incomplete"));
        assert_eq!(server.join().unwrap().len(), 3);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_native_preparation_has_one_observation_and_two_durable_outcomes() {
    let (origin, server) = server(vec![
        calls(vec![
            ("first", "read", json!({"path":"source.txt"})),
            ("duplicate", "read", json!({"path":"source.txt"})),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("source.txt"), "one observation").unwrap();
    let result = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert_eq!(result.generations, 2);
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    let results = branch
        .iter()
        .filter(|e| e["message"]["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    assert_eq!(results[1]["message"]["toolCallId"], "duplicate");
    assert!(
        results[1]["message"]["content"]
            .to_string()
            .contains("already pending")
    );
    assert_eq!(
        results[1]["message"]["details"]["rejectedBeforeExecution"],
        true
    );
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["customType"] == "inspection_evidence")
            .count(),
        1
    );
    assert_eq!(server.join().unwrap().len(), 2);
}

struct PauseFault(ag1_storage::Boundary);
impl ag1_storage::FaultInjector for PauseFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command.ends_with("-repetition-pause") && point == self.0 {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repetition_pause_requires_its_own_durable_acknowledgement() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let replies = (0..4)
            .map(|i| {
                calls(vec![(
                    &format!("read-{i}"),
                    "read",
                    json!({"path":"source.txt"}),
                )])
            })
            .collect();
        let (origin, server) = server(replies);
        let fixture = fixture_seeded_with_fault(
            &origin,
            "disabled",
            vec![],
            Some(Arc::new(PauseFault(point))),
        );
        std::fs::write(fixture.root.join("source.txt"), "same evidence").unwrap();
        assert!(
            fixture
                .host
                .run(
                    input(),
                    &Resolver::new(false),
                    fixture.files.clone(),
                    &ignore
                )
                .await
                .is_err()
        );
        assert!(!fixture.host.persistence_healthy().unwrap());
        assert_eq!(server.join().unwrap().len(), 4);
        assert_eq!(
            provider_outputs(&fixture.store)
                .iter()
                .filter(|v| v["kind"] == "runtime-provider-output")
                .count(),
            4
        );
        let journal = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        let branch = ag1_core::task::branch_values(&journal).unwrap();
        assert_eq!(
            branch
                .iter()
                .filter(|e| e["message"]["role"] == "toolResult")
                .count(),
            4
        );
        assert_eq!(
            branch.last().unwrap()["message"]["nativePolicy"] == "repetition_guard",
            point == ag1_storage::Boundary::BeforeAck
        );
    }
}

#[test]
fn native_repetition_matches_unchanged_sdk_local_filesystem_traces() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/repetition-oracle.json"
    ))
    .unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.ts"), "export const a = 1;\n").unwrap();
        std::fs::write(root.join("src/b.ts"), "export const b = 2;\n").unwrap();
        std::os::unix::fs::symlink(root.join("src"), root.join("alias")).unwrap();
        let guard = ag1_platform::session::repetition::Guard::new(root.clone());
        let mut now = 1000;
        let mut pending = std::collections::BTreeMap::<String, Value>::new();
        let mut outcomes = Vec::new();
        let mut events = Vec::new();
        for step in row["steps"].as_array().unwrap() {
            match step["kind"].as_str().unwrap() {
                "reset" => guard.reset().unwrap(),
                "start" => guard.start_turn().unwrap(),
                "time" => now += step["ms"].as_u64().unwrap(),
                "write" => std::fs::write(
                    root.join(step["path"].as_str().unwrap()),
                    step["text"].as_str().unwrap(),
                )
                .unwrap(),
                "tree" => {
                    for i in 0..257 {
                        std::fs::write(root.join("src").join(i.to_string()), "").unwrap();
                    }
                }
                kind => {
                    let id = step["id"].as_str().unwrap();
                    let item = if kind == "after" {
                        pending[id].clone()
                    } else {
                        step.clone()
                    };
                    let args: Value = serde_json::from_str(
                        &item["args"]
                            .to_string()
                            .replace("$ROOT", root.to_str().unwrap()),
                    )
                    .unwrap();
                    let tool = item["tool"].as_str().unwrap();
                    let result = json!({"content":[{"type":"text","text":item["text"]}],"isError":item["error"]});
                    if kind == "after" {
                        guard.after(tool, id, &args, &result, now).unwrap();
                    } else {
                        let (reason, emitted) = guard.before(tool, id, &args, now).unwrap();
                        outcomes.push(json!({"id":id,"reason":reason}));
                        events.extend(emitted.into_iter().map(|event| {
                            let mut event = serde_json::to_value(event).unwrap();
                            event.as_object_mut().unwrap().remove("signature");
                            event
                        }));
                        if reason.is_none() {
                            if kind == "execute" {
                                guard.after(tool, id, &args, &result, now).unwrap();
                            }
                            pending.insert(id.into(), item);
                        }
                    }
                }
            }
        }
        assert_eq!(json!(outcomes), row["outcomes"], "{} outcomes", row["id"]);
        assert_eq!(json!(events), row["events"], "{} events", row["id"]);
        assert_eq!(
            json!(guard.stop_reason().unwrap()),
            row["stop"],
            "{} stop",
            row["id"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_native_inspections_pause_with_durable_outcomes_and_resume_on_new_user() {
    let mut replies = (0..4)
        .map(|i| {
            calls(vec![(
                &format!("read-{i}"),
                "read",
                json!({"path":"source.txt"}),
            )])
        })
        .collect::<Vec<_>>();
    replies.push(calls(vec![(
        "fresh-read",
        "read",
        json!({"path":"source.txt"}),
    )]));
    replies.push(stop());
    let (origin, server) = server(replies);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("source.txt"), "unchanged evidence").unwrap();
    let events = Mutex::new(Vec::new());
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(event) = event {
            events.lock().unwrap().push(event.clone());
        }
    };
    let first = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &listener,
        )
        .await
        .unwrap();
    assert_eq!(first.generations, 4);
    assert_eq!(first.assistant["nativePolicy"], "repetition_guard");
    assert!(
        first.assistant["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("task is incomplete")
    );
    assert_eq!(first.assistant["usage"]["output"], 0);
    assert!(fixture.host.settled().unwrap());
    assert!(fixture.host.persistence_healthy().unwrap());
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    let results = branch
        .iter()
        .filter(|e| e["message"]["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 4);
    assert!(results[..2].iter().all(|e| e["message"]["isError"] != true));
    assert!(
        results[2..]
            .iter()
            .all(|e| e["message"]["details"]["rejectedBeforeExecution"] == true)
    );
    assert_eq!(branch.last().unwrap()["message"], first.assistant);
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e["type"] == "repetition_guard" && e["action"] == "paused")
            .count(),
        1
    );
    let mut resumed = input();
    resumed.command = "resume-after-pause".into();
    let second = fixture
        .host
        .run(
            resumed,
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert_eq!(second.generations, 2);
    assert_eq!(second.assistant["content"][0]["text"], "Done");
    assert_eq!(server.join().unwrap().len(), 6);
}
