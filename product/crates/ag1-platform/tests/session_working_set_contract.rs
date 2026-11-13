#![cfg(target_os = "linux")]
include!("support/session_fixture.rs");

struct ContextObserver {
    inner: Resolver,
    stats: Mutex<Vec<Value>>,
    inputs: Mutex<Vec<Value>>,
}
impl ProviderResolver for ContextObserver {
    fn resolve<'a>(
        &'a self,
        input: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest> {
        self.stats
            .lock()
            .unwrap()
            .push(input.projection.stats.clone());
        self.inputs
            .lock()
            .unwrap()
            .push(json!(input.projection.messages));
        self.inner.resolve(input, cancel)
    }
}

fn large_history() -> Vec<Value> {
    let mut messages = vec![
        json!({"role":"user","content":"Preserve this completed historical work","timestamp":1}),
    ];
    for i in 0..40 {
        messages.push(json!({"role":"assistant","content":[{"type":"thinking","thinking":"r".repeat(100)},{"type":"toolCall","id":format!("historical-{i}"),"name":"read","arguments":{"path":"historical.txt"}}],"api":"openai-completions","provider":"owned-fixture","model":"initial","stopReason":"toolUse","timestamp":1}));
        messages.push(json!({"role":"toolResult","toolCallId":format!("historical-{i}"),"toolName":"read","content":[{"type":"text","text":format!("round {i}\nCURRENT_CONTRACT: preserve exact historical content {i}\n{}","x".repeat(12000))}],"isError":false,"timestamp":1}));
    }
    messages
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_working_history_survives_tool_appends_without_reexternalizing_originals() {
    let (origin, server) = server(vec![
        calls(vec![("first-read", "read", json!({"path":"current.txt"}))]),
        calls(vec![("second-read", "read", json!({"path":"current.txt"}))]),
        stop(),
    ]);
    let seed = large_history();
    let fixture = fixture_seeded(&origin, "disabled", seed.clone());
    std::fs::write(fixture.root.join("current.txt"), "Current small result").unwrap();
    let resolver = ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    };
    let result = fixture
        .host
        .run(input(), &resolver, fixture.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 3);
    assert!(fixture.host.settled().unwrap());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests {
        assert!(
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["role"] == "user")
        );
    }
    let stats = resolver.stats.lock().unwrap();
    assert!(stats[0]["externalizedToolResults"].as_u64().unwrap() >= 30);
    for current in &stats[1..] {
        assert_eq!(
            current["unchangedHistoryPrefixMessages"],
            current["admittedHistoryMessages"]
        );
        assert_eq!(
            current["requestPrefixComparison"]["configurationUnchanged"],
            true
        );
    }
    // A newly unprotected tail can need reduction. Already reduced older
    // history must not be externalized and evicted again on every tool round.
    assert!(
        stats[1]["externalizedToolResults"].as_u64().unwrap() < 4,
        "{}",
        stats[1]
    );
    assert!(
        stats[2]["externalizedToolResults"].as_u64().unwrap() < 4,
        "{}",
        stats[2]
    );
    assert!(
        stats[1]["tokensBefore"].as_u64().unwrap() < stats[0]["tokensBefore"].as_u64().unwrap() / 2
    );
    let records = fixture.host.client_records().unwrap();
    let retained: Vec<_> = records
        .iter()
        .filter(|r| r["type"] == "message")
        .take(seed.len())
        .map(|r| r["message"].clone())
        .collect();
    assert_eq!(
        retained, seed,
        "Compiler reuse must not replace or reduce durable original records"
    );
    let final_messages = requests[2]["messages"].as_array().unwrap();
    for id in ["first-read", "second-read"] {
        assert_eq!(
            final_messages
                .iter()
                .filter(|m| m["role"] == "tool" && m["tool_call_id"] == id)
                .count(),
            1
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discarded_preparation_cannot_publish_a_working_set() {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture_seeded(&origin, "disabled", large_history());
    let resolver = Arc::new(ContextObserver {
        inner: Resolver::new(true),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    });
    let worker = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let work = tokio::spawn(async move { host.run(input(), &*worker, files, &ignore).await });
    resolver.inner.entered.notified().await;
    let task = ag1_core::task::TaskState::reset("Fresh steering while preparation is unsent", NOW)
        .unwrap();
    fixture
        .host
        .append_task_state("steer", "steer", NOW, task, &ignore)
        .await
        .unwrap();
    resolver.inner.release.notify_one();
    work.await.unwrap().unwrap();
    let stats = resolver.stats.lock().unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0]["workingSetReused"], false);
    assert_eq!(stats[1]["workingSetReused"], false);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .to_string()
            .contains("Fresh steering while preparation is unsent")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_evidence_lease_applies_to_retained_compiler_artifact_identity() {
    let (origin, server) = server(vec![stop(), stop()]);
    // Ten rounds leave resident metadata to lease; the forty-round pressure
    // fixture deliberately evicts the whole old catalog before the next turn.
    let fixture = fixture_seeded(&origin, "disabled", large_history()[..21].to_vec());
    let resolver = ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    };
    fixture
        .host
        .run(input(), &resolver, fixture.files.clone(), &ignore)
        .await
        .unwrap();
    let first = resolver.inputs.lock().unwrap()[0].to_string();
    let ids = regex::Regex::new("art_[0-9a-f]{32}").unwrap();
    let id = ids
        .find(&first)
        .expect("A resident externalized artifact must be visible")
        .as_str()
        .to_owned();
    let origin = fixture.store.artifact_origin("session", &id).unwrap();
    fixture
        .store
        .lease(
            "session",
            &origin,
            &id,
            "Hold current evidence across the next request",
            120000,
            true,
        )
        .unwrap();
    let mut next = input();
    next.command = "next-probe".into();
    fixture
        .host
        .run(next, &resolver, fixture.files.clone(), &ignore)
        .await
        .unwrap();
    let stats = resolver.stats.lock().unwrap();
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[1]["workingSetReused"], true);
    assert!(
        stats[1]["leasedEvidenceMessages"].as_u64().unwrap() > 0,
        "{}",
        stats[1]
    );
    assert!(resolver.inputs.lock().unwrap()[1].to_string().contains(&id));
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn branch_navigation_does_not_reuse_abandoned_working_history() {
    use ag1_platform::session::tree::{TreeInput, TreeOptions};
    let (origin, server) = server(vec![stop(), stop()]);
    let f = fixture_seeded(&origin, "disabled", large_history());
    let resolver = Arc::new(ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    });
    f.host
        .run(input(), &*resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    let before = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    f.host
        .navigate_tree(
            TreeInput {
                command: "navigate".into(),
                timestamp: NOW.into(),
                options: TreeOptions {
                    target: "seed-0".into(),
                    summarize: false,
                    custom_instructions: None,
                    replace_instructions: false,
                    label: None,
                },
                reserve_tokens: 16384,
                summary_timeout_ms: 5000,
            },
            resolver.clone(),
            f.files.clone(),
            Arc::new(ignore),
        )
        .await
        .unwrap();
    let mut next = input();
    next.command = "after-branch".into();
    f.host
        .run(next, &*resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(resolver.stats.lock().unwrap()[1]["workingSetReused"], false);
    assert!(
        !requests[1]["messages"]
            .to_string()
            .contains("CURRENT_CONTRACT")
    );
    let after = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert!(after.entries.starts_with(&before.entries));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_changes_recompile_while_memory_changes_update_only_request_tail() {
    use ag1_core::memory::MemoryService;
    for change in ["model", "memory"] {
        let (origin, server) = server(vec![stop(), stop()]);
        let f = fixture_seeded(&origin, "disabled", large_history());
        let resolver = ContextObserver {
            inner: Resolver::new(false),
            stats: Mutex::new(vec![]),
            inputs: Mutex::new(vec![]),
        };
        f.host
            .run(input(), &resolver, f.files.clone(), &ignore)
            .await
            .unwrap();
        if change == "model" {
            let mut model = f.model.clone();
            model["id"] = json!("replacement");
            f.host
                .select_model("switch", "switch", NOW, model, &ignore)
                .await
                .unwrap();
        } else {
            MemoryService::mutate(&*f.store, f.workspace.id().as_str(), None, |memory| {
                Ok(memory
                    .add(
                        "decision",
                        "Preserve the newly selected fixture behavior",
                        2,
                        "core",
                        &json!({}),
                    )?
                    .store)
            })
            .unwrap();
        }
        let mut next = input();
        next.command = "after-change".into();
        f.host
            .run(next, &resolver, f.files.clone(), &ignore)
            .await
            .unwrap();
        assert_eq!(
            resolver.stats.lock().unwrap()[1]["workingSetReused"],
            change == "memory",
            "{change}"
        );
        let requests = server.join().unwrap();
        let stats = resolver.stats.lock().unwrap();
        assert_eq!(
            stats[1]["requestPrefixComparison"]["configurationUnchanged"],
            change == "memory"
        );
        if change == "model" {
            assert_eq!(requests[1]["model"], "replacement");
        } else {
            assert_eq!(
                stats[1]["unchangedHistoryPrefixMessages"],
                stats[1]["admittedHistoryMessages"]
            );
            let first = resolver.inputs.lock().unwrap()[0]
                .as_array()
                .unwrap()
                .clone();
            let next = resolver.inputs.lock().unwrap()[1]
                .as_array()
                .unwrap()
                .clone();
            let common = first
                .iter()
                .zip(&next)
                .take_while(|(before, after)| before == after)
                .count();
            assert_eq!(
                stats[1]["requestPrefixComparison"]["unchangedMessagePrefix"],
                common
            );
            assert_eq!(
                requests[0]["messages"][0], requests[1]["messages"][0],
                "Memory mutation must preserve the system prefix"
            );
            assert!(
                !requests[1]["messages"][0]
                    .to_string()
                    .contains("Preserve the newly selected fixture behavior")
            );
            assert!(
                requests[1]
                    .to_string()
                    .contains("Preserve the newly selected fixture behavior")
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn long_native_rounds_batch_reductions_and_retain_final_request_diagnostics() {
    let mut responses = (0..20)
        .map(|id| {
            calls(vec![(
                &format!("read-{id}"),
                "read",
                json!({"path":format!("evidence-{id}.txt")}),
            )])
        })
        .collect::<Vec<_>>();
    responses.push(stop());
    let (origin, server) = server(responses);
    let f = fixture_configured_tools(&origin, "disabled", vec![], None, None, 3, &["read"]);
    for id in 0..20 {
        std::fs::write(
            f.root.join(format!("evidence-{id}.txt")),
            format!(
                "CURRENT_CONTRACT: exact evidence {id}\n{}",
                "x".repeat(8000)
            ),
        )
        .unwrap();
    }
    let resolver = ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    };
    let mut turn = input();
    turn.max_generations = Some(21);
    let result = f
        .host
        .run(turn, &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 21);
    assert!(f.host.settled().unwrap());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 21);
    let stats = resolver.stats.lock().unwrap();
    let inputs = resolver.inputs.lock().unwrap();
    let mut reductions = 0;
    let mut stable = 0;
    for i in 1..stats.len() {
        let stat = &stats[i];
        assert_eq!(stat["workingSetReused"], true);
        let before = inputs[i - 1].as_array().unwrap();
        let after = inputs[i].as_array().unwrap();
        let common = before.iter().zip(after).take_while(|(a, b)| a == b).count();
        assert_eq!(
            stat["requestPrefixComparison"]["unchangedMessagePrefix"],
            common
        );
        assert_eq!(
            stat["requestPrefixComparison"]["configurationUnchanged"],
            true
        );
        assert_eq!(
            stat["requestPrefixComparison"]["firstDifferentMessage"],
            common
        );
        assert!(ag1_core::context::retry::complete_pairs(after));
        if stat["externalizedToolResults"].as_u64().unwrap() > 0 {
            reductions += 1;
            assert!(
                stat["tokensAfter"].as_u64().unwrap()
                    <= stat["compactionTargetTokens"].as_u64().unwrap(),
                "{stat}"
            );
        } else {
            stable += 1;
            assert_eq!(
                stat["unchangedHistoryPrefixMessages"],
                stat["admittedHistoryMessages"]
            );
        }
    }
    assert!(reductions >= 2, "fixture must cross pressure repeatedly");
    assert!(
        stable > reductions,
        "reductions should create multi-round headroom"
    );
    eprintln!(
        "controlled native context: {} requests; {reductions} reduction rounds; {stable} unchanged-history continuation rounds",
        stats.len()
    );
    let retained = f
        .store
        .artifact_catalog("session", None, 128, Some("runtime-request-compiled"))
        .unwrap();
    assert_eq!(retained.len(), inputs.len());
    let context = ag1_storage::context::ContextStore::new(
        &f.store,
        SessionId::new("session").unwrap(),
        "session",
        "inspect",
        NOW,
        ag1_core::context::features::Features::default(),
    )
    .unwrap();
    for record in retained {
        let text = context
            .bounded_text(&record.position.id, 1_000_000)
            .unwrap();
        let request: Value = serde_json::from_str(&text).unwrap();
        let index = inputs
            .iter()
            .position(|input| input == &request["messages"])
            .unwrap();
        assert_eq!(
            request["stats"], stats[index],
            "retained diagnostics must match the actual final compiler projection"
        );
        assert_eq!(request["thinking"], "off");
    }
    let records = f.host.client_records().unwrap();
    for id in 0..20 {
        let results = records
            .iter()
            .filter(|record| {
                record["message"]["role"] == "toolResult"
                    && record["message"]["toolCallId"] == format!("read-{id}")
            })
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1);
        assert!(
            results[0]["message"]["content"]
                .to_string()
                .contains(&"x".repeat(8000)),
            "durable original must not shrink"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_history_never_caches_source_freshness_verdicts() {
    use ag1_core::task::TaskState;
    use ag1_platform::source::SourceService;
    let (origin, server) = server(vec![stop(), stop()]);
    let f = fixture_seeded(&origin, "disabled", large_history());
    std::fs::write(f.root.join("evidence.txt"), "before").unwrap();
    let source = SourceService::new(f.workspace.clone())
        .capture("evidence.txt")
        .unwrap()
        .reference()
        .reference();
    let (task, _) = TaskState::empty(NOW)
        .set(&json!({"objective":"Use evidence"}), NOW)
        .unwrap()
        .record(
            &json!({"kind":"FACT","statement":"Owned fixture evidence","sources":[source]}),
            "ts_00000001",
            NOW,
        )
        .unwrap();
    f.host
        .append_task_state("task", "task", NOW, task, &ignore)
        .await
        .unwrap();
    let resolver = ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    };
    f.host
        .run(input(), &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    std::fs::write(f.root.join("evidence.txt"), "after!").unwrap();
    let mut next = input();
    next.command = "after-source".into();
    f.host
        .run(next, &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(resolver.stats.lock().unwrap()[1]["workingSetReused"], true);
    let requests = server.join().unwrap();
    assert!(!requests[0].to_string().contains("[STALE current="));
    assert!(requests[1].to_string().contains("stale_source=true"));
    assert!(requests[1].to_string().contains("[STALE current="));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reopened_owner_reconstructs_from_durable_originals() {
    let (origin, server) = server(vec![stop(), stop()]);
    let f = fixture_seeded(&origin, "disabled", large_history());
    let resolver = ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    };
    f.host
        .run(input(), &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    assert!(f.host.settled().unwrap());
    let state = SessionRuntime::resume(
        SessionId::new("session").unwrap(),
        2,
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
        f.host.configuration().unwrap(),
    )
    .unwrap();
    let reopened = NativeSession::new(
        state,
        f.store.clone(),
        "session".into(),
        f.workspace.clone(),
        Arc::new(BlockingPool::new(3, 32).unwrap()),
        f.scheduler.clone(),
        ProviderHttp::new(&[origin], Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    let mut next = input();
    next.command = "after-reopen".into();
    reopened
        .run(next, &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(resolver.stats.lock().unwrap()[1]["workingSetReused"], false);
    assert!(
        resolver.stats.lock().unwrap()[1]["externalizedToolResults"]
            .as_u64()
            .unwrap()
            >= 30
    );
    assert_eq!(server.join().unwrap().len(), 2);
    assert!(reopened.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_scope_is_request_local_and_tracks_actual_source_tools() {
    let (origin, server) = server(vec![stop(), stop(), stop()]);
    let f = fixture_seeded(&origin, "disabled", large_history());
    let resolver = ContextObserver {
        inner: Resolver::new(false),
        stats: Mutex::new(vec![]),
        inputs: Mutex::new(vec![]),
    };
    f.host
        .run(input(), &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    let mut next = input();
    next.command = "warm".into();
    f.host
        .run(next, &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    let tools = f
        .host
        .configuration()
        .unwrap()
        .tools
        .into_iter()
        .filter(|t| t["name"] == "read")
        .collect();
    f.host
        .select_tools("readonly", "readonly", NOW, tools, &ignore)
        .await
        .unwrap();
    let mut next = input();
    next.command = "readonly-turn".into();
    f.host
        .run(next, &resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    for request in &requests[..2] {
        let text = request["messages"].to_string();
        assert_eq!(text.matches("<source-verification-scope>").count(), 1);
        assert!(text.contains("A passing command proves only the checks it executed"));
        assert!(text.contains("uncovered boundary or degenerate inputs implied by the contract"));
    }
    assert!(
        !requests[2]["messages"]
            .to_string()
            .contains("<source-verification-scope>")
    );
    assert_eq!(resolver.stats.lock().unwrap()[1]["workingSetReused"], true);
    assert!(
        !json!(f.host.client_records().unwrap().as_ref())
            .to_string()
            .contains("<source-verification-scope>")
    );
}
