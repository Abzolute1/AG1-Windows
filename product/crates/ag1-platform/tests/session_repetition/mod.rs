use super::*;

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
