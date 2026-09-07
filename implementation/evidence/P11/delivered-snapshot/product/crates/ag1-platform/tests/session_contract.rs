#![cfg(target_os = "linux")]
//! Real native loop, SQL/context custody, HTTP codec and file effects. The only
//! simulated component is the owned loopback model endpoint.
include!("support/session_fixture.rs");
#[path = "session_commands/mod.rs"]
mod commands;
#[path = "session_completion_receipts/mod.rs"]
mod completion_receipts;
#[path = "session_continuation/mod.rs"]
mod continuation;
#[path = "session_controlled/mod.rs"]
mod controlled;
#[path = "session_controls/mod.rs"]
mod controls;
#[path = "session_extensions/mod.rs"]
mod extensions;
#[path = "session_failures/mod.rs"]
mod failures;
#[path = "session_health/mod.rs"]
mod health;
#[path = "session_jobs/mod.rs"]
mod jobs;
#[path = "session_planning/mod.rs"]
mod planning;
#[path = "session_print/mod.rs"]
mod print;
#[path = "session_repetition/mod.rs"]
mod repetition;
#[path = "session_restart/mod.rs"]
mod restart;
#[path = "session_subagent/mod.rs"]
mod subagent;
#[path = "session_tree/mod.rs"]
mod tree;
#[path = "session_verification/mod.rs"]
mod verification;
#[path = "session_xml/mod.rs"]
mod xml;
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_turn_writes_reads_and_continues_with_durable_tool_history() {
    let (origin, server) = server(vec![
        calls(vec![(
            "write-call",
            "write",
            json!({"path":"output.txt","content":"hello native"}),
        )]),
        calls(vec![
            ("read-one", "read", json!({"path":"output.txt","limit":1})),
            ("read-two", "read", json!({"path":"output.txt","limit":2})),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let revisions = Mutex::new(vec![]);
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Committed { revision, .. } = event {
            revisions.lock().unwrap().push(*revision);
        }
    };
    let result = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &listener,
        )
        .await
        .unwrap();
    assert_eq!(result.generations, 3);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("output.txt")).unwrap(),
        "hello native"
    );
    assert_eq!(result.assistant["content"][0]["text"], "Done");
    assert!(fixture.host.settled().unwrap());
    assert_eq!(*revisions.lock().unwrap(), (1..=10).collect::<Vec<_>>());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    let second_tools = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect::<Vec<_>>();
    assert_eq!(second_tools.len(), 1);
    assert!(
        second_tools[0]["content"]
            .as_str()
            .unwrap()
            .contains("output.txt")
    );
    let results = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 3);
    assert!(
        results[1]["content"]
            .as_str()
            .unwrap()
            .contains("hello native")
    );
    assert_eq!(results[1]["tool_call_id"], "read-one");
    assert_eq!(results[2]["tool_call_id"], "read-two");
    let state = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(state.revision, 10);
    let branch = ag1_core::task::branch_values(&state).unwrap();
    assert_eq!(
        branch
            .iter()
            .filter(|entry| entry["customType"] == "inspection_evidence")
            .count(),
        2
    );
    let final_user = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "user")
        .unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(final_user.contains("Inspection evidence indexes"));
    assert!(final_user.contains("result=tool-call:read-two"));
    assert!(final_user.contains("status=current"));
    assert!(
        !requests[0]
            .to_string()
            .contains("Inspection evidence indexes")
    );
    assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 0);
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_change_during_async_resolution_discards_unsent_request_and_rebuilds() {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture(&origin, "disabled");
    let resolver = Arc::new(Resolver::new(true));
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run =
        tokio::spawn(async move { host.run(input(), &*worker_resolver, files, &ignore).await });
    resolver.entered.notified().await;
    let mut model = fixture.model.clone();
    model["id"] = json!("replacement");
    fixture
        .host
        .select_model("switch", "switch", NOW, model, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    let result = run.await.unwrap().unwrap();
    assert_eq!(result.generations, 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    let requests = server.join().unwrap();
    assert_eq!(requests[0]["model"], "replacement");
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planning_policy_rejects_write_with_a_durable_result_and_no_file_effect() {
    let (origin, server) = server(vec![
        calls(vec![(
            "blocked-write",
            "write",
            json!({"path":"forbidden.txt","content":"no"}),
        )]),
        stop(),
    ]);
    let fixture = fixture(&origin, "planning");
    fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert!(!fixture.root.join("forbidden.txt").exists());
    let state = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let messages = ag1_core::task::branch_values(&state).unwrap();
    let result = messages
        .iter()
        .find(|e| e["message"]["role"] == "toolResult")
        .unwrap();
    assert_eq!(result["message"]["isError"], true);
    assert_eq!(
        result["message"]["details"]["rejectedBeforeExecution"],
        true
    );
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listener_exception_cannot_break_transport_or_internal_journal_acknowledgement() {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture(&origin, "disabled");
    let seen = AtomicUsize::new(0);
    let listener = |event: &SessionEvent| {
        let kind = match event {
            SessionEvent::Committed { .. } => 1,
            SessionEvent::Provider(_) => 2,
            SessionEvent::Recovery(_) => 4,
        };
        if seen.fetch_or(kind, Ordering::SeqCst) & kind == 0 {
            panic!("owned listener failure");
        }
    };
    let result = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &listener,
        )
        .await
        .unwrap();
    assert_eq!(result.journal_revision, 3);
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    assert!(fixture.host.settled().unwrap());
    server.join().unwrap();
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_drops_safe_async_preparation_without_sending_http() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let resolver = Arc::new(Resolver::new(true));
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run =
        tokio::spawn(async move { host.run(input(), &*worker_resolver, files, &ignore).await });
    resolver.entered.notified().await;
    fixture.host.cancel().unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(fixture.host.journal_revision().unwrap(), 2);
    assert!(fixture.host.settled().unwrap());
    assert_eq!(
        fixture
            .scheduler
            .status("http://127.0.0.1:1")
            .unwrap()
            .active,
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_tool_is_removed_from_the_rebuilt_request_and_cannot_execute() {
    let (origin, server) = server(vec![
        calls(vec![(
            "disabled-call",
            "write",
            json!({"path":"disabled.txt","content":"no"}),
        )]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let resolver = Arc::new(Resolver::new(true));
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run =
        tokio::spawn(async move { host.run(input(), &*worker_resolver, files, &ignore).await });
    resolver.entered.notified().await;
    fixture
        .host
        .select_tools("disable", "disable", NOW, vec![], &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    assert!(!fixture.root.join("disabled.txt").exists());
    let requests = server.join().unwrap();
    assert!(requests.iter().all(|request| {
        request
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    }));
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&snapshot).unwrap();
    let result = branch
        .iter()
        .find(|entry| entry["message"]["role"] == "toolResult")
        .unwrap();
    assert_eq!(
        result["message"]["details"]["rejectedBeforeExecution"],
        true
    );
}

struct WaitingApproval {
    entered: Notify,
    approved: AtomicBool,
}
impl ag1_platform::session::files::ApprovalHandler for WaitingApproval {
    fn approve(
        &self,
        _: &ag1_core::tools::approval::ApprovalRequest,
        cancel: &AtomicBool,
    ) -> Result<bool, DomainError> {
        self.entered.notify_one();
        let limit = Instant::now() + Duration::from_secs(5);
        while !cancel.load(Ordering::Acquire) && !self.approved.load(Ordering::Acquire) {
            if Instant::now() >= limit {
                return Err(DomainError::TimedOut);
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(self.approved.load(Ordering::Acquire))
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_native_approval_drains_one_rejection_per_call() {
    let (origin, server) = server(vec![calls(vec![
        (
            "first-call",
            "write",
            json!({"path":"output.txt","content":"first"}),
        ),
        (
            "second-call",
            "write",
            json!({"path":"output.txt","content":"second"}),
        ),
    ])]);
    let mut fixture = fixture(&origin, "interactive");
    let approval = Arc::new(WaitingApproval {
        entered: Notify::new(),
        approved: AtomicBool::new(false),
    });
    Arc::get_mut(&mut fixture.files).unwrap().approvals = approval.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), files, &ignore)
            .await
    });
    approval.entered.notified().await;
    assert!(!fixture.host.settled().unwrap());
    fixture.host.cancel().unwrap();
    assert!(matches!(run.await.unwrap(), Err(DomainError::Cancelled)));
    assert!(fixture.host.settled().unwrap());
    assert!(!fixture.root.join("output.txt").exists());
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&snapshot).unwrap();
    let outcomes = branch
        .iter()
        .filter(|entry| entry["message"]["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(outcomes[0]["message"]["toolCallId"], "first-call");
    assert_eq!(outcomes[1]["message"]["toolCallId"], "second-call");
    assert!(
        outcomes
            .iter()
            .all(|entry| entry["message"]["details"]["rejectedBeforeExecution"] == true)
    );
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_selection_change_while_approval_waits_invalidates_the_old_authorization() {
    let (origin, server) = server(vec![
        calls(vec![(
            "stale-call",
            "write",
            json!({"path":"output.txt","content":"no"}),
        )]),
        stop(),
    ]);
    let mut fixture = fixture_configured(&origin, "interactive", vec![], None, None, 1);
    let approval = Arc::new(WaitingApproval {
        entered: Notify::new(),
        approved: AtomicBool::new(false),
    });
    Arc::get_mut(&mut fixture.files).unwrap().approvals = approval.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), files, &ignore)
            .await
    });
    approval.entered.notified().await;
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture
            .host
            .select_tools("disable", "disable", NOW, vec![], &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    approval.approved.store(true, Ordering::Release);
    run.await.unwrap().unwrap();
    assert!(!fixture.root.join("output.txt").exists());
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&snapshot).unwrap();
    let result = branch
        .iter()
        .find(|entry| entry["message"]["role"] == "toolResult")
        .unwrap();
    assert_eq!(
        result["message"]["details"]["rejectedBeforeExecution"],
        true
    );
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_transport_keeps_session_unsettled_until_owned_endpoint_reconciliation() {
    let (origin, server) = server(vec![json!({"fixtureDisconnect":true})]);
    let fixture = fixture(&origin, "disabled");
    let result = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await;
    assert!(result.is_err());
    server.join().unwrap();
    assert!(!fixture.host.settled().unwrap());
    let ticket = fixture.host.pending_transport().unwrap().unwrap();
    assert!(!fixture.scheduler.ticket_settled(&ticket).unwrap());
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
    // The owned fixture server has actually exited. A production embedding
    // obtains equivalent supervisor/transport evidence before reconciliation.
    fixture.scheduler.reconcile(&ticket).unwrap();
    assert!(fixture.host.pending_transport().unwrap().is_none());
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_pressure_recovery_stops_after_checkpoint_when_summary_transport_is_uncertain() {
    let (origin, server) = server(vec![json!({"fixtureDisconnect":true})]);
    let fixture = fixture_seeded(&origin, "disabled", pressure_history());
    fixture
        .host
        .select_tools("tools", "tools", NOW, vec![], &ignore)
        .await
        .unwrap();
    let mut model = fixture.model.clone();
    model["contextWindow"] = json!(10000);
    fixture
        .host
        .select_model("model", "model", NOW, model, &ignore)
        .await
        .unwrap();
    let mut turn = input();
    turn.compaction = compact_input().settings;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        fixture
            .host
            .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(DomainError::Unavailable(_))));
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&snapshot).unwrap();
    let checkpoint = branch
        .iter()
        .find(|entry| entry["type"] == "compaction")
        .unwrap();
    assert_eq!(
        checkpoint["details"]["method"],
        "persistent_emergency_checkpoint"
    );
    assert_eq!(checkpoint["contextEpoch"], 2);
    assert_eq!(server.join().unwrap().len(), 1);
    assert!(!fixture.host.settled().unwrap());
    fixture
        .scheduler
        .reconcile(&fixture.host.pending_transport().unwrap().unwrap())
        .unwrap();
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_excludes_failed_attempt_from_next_request_but_keeps_its_durable_record() {
    let (origin, server) = server(vec![
        json!({"error":{"message":"server overloaded"}}),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let mut turn = input();
    turn.retry.base_delay_ms = 1;
    let events = Mutex::new(vec![]);
    let listener = |event: &SessionEvent| {
        if let SessionEvent::Recovery(value) = event {
            events.lock().unwrap().push(value.clone());
        }
    };
    let result = fixture
        .host
        .run(
            turn,
            &Resolver::new(false),
            fixture.files.clone(),
            &listener,
        )
        .await
        .unwrap();
    assert_eq!(result.generations, 2);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["messages"], requests[1]["messages"]);
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(journal.entries.len(), 4);
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    assert_eq!(branch.len(), 3);
    assert!(
        !branch
            .iter()
            .any(|entry| entry["message"]["stopReason"] == "error")
    );
    let events = events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["degraded_turn", "auto_retry_start", "auto_retry_end"]
    );
    assert_eq!(events[0]["reasons"], json!(["error_turn"]));
    assert_eq!(events[2]["success"], true);
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_limit_keeps_last_failure_active_without_sending_an_extra_request() {
    let fail = json!({"error":{"message":"server overloaded"}});
    let (origin, server) = server(vec![fail.clone(), fail]);
    let fixture = fixture(&origin, "disabled");
    let mut turn = input();
    turn.retry.base_delay_ms = 1;
    turn.retry.max_retries = 1;
    assert!(
        fixture
            .host
            .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
            .await
            .is_err()
    );
    assert_eq!(server.join().unwrap().len(), 2);
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(journal.entries.len(), 4);
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    assert_eq!(branch.len(), 3);
    assert_eq!(branch.last().unwrap()["message"]["stopReason"], "error");
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_or_model_change_during_backoff_keeps_the_failed_branch_and_prevents_replay() {
    for cancel in [true, false] {
        let (origin, server) = server(vec![json!({"error":{"message":"server overloaded"}})]);
        let fixture = fixture(&origin, "disabled");
        let ready = Arc::new(Notify::new());
        let listener_ready = ready.clone();
        let host = fixture.host.clone();
        let files = fixture.files.clone();
        let run = tokio::spawn(async move {
            let mut turn = input();
            turn.retry.base_delay_ms = 60000;
            let listener = move |event: &SessionEvent| {
                if let SessionEvent::Recovery(value) = event
                    && value["type"] == "auto_retry_start"
                {
                    listener_ready.notify_one();
                }
            };
            host.run(turn, &Resolver::new(false), files, &listener)
                .await
        });
        ready.notified().await;
        assert!(!fixture.host.settled().unwrap());
        if cancel {
            fixture.host.cancel().unwrap();
        } else {
            let mut model = fixture.model.clone();
            model["id"] = json!("changed");
            fixture
                .host
                .select_model("change", "change", NOW, model, &ignore)
                .await
                .unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            result,
            Err(DomainError::Cancelled | DomainError::Conflict)
        ));
        assert_eq!(server.join().unwrap().len(), 1);
        let journal = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        assert!(
            ag1_core::task::branch_values(&journal)
                .unwrap()
                .iter()
                .any(|entry| entry["message"]["stopReason"] == "error")
        );
        assert!(fixture.host.settled().unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn model_change_after_retry_branch_commit_fences_the_resolved_replay() {
    let (origin, server) = server(vec![json!({"error":{"message":"server overloaded"}})]);
    let fixture = fixture(&origin, "disabled");
    let mut resolver = Resolver::new(true);
    resolver.pause_index = 1;
    let resolver = Arc::new(resolver);
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move {
        let mut turn = input();
        turn.retry.base_delay_ms = 1;
        host.run(turn, &*worker_resolver, files, &ignore).await
    });
    resolver.entered.notified().await;
    let mut model = fixture.model.clone();
    model["id"] = json!("changed");
    fixture
        .host
        .select_model("change", "change", NOW, model, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    assert!(matches!(run.await.unwrap(), Err(DomainError::Conflict)));
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert!(fixture.host.settled().unwrap());
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_preempts_retry_backoff_and_commits_its_summary() {
    let (origin, server) = server(vec![
        json!({"error":{"message":"server overloaded"}}),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let ready = Arc::new(Notify::new());
    let listener_ready = ready.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move {
        let mut turn = input();
        turn.retry.base_delay_ms = 60000;
        let listener = move |event: &SessionEvent| {
            if let SessionEvent::Recovery(value) = event
                && value["type"] == "auto_retry_start"
            {
                listener_ready.notify_one();
            }
        };
        host.run(turn, &Resolver::new(false), files, &listener)
            .await
    });
    ready.notified().await;
    let compact = compact_input();
    assert!(
        fixture
            .host
            .compact(compact, &Resolver::new(false), &ignore)
            .await
            .unwrap()
            .is_some()
    );
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(server.join().unwrap().len(), 2);
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_overflow_compacts_and_replays_once_while_preserving_failed_attempts() {
    for fails_again in [false, true] {
        let overflow =
            json!({"error":{"message":"prompt is too long: 50000 tokens > 32768 maximum"}});
        let (origin, server) = server(vec![
            overflow.clone(),
            stop(),
            if fails_again { overflow } else { stop() },
        ]);
        let fixture = fixture_seeded(&origin, "disabled", summary_history());
        let mut turn = input();
        turn.compaction = compact_input().settings;
        let result = fixture
            .host
            .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
            .await;
        assert_eq!(result.is_err(), fails_again);
        if let Ok(result) = result {
            assert_eq!(result.generations, 2);
        }
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1]["max_completion_tokens"], 2048);
        assert!(!requests[2].to_string().contains("prompt is too long"));
        let journal = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        let branch = ag1_core::task::branch_values(&journal).unwrap();
        assert_eq!(
            branch
                .iter()
                .filter(|entry| entry["type"] == "compaction")
                .count(),
            1
        );
        let raw_errors = journal
            .entries
            .iter()
            .filter(|entry| {
                String::from_utf8_lossy(&entry.normalized).contains("prompt is too long")
            })
            .count();
        assert_eq!(raw_errors, if fails_again { 2 } else { 1 });
        assert!(fixture.host.settled().unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_retry_preparation_restores_the_original_failed_branch_when_state_is_unchanged() {
    let (origin, server) = server(vec![json!({"error":{"message":"server overloaded"}})]);
    let fixture = fixture(&origin, "disabled");
    let mut resolver = Resolver::new(false);
    resolver.fail_index = Some(1);
    let mut turn = input();
    turn.retry.base_delay_ms = 1;
    assert!(matches!(
        fixture
            .host
            .run(turn, &resolver, fixture.files.clone(), &ignore)
            .await,
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(server.join().unwrap().len(), 1);
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(journal.revision, 5);
    assert_eq!(journal.entries.len(), 3);
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    assert_eq!(branch.len(), 3);
    assert_eq!(branch.last().unwrap()["message"]["stopReason"], "error");
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_http_status_takes_precedence_over_retryable_words_in_the_body() {
    for status in [401, 403, 404, 503] {
        let mut replies = vec![
            json!({"fixtureHttpStatus":status,"body":{"error":{"message":"server overloaded"}}}),
        ];
        if status == 503 {
            replies.push(stop());
        }
        let (origin, server) = server(replies);
        let fixture = fixture(&origin, "disabled");
        let mut turn = input();
        turn.retry.base_delay_ms = 1;
        let result = fixture
            .host
            .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
            .await;
        assert_eq!(result.is_ok(), status == 503);
        assert_eq!(
            server.join().unwrap().len(),
            if status == 503 { 2 } else { 1 }
        );
        assert!(fixture.host.settled().unwrap());
        assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
    }
}

fn pressure_history() -> Vec<Value> {
    (0..6).flat_map(|index|[
        json!({"role":"user","content":format!("Task {index}: {}","original evidence ".repeat(460))}),
        json!({"role":"assistant","content":[{"type":"text","text":format!("Result {index}")}],"stopReason":"stop"})
    ]).collect()
}

fn summary_history() -> Vec<Value> {
    vec![
        json!({"role":"user","content":"Original task and evidence ".repeat(200)}),
        json!({"role":"assistant","content":[{"type":"text","text":"Earlier findings are recorded."}],"stopReason":"stop"}),
        json!({"role":"user","content":"Continue with the current task."}),
        json!({"role":"assistant","content":[{"type":"text","text":"Current work has a partial investigation that needs continuation."}],"stopReason":"stop"}),
    ]
}
fn compact_input() -> ag1_platform::session::compaction::CompactInput {
    ag1_platform::session::compaction::CompactInput {
        command: "compact".into(),
        timestamp: NOW.into(),
        settings: ag1_core::context::compaction::Settings {
            enabled: true,
            reserve_tokens: 8192,
            keep_recent_tokens: 1,
        },
        focus: Some("Keep exact evidence and the current task.".into()),
        reason: ag1_platform::session::compaction::Reason::Manual,
        llm_timeout_ms: 900000,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_compaction_runs_split_summaries_sequentially_and_next_turn_uses_the_committed_context()
 {
    let (origin, server) = server(vec![stop(), stop(), stop()]);
    let fixture = fixture_seeded(&origin, "disabled", summary_history());
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let prepared = ag1_core::context::compaction::prepare_full(&branch, &compact_input().settings)
        .unwrap()
        .unwrap();
    assert_eq!(prepared["isSplitTurn"], true);
    assert!(
        !prepared["messagesToSummarize"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let result = fixture
        .host
        .compact(compact_input(), &Resolver::new(false), &ignore)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.context_epoch, 2);
    assert_eq!(result.journal_revision, 5);
    assert!(
        result.result["summary"]
            .as_str()
            .unwrap()
            .contains("**Turn Context (split turn):**")
    );
    assert_eq!(
        result.result["details"]["summaryOutputs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let original = result.result["details"]["originalJournal"]["id"]
        .as_str()
        .unwrap();
    let custody: Value = serde_json::from_slice(
        &fixture
            .store
            .page("session", "session", original, 0, 32768)
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert_eq!(custody["entries"].as_array().unwrap().len(), 4);
    assert!(
        fixture
            .host
            .compact(compact_input(), &Resolver::new(false), &ignore)
            .await
            .unwrap()
            .is_none()
    );
    fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests[..2] {
        assert_eq!(request["model"], "initial");
        assert!(request.get("tools").is_none());
        assert_eq!(
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|message| message["role"] != "system" && message["role"] != "developer")
                .count(),
            1
        );
        assert_eq!(request["max_completion_tokens"], 2048);
    }
    assert!(
        requests[0]
            .to_string()
            .contains("Original task and evidence")
    );
    assert!(
        requests[1]
            .to_string()
            .contains("Continue with the current task")
    );
    assert!(
        requests[2]
            .to_string()
            .contains("**Turn Context (split turn):**")
    );
    assert!(fixture.host.settled().unwrap());
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_model_rejects_a_late_summary_without_committing_an_epoch() {
    let (origin, server) = server(vec![]);
    server.join().unwrap();
    let fixture = fixture_seeded(&origin, "disabled", summary_history());
    let resolver = Arc::new(Resolver::new(true));
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let work = tokio::spawn(async move {
        host.compact(compact_input(), &*worker_resolver, &ignore)
            .await
    });
    resolver.entered.notified().await;
    let mut model = fixture.model.clone();
    model["id"] = json!("changed");
    fixture
        .host
        .select_model("change", "change", NOW, model, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    assert!(matches!(work.await.unwrap(), Err(DomainError::Conflict)));
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert!(!branch.iter().any(|entry| entry["type"] == "compaction"));
    assert!(fixture.host.settled().unwrap());
    assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_reserved_compaction_drains_preparation_and_rejects_competing_summaries() {
    let (origin, server) = server(vec![]);
    server.join().unwrap();
    let fixture = fixture_seeded(&origin, "disabled", summary_history());
    let resolver = Arc::new(Resolver::new(true));
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let work = tokio::spawn(async move {
        host.compact(compact_input(), &*worker_resolver, &ignore)
            .await
    });
    resolver.entered.notified().await;
    assert!(matches!(
        fixture
            .host
            .compact(compact_input(), &Resolver::new(false), &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
    fixture.host.cancel().unwrap();
    assert!(!fixture.host.settled().unwrap());
    resolver.release.notify_one();
    assert!(matches!(work.await.unwrap(), Err(DomainError::Cancelled)));
    assert!(fixture.host.settled().unwrap());
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert!(
        !ag1_core::task::branch_values(&snapshot)
            .unwrap()
            .iter()
            .any(|entry| entry["type"] == "compaction")
    );
    assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summary_deadline_bounds_resolution_and_queueing_without_releasing_another_generation() {
    for paused_resolution in [true, false] {
        let (origin, server) = server(vec![]);
        server.join().unwrap();
        let fixture = fixture_seeded(&origin, "disabled", summary_history());
        let ticket = fixture
            .scheduler
            .submit(
                &origin,
                ag1_core::OperationId::new("other-session").unwrap(),
            )
            .unwrap();
        let permit = fixture.scheduler.start(&ticket).unwrap().unwrap();
        let mut options = compact_input();
        options.llm_timeout_ms = 20;
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            fixture
                .host
                .compact(options, &Resolver::new(paused_resolution), &ignore),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(DomainError::TimedOut)));
        let status = fixture.scheduler.status(&origin).unwrap();
        assert_eq!(status.active, 1);
        assert_eq!(status.waiting, 0);
        assert_eq!(status.quarantined, 0);
        assert!(fixture.host.settled().unwrap());
        let snapshot = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        assert!(
            !ag1_core::task::branch_values(&snapshot)
                .unwrap()
                .iter()
                .any(|entry| entry["type"] == "compaction")
        );
        permit.terminal().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_interrupts_parent_preparation_without_restarting_its_cancelled_transport()
 {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture_seeded(&origin, "disabled", summary_history());
    let resolver = Arc::new(Resolver::new(true));
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let parent =
        tokio::spawn(async move { host.run(input(), &*worker_resolver, files, &ignore).await });
    resolver.entered.notified().await;
    let result = fixture
        .host
        .compact(compact_input(), &Resolver::new(false), &ignore)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.context_epoch, 2);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), parent)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(server.join().unwrap().len(), 1);
    assert!(fixture.host.settled().unwrap());
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pressure_compaction_retries_the_original_analysis_groups_then_resumes_the_parent() {
    let (origin, server) = server(vec![
        json!({"error":{"message":"prompt too long: max 10000 current 15000"}}),
        stop(),
        stop(),
        stop(),
    ]);
    let fixture = fixture_seeded(&origin, "disabled", pressure_history());
    fixture
        .host
        .select_tools("tools", "tools", NOW, vec![], &ignore)
        .await
        .unwrap();
    let mut model = fixture.model.clone();
    model["contextWindow"] = json!(10000);
    fixture
        .host
        .select_model("model", "model", NOW, model, &ignore)
        .await
        .unwrap();
    let mut turn = input();
    turn.compaction = compact_input().settings;
    let result = fixture
        .host
        .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 1);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].to_string().len() < requests[0].to_string().len());
    assert!(requests[0].to_string().contains("original evidence"));
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&snapshot).unwrap();
    let summary = branch
        .iter()
        .find(|entry| entry["type"] == "compaction")
        .unwrap();
    assert_eq!(summary["contextEpoch"], 2);
    assert_eq!(
        summary["details"]["summaryOutputs"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        requests[3]
            .to_string()
            .contains("Deterministic Working Memory")
    );
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_summary_failure_commits_a_rehydratable_checkpoint_and_preserves_transport_uncertainty()
 {
    for disconnect in [false, true] {
        let replies = if disconnect {
            vec![json!({"fixtureDisconnect":true})]
        } else {
            vec![
                json!({"error":{"message":"summary service unavailable"}}),
                stop(),
            ]
        };
        let (origin, server) = server(replies);
        let fixture = fixture_seeded(&origin, "disabled", summary_history());
        let mut options = compact_input();
        options.reason = ag1_platform::session::compaction::Reason::Threshold;
        let compacted = fixture
            .host
            .compact(options, &Resolver::new(false), &ignore)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(compacted.context_epoch, 2);
        assert_eq!(
            compacted.result["details"]["method"],
            "persistent_emergency_checkpoint"
        );
        assert!(
            compacted.result["summary"]
                .as_str()
                .unwrap()
                .contains("No historical semantic claim was inferred.")
        );
        let trajectory = compacted.result["details"]["trajectoryArtifactId"]
            .as_str()
            .unwrap();
        let raw = fixture
            .store
            .page("session", "session", trajectory, 0, 32768)
            .unwrap();
        let envelope: Value = serde_json::from_slice(&raw.bytes).unwrap();
        assert_eq!(envelope["kind"], "compaction-trajectory");
        assert!(
            envelope["messagesToSummarize"]
                .to_string()
                .contains("Original task and evidence")
        );
        let outputs = compacted.result["details"]["summaryOutputs"]
            .as_array()
            .unwrap();
        assert_eq!(outputs.len(), 1);
        let partial = fixture
            .store
            .page(
                "session",
                "session",
                outputs[0]["id"].as_str().unwrap(),
                0,
                32768,
            )
            .unwrap();
        assert!(
            String::from_utf8(partial.bytes)
                .unwrap()
                .contains(if disconnect {
                    "partial"
                } else {
                    "summary service unavailable"
                })
        );
        let snapshot = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        let restored = SessionRuntime::resume(
            SessionId::new("session").unwrap(),
            1,
            &snapshot,
            Configuration {
                model: fixture.model.clone(),
                thinking: "off".into(),
                tools: vec![],
                system_prompt: "Owned native fixture".into(),
                prompt: None,
                compiler: CompileOptions::default(),
            },
        )
        .unwrap();
        assert_eq!(restored.stamp().epoch(), 2);
        let result = fixture
            .host
            .run(
                input(),
                &Resolver::new(false),
                fixture.files.clone(),
                &ignore,
            )
            .await;
        if disconnect {
            assert!(matches!(result, Err(DomainError::Unavailable(_))));
            assert!(!fixture.host.settled().unwrap());
            assert_eq!(server.join().unwrap().len(), 1);
            let ticket = fixture.host.pending_transport().unwrap().unwrap();
            fixture.scheduler.reconcile(&ticket).unwrap();
            assert!(fixture.host.settled().unwrap());
        } else {
            result.unwrap();
            let requests = server.join().unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[1].to_string().contains(trajectory));
            assert!(fixture.host.settled().unwrap());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_objective_survives_new_and_repeated_instructions_without_persisting_request_addenda()
{
    let (origin, server) = server(vec![stop(), stop(), stop()]);
    let first = "Build the owned fixture application";
    let fixture = fixture_seeded(
        &origin,
        "disabled",
        vec![json!({"role":"user","content":first,"timestamp":0})],
    );
    for (index, instruction) in [
        "Continue the implementation",
        "Verify the result",
        "Verify the result",
    ]
    .iter()
    .enumerate()
    {
        let mut turn = input();
        turn.command = format!("instruction-{index}");
        turn.message["content"] = json!(instruction);
        fixture
            .host
            .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
            .await
            .unwrap();
    }
    let requests = server.join().unwrap();
    for request in &requests {
        let users = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .collect::<Vec<_>>();
        let latest = users.last().unwrap()["content"].as_str().unwrap();
        assert!(latest.contains(&format!("objective: {first}")));
        assert_eq!(
            latest
                .matches(ag1_core::runtime::context::WORKING_STATE)
                .count(),
            1
        );
        assert!(
            users[..users.len() - 1]
                .iter()
                .all(|m| !m.to_string().contains("AG1 REQUEST-LOCAL WORKING STATE"))
        );
    }
    let snapshot = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&snapshot).unwrap();
    let tasks = branch
        .iter()
        .filter(|e| e["customType"] == "task_state")
        .collect::<Vec<_>>();
    assert_eq!(
        tasks.len(),
        2,
        "repeating the instruction must not create a new task record"
    );
    assert_eq!(tasks[1]["data"]["objective"], first);
    assert_eq!(tasks[1]["data"]["latestInstruction"], "Verify the result");
    assert!(
        branch
            .iter()
            .all(|e| !e.to_string().contains("AG1 REQUEST-LOCAL WORKING STATE"))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_change_during_resolution_rebuilds_the_request_from_committed_state() {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture(&origin, "disabled");
    let resolver = Arc::new(Resolver::new(true));
    let worker = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move { host.run(input(), &*worker, files, &ignore).await });
    resolver.entered.notified().await;
    let task = ag1_core::task::TaskState::empty(NOW)
        .set(
            &json!({"objective":"Changed objective","nextAction":"Inspect the new evidence"}),
            NOW,
        )
        .unwrap();
    fixture
        .host
        .append_task_state("task-update", "task-update", NOW, task, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_string();
    assert!(request.contains("objective: Changed objective"));
    assert!(request.contains("next_action: Inspect the new evidence"));
    assert!(!request.contains("objective: Use the file tools"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_memory_and_source_changes_during_resolution_rebuild_before_transport() {
    use ag1_core::{memory::MemoryService, task::TaskState};
    use ag1_platform::source::SourceService;
    for change in ["memory", "task-source", "memory-source"] {
        let (origin, server) = server(vec![stop()]);
        let fixture = fixture(&origin, "disabled");
        std::fs::write(fixture.root.join("evidence.txt"), "before").unwrap();
        let reference = SourceService::new(fixture.workspace.clone())
            .capture("evidence.txt")
            .unwrap()
            .reference()
            .reference();
        if change == "task-source" {
            let (task, _) = TaskState::empty(NOW).set(&json!({"objective":"Use evidence"}),NOW).unwrap().record(
                &json!({"kind":"FACT","statement":"Owned fixture evidence","sources":[reference]}), "ts_00000001", NOW).unwrap();
            fixture
                .host
                .append_task_state("seed-task", "seed-task", NOW, task, &ignore)
                .await
                .unwrap();
        }
        if change == "memory-source" {
            MemoryService::mutate(
                fixture.store.as_ref(),
                fixture.workspace.id().as_str(),
                None,
                |memory| {
                    Ok(memory
                        .add(
                            "fact",
                            "Owned fixture evidence",
                            1,
                            "core",
                            &json!({"sources":[reference]}),
                        )?
                        .store)
                },
            )
            .unwrap();
        }
        let resolver = Arc::new(Resolver::new(true));
        let worker = resolver.clone();
        let host = fixture.host.clone();
        let files = fixture.files.clone();
        let run = tokio::spawn(async move { host.run(input(), &*worker, files, &ignore).await });
        resolver.entered.notified().await;
        if change == "memory" {
            MemoryService::mutate(
                fixture.store.as_ref(),
                fixture.workspace.id().as_str(),
                None,
                |memory| {
                    Ok(memory
                        .add(
                            "decision",
                            "Preserve the existing tuned runtime",
                            2,
                            "core",
                            &json!({}),
                        )?
                        .store)
                },
            )
            .unwrap();
        } else {
            // Same length, different bytes: metadata alone cannot establish freshness.
            std::fs::write(fixture.root.join("evidence.txt"), "after!").unwrap();
        }
        resolver.release.notify_one();
        run.await.unwrap().unwrap();
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 2, "{change}");
        let requests = server.join().unwrap();
        assert_eq!(
            requests.len(),
            1,
            "the stale prepared request must not reach transport"
        );
        let request = requests[0].to_string();
        match change {
            "memory" => assert!(request.contains("Preserve the existing tuned runtime")),
            "task-source" => assert!(
                request.contains("stale_source=true") && request.contains("[STALE current=")
            ),
            "memory-source" => assert!(request.contains("historical stale/missing memory")),
            _ => unreachable!(),
        }
        assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 0);
        assert!(fixture.host.settled().unwrap());
    }
}

fn provider_outputs(store: &Store) -> Vec<Value> {
    store
        .search("session", "\"kind\":\"runtime-provider-output\"", 100)
        .unwrap()
        .into_iter()
        .map(|hit| {
            let mut bytes = vec![];
            let mut offset = 0;
            loop {
                let page = store
                    .page("session", &hit.origin, &hit.artifact, offset, 32768)
                    .unwrap();
                bytes.extend(page.bytes);
                match page.next_offset {
                    Some(next) => offset = next,
                    None => break,
                }
            }
            serde_json::from_slice(&bytes).unwrap()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_output_is_retained_before_journal_limits_or_identity_validation() {
    for huge in [false, true] {
        let text = "owned provider evidence ".repeat(if huge { 200000 } else { 1 });
        let response = if huge {
            let mut frames = text.as_bytes().chunks(65536).map(|chunk|json!({"choices":[{"delta":{"content":std::str::from_utf8(chunk).unwrap()}}]})).collect::<Vec<_>>();
            frames.push(json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":2}}));
            json!({"fixtureFrames":frames})
        } else {
            calls(vec![(
                &"x".repeat(129),
                "write",
                json!({"path":"forbidden.txt","content":text}),
            )])
        };
        let (origin, server) = server(vec![response]);
        let fixture = fixture(&origin, "disabled");
        let mut resolver = Resolver::new(false);
        resolver.output_limit = 8 * 1024 * 1024;
        let result = fixture
            .host
            .run(input(), &resolver, fixture.files.clone(), &ignore)
            .await;
        assert!(
            matches!(
                result,
                Err(DomainError::LimitExceeded(_)) | Err(DomainError::InvalidInput(_))
            ),
            "{:?}",
            result.as_ref().err()
        );
        assert_eq!(server.join().unwrap().len(), 1);
        assert!(!fixture.root.join("forbidden.txt").exists());
        assert!(
            fixture.host.settled().unwrap(),
            "transport and accepted work have drained; persistence still blocks the next turn"
        );
        assert!(
            fixture
                .host
                .run(input(), &resolver, fixture.files.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        let stored = provider_outputs(&fixture.store);
        assert_eq!(stored.len(), 1);
        assert!(stored[0]["error"].is_null());
        assert_eq!(stored[0]["snapshot"]["settled"], true);
        if huge {
            assert_eq!(stored[0]["snapshot"]["content"][0]["text"], text);
        } else {
            assert_eq!(stored[0]["snapshot"]["content"][0]["id"], "x".repeat(129));
            assert_eq!(
                stored[0]["snapshot"]["content"][0]["arguments"]["content"],
                text
            );
        }
        assert_eq!(
            fixture
                .store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap()
                .entries
                .len(),
            2
        );
        assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
        let Fixture {
            _directory: directory,
            host,
            files,
            store,
            ..
        } = fixture;
        drop(host);
        drop(files);
        drop(store);
        let reopened = Store::open(&directory.path().join("store")).unwrap();
        assert_eq!(provider_outputs(&reopened), stored);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn partial_provider_snapshot_is_retained_before_error_normalization() {
    let (origin, server) = server(vec![json!({"fixtureDisconnect":true})]);
    let fixture = fixture(&origin, "disabled");
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
    server.join().unwrap();
    let outputs = provider_outputs(&fixture.store);
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0]["snapshot"]["content"][0]["text"], "partial");
    assert!(!outputs[0]["error"].is_null());
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let message = &branch.last().unwrap()["message"];
    assert_eq!(message["stopReason"], "error");
    assert!(
        message["nativeOutputArtifact"]["id"]
            .as_str()
            .unwrap()
            .starts_with("art_")
    );
    assert_ne!(outputs[0]["snapshot"]["error"], message["errorMessage"]);
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 1);
    fixture
        .scheduler
        .reconcile(&fixture.host.pending_transport().unwrap().unwrap())
        .unwrap();
    assert!(fixture.host.settled().unwrap());
}

struct ArmedStorageFault {
    point: ag1_storage::Boundary,
    armed: AtomicBool,
}
struct RetirementFault {
    point: ag1_storage::Boundary,
    remaining: AtomicUsize,
}
impl ag1_storage::FaultInjector for RetirementFault {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point == self.point
            && self
                .remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                == Ok(1)
        {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_job_quiescence_stops_owned_process_and_retains_cancelled_output_across_metadata_faults()
 {
    use ag1_platform::session::ToolRunner;
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![
            calls(vec![(
                "start",
                "job",
                json!({"action":"start","command":"printf 'retirement evidence\\n'; printf '%s' \"$$\" > owned.pid; exec /usr/bin/sleep 30"}),
            )]),
            stop(),
        ]);
        let fault = Arc::new(RetirementFault {
            point,
            remaining: AtomicUsize::new(0),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        let runner = native_adapters(&fixture, &["job"], None).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), runner.clone(), &ignore)
            .await
            .unwrap();
        server.join().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !fixture.root.join("owned.pid").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(fixture.root.join("owned.pid"))
            .unwrap()
            .parse::<u32>()
            .unwrap();
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
        let scope = ag1_core::tools::registry::Scope {
            workspace: fixture.workspace.id().as_str().into(),
            session: "session".into(),
            epoch: 1,
            revision: fixture.host.journal_revision().unwrap(),
        };
        let wrong = ag1_core::tools::registry::Scope {
            session: "foreign".into(),
            ..scope.clone()
        };
        assert!(runner.quiesce(&wrong, "denied-retirement").is_err());
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());
        // Small output: raw custody, rendered result custody, then job metadata.
        fault.remaining.store(3, Ordering::SeqCst);
        assert!(runner.quiesce(&scope, "retire-job").is_err());
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        assert!(
            std::fs::read_dir(fixture._directory.path().join("tool-spools"))
                .unwrap()
                .next()
                .is_some(),
            "failed acknowledgement retains owned output spool"
        );
        runner.quiesce(&scope, "retire-job").unwrap();
        runner.quiesce(&scope, "retire-job").unwrap();
        assert!(
            std::fs::read_dir(fixture._directory.path().join("tool-spools"))
                .unwrap()
                .next()
                .is_none()
        );
        let jobs = runner.adapters.jobs.as_ref().unwrap();
        assert_eq!(
            jobs.notices().unwrap().len(),
            1,
            "retirement does not acknowledge parent notice"
        );
        let catalog = fixture
            .store
            .artifact_catalog("session", None, 100, Some("job-result"))
            .unwrap();
        let results: Vec<Value> = catalog
            .iter()
            .filter(|record| record.record["kind"] == "job-result")
            .map(|record| {
                let page = fixture
                    .store
                    .page(
                        "session",
                        "session",
                        record.record["id"].as_str().unwrap(),
                        0,
                        32768,
                    )
                    .unwrap();
                serde_json::from_slice(&page.bytes).unwrap()
            })
            .collect();
        assert!(!results.is_empty());
        for result in results {
            assert_eq!(result["details"]["termination"], "cancelled");
            assert!(
                result["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("retirement evidence")
            );
            let raw = &result["details"]["rawOutputArtifact"];
            let page = fixture
                .store
                .page("session", "session", raw["id"].as_str().unwrap(), 0, 32768)
                .unwrap();
            let snapshot: Value = serde_json::from_slice(&page.bytes).unwrap();
            assert_eq!(snapshot["complete"], true);
        }
    }
}
impl ag1_storage::FaultInjector for ArmedStorageFault {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point == self.point && self.armed.swap(false, Ordering::SeqCst) {
            Err(ag1_storage::StorageError::Injected(point))
        } else {
            Ok(())
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_provider_output_custody_blocks_journal_publication_and_following_requests() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let (origin, server) = server(vec![calls(vec![(
            "call-one",
            "write",
            json!({"path":"forbidden.txt","content":"not executed"}),
        )])]);
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        let listener = |event: &SessionEvent| {
            if matches!(event, SessionEvent::Provider(_)) {
                fault.armed.store(true, Ordering::SeqCst);
            }
        };
        assert!(
            fixture
                .host
                .run(
                    input(),
                    &Resolver::new(false),
                    fixture.files.clone(),
                    &listener
                )
                .await
                .is_err()
        );
        assert_eq!(server.join().unwrap().len(), 1);
        assert!(!fixture.root.join("forbidden.txt").exists());
        assert!(
            fixture.host.settled().unwrap(),
            "transport and accepted work have drained; persistence still blocks the next turn"
        );
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
        assert_eq!(fixture.host.journal_revision().unwrap(), 2);
        let stored = provider_outputs(&fixture.store);
        assert_eq!(
            stored.len(),
            usize::from(point == ag1_storage::Boundary::BeforeAck)
        );
        assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 0);
        assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loaded_resources_and_active_tool_routing_rebuild_during_request_preparation() {
    use ag1_platform::{
        resource_fs::{ResourceFs, ScanControl},
        resources::{Config, Paths, ResourceLoader},
    };
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture(&origin, "disabled");
    let root = fixture._directory.path();
    std::fs::create_dir(root.join("agent")).unwrap();
    std::fs::create_dir(root.join("home")).unwrap();
    std::fs::create_dir_all(fixture.root.join(".ag1/skills/owned-skill")).unwrap();
    std::fs::write(
        fixture.root.join("AGENTS.md"),
        "Original project instructions.",
    )
    .unwrap();
    std::fs::write(
        fixture.root.join(".ag1/skills/owned-skill/SKILL.md"),
        "---\nname: owned-skill\ndescription: Inspect the owned fixture sources.\n---\nOwned body.",
    )
    .unwrap();
    let resources_root = WorkspaceService::new().unwrap().register(root).unwrap();
    let mut loader = ResourceLoader::new(
        ResourceFs::new(vec![resources_root]).unwrap(),
        Config {
            cwd: fixture.root.clone(),
            agent_dir: root.join("agent"),
            home: root.join("home"),
            additional: Paths::default(),
            disabled: vec![],
            installed: vec![],
            system: None,
            append_system: Some("Host resource addendum.".into()),
        },
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    loader.reload(0, &control).unwrap();
    fixture
        .host
        .install_resources(
            "resources-initial",
            NOW,
            loader.snapshot().unwrap(),
            (*fixture.files.registry).clone(),
            &ignore,
        )
        .await
        .unwrap();
    let resolver = Arc::new(Resolver::new(true));
    let worker = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move { host.run(input(), &*worker, files, &ignore).await });
    resolver.entered.notified().await;
    std::fs::write(
        fixture.root.join("AGENTS.md"),
        "Reloaded project instructions.",
    )
    .unwrap();
    loader.reload(1, &control).unwrap();
    fixture
        .host
        .install_resources(
            "resources-reloaded",
            NOW,
            loader.snapshot().unwrap(),
            (*fixture.files.registry).clone(),
            &ignore,
        )
        .await
        .unwrap();
    let tools = fixture
        .files
        .registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "read")
        .collect();
    fixture
        .host
        .select_tools("only-read", "only-read", NOW, tools, &ignore)
        .await
        .unwrap();
    let mut model = fixture.model.clone();
    model["provider"] = json!("llamacpp");
    fixture
        .host
        .select_model("local-prompt", "local-prompt", NOW, model, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    let system = requests[0]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "system")
        .unwrap()["content"]
        .as_str()
        .unwrap();
    assert!(system.contains("Reloaded project instructions."));
    assert!(!system.contains("Original project instructions."));
    assert!(system.contains("Host resource addendum."));
    assert!(system.contains("owned-skill"));
    assert!(system.contains("- inspect files: read"));
    assert!(!system.contains("- change files:"));
    assert!(system.contains(fixture.root.to_str().unwrap()));
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 1);
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let captures = branch
        .iter()
        .filter(|entry| entry["customType"] == "ag1_prompt_resources")
        .collect::<Vec<_>>();
    assert_eq!(captures.len(), 2);
    assert_eq!(
        captures[1]["data"]["fingerprint"],
        loader.snapshot().unwrap().fingerprint
    );
    // Resource refresh obeys the same admission fence as model/tool changes.
    let registry = ag1_platform::session::registry::RuntimeRegistry::durable(
        ag1_platform::session::registry::PreparedRuntime::from_shared(
            fixture.host.clone(),
            resolver,
            fixture.files.clone(),
        ),
        None,
        "select-resource-owner".into(),
    )
    .unwrap();
    let revision = fixture.host.journal_revision().unwrap();
    assert!(matches!(
        fixture
            .host
            .install_resources(
                "resources-frozen",
                NOW,
                loader.snapshot().unwrap(),
                (*fixture.files.registry).clone(),
                &ignore
            )
            .await,
        Err(DomainError::Conflict)
    ));
    assert_eq!(fixture.host.journal_revision().unwrap(), revision);
    registry.reconcile_selection().await.unwrap();
}

async fn native_adapters(
    fixture: &Fixture,
    names: &[&str],
    prefix: Option<String>,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    native_adapters_configured(fixture, names, prefix, Some(1024 * 1024 * 1024)).await
}
async fn native_adapters_configured(
    fixture: &Fixture,
    names: &[&str],
    prefix: Option<String>,
    address_space: Option<u64>,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{
        git::GitService,
        git_tools::GitTools,
        job_tools::JobService,
        process::ProcessSupervisor,
        search_tools::SearchService,
        session::adapters::NativeAdapters,
        shell_tools::{Sandbox, ShellConfig, ShellService},
    };
    let spools = fixture._directory.path().join("tool-spools");
    std::fs::create_dir(&spools).unwrap();
    let processes =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let shell = Arc::new(
        ShellService::new(
            fixture.workspace.clone(),
            processes.clone(),
            &spools,
            ShellConfig {
                executable: "/bin/bash".into(),
                env: std::collections::BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                command_prefix: prefix,
                command_template: None,
                sandbox: Sandbox::Off,
                bubblewrap: None,
                output_bytes: 2 * 1024 * 1024,
                address_space,
            },
        )
        .unwrap(),
    );
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: 0,
    };
    let adapters = NativeAdapters {
        search: Some(Arc::new(
            SearchService::new(
                fixture.workspace.clone(),
                processes,
                "/usr/bin/rg".as_ref(),
                "/usr/bin/fd".as_ref(),
                &spools,
            )
            .unwrap(),
        )),
        git: Some(Arc::new(GitTools::new(
            fixture.workspace.clone(),
            GitService::new(
                env!("CARGO_BIN_EXE_ag1-process-host").as_ref(),
                "/usr/bin/git".as_ref(),
                &spools,
            )
            .unwrap(),
        ))),
        jobs: Some(Arc::new(
            JobService::new(
                fixture.store.clone(),
                shell.clone(),
                Arc::new(FixedClock),
                scope,
                "native-job-owner",
            )
            .unwrap(),
        )),
        shell: Some(shell),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    let names = names
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    registry
        .activate(&names, &adapters.available_names().into_iter().collect())
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| names.iter().any(|name| tool["name"] == *name))
        .collect();
    fixture
        .host
        .select_tools("native-adapters", "native-adapters", NOW, tools, &ignore)
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_shell_search_listing_and_git_diff_share_the_session_tool_lifecycle() {
    let (origin, server) = server(vec![
        calls(vec![(
            "shell-write",
            "bash",
            json!({"command":"printf 'native needle\n' > output.txt; printf 'changed\n' > tracked.txt"}),
        )]),
        calls(vec![
            ("list-files", "ls", json!({"path":"."})),
            (
                "search-file",
                "grep",
                json!({"pattern":"needle","path":"output.txt"}),
            ),
            ("find-file", "find", json!({"pattern":"*.txt","path":"."})),
            ("diff-file", "diff", json!({"from":"tracked.txt"})),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("tracked.txt"), "initial\n").unwrap();
    for args in [&["init", "--quiet"][..], &["add", "tracked.txt"][..]] {
        let output = std::process::Command::new("/usr/bin/git")
            .args(args)
            .current_dir(&fixture.root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", fixture._directory.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let runner = native_adapters(&fixture, &["bash", "ls", "grep", "find", "diff"], None).await;
    let result = fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 3);
    let requests = server.join().unwrap();
    let results = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 5);
    for (result, expected) in results.iter().zip([
        "shell-write",
        "list-files",
        "search-file",
        "find-file",
        "diff-file",
    ]) {
        assert_eq!(result["tool_call_id"], expected);
    }
    assert!(
        results[1]["content"]
            .as_str()
            .unwrap()
            .contains("output.txt")
    );
    assert!(
        results[2]["content"]
            .as_str()
            .unwrap()
            .contains("native needle")
    );
    assert!(
        results[3]["content"]
            .as_str()
            .unwrap()
            .contains("output.txt")
    );
    assert!(results[4]["content"].as_str().unwrap().contains("changed"));
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let outcomes = branch
        .iter()
        .filter(|entry| entry["message"]["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert!(
        outcomes
            .iter()
            .all(|entry| entry["message"]["isError"] == false)
    );
    assert!(fixture.host.settled().unwrap());
    assert!(
        std::fs::read_dir(fixture._directory.path().join("tool-spools"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_shell_prefix_is_authorized_as_the_actual_command_before_execution() {
    let (origin, server) = server(vec![
        calls(vec![("prefixed", "bash", json!({"command":"pwd"}))]),
        stop(),
    ]);
    let fixture = fixture(&origin, "planning");
    let runner = native_adapters(
        &fixture,
        &["bash"],
        Some("printf 'blocked' > forbidden.txt".into()),
    )
    .await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert!(!fixture.root.join("forbidden.txt").exists());
    assert_eq!(server.join().unwrap().len(), 2);
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let result = &branch
        .iter()
        .find(|entry| entry["message"]["role"] == "toolResult")
        .unwrap()["message"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["details"]["rejectedBeforeExecution"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_background_job_start_and_log_retain_output_without_relaunching_the_job() {
    let (origin, server) = server(vec![
        calls(vec![(
            "job-start",
            "job",
            json!({"action":"start","command":"printf 'ran once\n' >> executions.txt; printf 'owned job evidence\n'"}),
        )]),
        json!({"fixtureJobAction":"log"}),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_adapters(&fixture, &["job"], None).await;
    let jobs = runner.adapters.jobs.as_ref().unwrap().clone();
    let mut resolver = Resolver::new(true);
    resolver.pause_index = 1;
    let resolver = Arc::new(resolver);
    let worker = resolver.clone();
    let host = fixture.host.clone();
    let run = tokio::spawn(async move { host.run(input(), &*worker, runner, &ignore).await });
    resolver.entered.notified().await;
    let mut artifacts = ag1_storage::context::ContextStore::new(
        &fixture.store,
        SessionId::new("session").unwrap(),
        "session",
        "job-observer",
        NOW,
        Default::default(),
    )
    .unwrap();
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        jobs.refresh(&mut artifacts).unwrap();
        if !jobs.notices().unwrap().is_empty() {
            break;
        }
        assert!(Instant::now() < limit, "owned job did not exit");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    let requests = server.join().unwrap();
    let last = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .unwrap();
    assert_eq!(last["tool_call_id"], "job-log");
    assert!(
        last["content"]
            .as_str()
            .unwrap()
            .contains("owned job evidence")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("executions.txt")).unwrap(),
        "ran once\n"
    );
    assert_eq!(
        jobs.notices().unwrap().len(),
        1,
        "notice remains pending until the session durably acknowledges it"
    );
    assert!(fixture.host.settled().unwrap());
    assert!(
        std::fs::read_dir(fixture._directory.path().join("tool-spools"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_shell_cancellation_drains_the_owned_process_and_commits_its_final_result() {
    let (origin, server) = server(vec![calls(vec![(
        "cancel-shell",
        "bash",
        json!({"command":"printf ready > started.txt; /usr/bin/tail -f /dev/null"}),
    )])]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_adapters(&fixture, &["bash"], None).await;
    let host = fixture.host.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), runner, &ignore)
            .await
    });
    let limit = Instant::now() + Duration::from_secs(5);
    while !fixture.root.join("started.txt").exists() {
        assert!(Instant::now() < limit, "owned shell did not start");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    fixture.host.cancel().unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(server.join().unwrap().len(), 1);
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let results = branch
        .iter()
        .filter(|entry| entry["message"]["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["message"]["isError"], true);
    assert_eq!(results[0]["message"]["details"]["termination"], "cancelled");
    assert!(results[0]["message"]["details"]["rawOutputArtifact"]["id"].is_string());
    assert!(fixture.host.settled().unwrap());
    assert!(
        std::fs::read_dir(fixture._directory.path().join("tool-spools"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaying_a_settled_shell_receipt_does_not_repeat_unknown_filesystem_effects() {
    use ag1_platform::session::ToolRunner;
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let runner = native_adapters(&fixture, &["bash"], None).await;
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let revision = std::sync::atomic::AtomicU64::new(scope.revision);
    let cancel = AtomicBool::new(false);
    let call = ag1_core::runtime::session::PendingCall {
        id: "once".into(),
        name: "bash".into(),
        block: json!({"type":"toolCall","id":"once","name":"bash","arguments":{"command":"printf 'once\n' >> invocations.txt"}}),
    };
    for replayed in [false, true] {
        let outcomes = runner
            .execute(
                &scope,
                std::slice::from_ref(&call),
                &ag1_platform::session::ToolSelection {
                    journal: None,
                    names: vec!["bash".into()],
                    research_profile: ag1_core::tools::research::profile(90000., 0.),
                    loaded_memory: Default::default(),
                },
                "owned-receipt",
                &cancel,
                &revision,
            )
            .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].outcome.replayed, replayed);
        assert_eq!(
            outcomes[0].outcome.record.effect,
            ag1_core::tools::executor::Effect::Unknown
        );
        assert!(outcomes[0].execution_settled);
    }
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("invocations.txt")).unwrap(),
        "once\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn activating_an_available_native_adapter_during_preparation_uses_the_current_selection() {
    let (origin, server) = server(vec![
        calls(vec![(
            "new-shell",
            "bash",
            json!({"command":"printf activated > activated.txt"}),
        )]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_adapters(&fixture, &["read"], None).await;
    assert!(!runner.registry.active_names().contains(&"bash".into()));
    let tools = runner
        .registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "bash")
        .collect();
    let resolver = Arc::new(Resolver::new(true));
    let worker = resolver.clone();
    let host = fixture.host.clone();
    let run = tokio::spawn(async move { host.run(input(), &*worker, runner, &ignore).await });
    resolver.entered.notified().await;
    fixture
        .host
        .select_tools("activate-shell", "activate-shell", NOW, tools, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    run.await.unwrap().unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 3);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(requests[0]["tools"][0]["function"]["name"], "bash");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("activated.txt")).unwrap(),
        "activated"
    );
}

async fn native_web_runner(
    fixture: &Fixture,
    origin: &str,
    epoch: u64,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{
        session::adapters::{NativeAdapters, NativeWeb},
        web_http::{WebHttp, WebPolicy},
        web_tools::{SearchConfig, SearchEndpoints, SearchService, WebService},
    };
    let fetch = Arc::new(
        WebService::new(
            WebHttp::new(WebPolicy::with_private_origins(&[origin.into()]).unwrap()),
            ag1_core::tools::registry::Scope {
                workspace: fixture.workspace.id().as_str().into(),
                session: "session".into(),
                epoch,
                revision: 0,
            },
            "native-web",
            Default::default(),
        )
        .unwrap(),
    );
    let search = Arc::new(
        SearchService::new(
            fetch.clone(),
            SearchConfig {
                mode: "duckduckgo".into(),
                brave: None,
                endpoints: SearchEndpoints {
                    html: format!("{origin}/html"),
                    lite: format!("{origin}/lite"),
                    brave: format!("{origin}/brave"),
                },
            },
            "native-search",
        )
        .unwrap(),
    );
    let adapters = NativeAdapters {
        web: Some(Arc::new(NativeWeb {
            fetch,
            search: Some(search),
            runtime: tokio::runtime::Handle::current(),
        })),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    let names = ["webSearch".into(), "webFetch".into()];
    registry
        .activate(&names, &adapters.available_names().into_iter().collect())
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| names.iter().any(|name| tool["name"] == *name))
        .collect();
    fixture
        .host
        .select_tools("native-web-tools", "native-web-tools", NOW, tools, &ignore)
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
fn read_web_request(socket: &mut TcpStream) -> String {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = vec![];
    loop {
        let mut byte = [0];
        socket.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return String::from_utf8(bytes).unwrap();
        }
        assert!(bytes.len() < 16384);
    }
}
fn web_accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((socket, _)) => return socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < limit, "expected owned web request");
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("{error}"),
        }
    }
}
fn session_tool_results(fixture: &Fixture) -> Vec<Value> {
    ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap()
    .into_iter()
    .filter(|entry| entry["message"]["role"] == "toolResult")
    .map(|entry| entry["message"].clone())
    .collect()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_web_search_and_fetch_retain_evidence_and_use_current_context_pressure() {
    use ag1_core::{context::ContextArtifactPort, tools::executor::ExecutionArtifacts};
    use base64::Engine;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let web_origin = format!("http://{}", listener.local_addr().unwrap());
    let document_url = format!("{web_origin}/article");
    let search_body = format!(
        "<a class=\"result__a\" href=\"{document_url}\">Native reference</a><a class=\"result__snippet\">Owned native web evidence.</a>"
    );
    let document_body = "<html><article><h1>Native reference</h1><p>Owned document evidence survives restart.</p></article></html>";
    let expected = [search_body.clone(), document_body.to_owned()];
    let web = thread::spawn(move || {
        let mut requests = vec![];
        for body in [search_body, document_body.to_owned()] {
            let mut socket = web_accept(&listener);
            requests.push(read_web_request(&mut socket));
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    let mut fetch_call = calls(vec![(
        "fetch-native",
        "webFetch",
        json!({"url":document_url}),
    )]);
    // The current 90k model is under pressure only on the second batch. The
    // fixture supplies usage through the real provider reducer and journal.
    fetch_call["usage"] = json!({"prompt_tokens":75000,"completion_tokens":10});
    let (origin, model) = server(vec![
        calls(vec![(
            "search-native",
            "webSearch",
            json!({"query":"native reference"}),
        )]),
        fetch_call,
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let mut larger_model = fixture.model.clone();
    larger_model["contextWindow"] = json!(90000);
    fixture
        .host
        .select_model("web-model", "web-model", NOW, larger_model, &ignore)
        .await
        .unwrap();
    let runner = native_web_runner(&fixture, &web_origin, 1).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let requests = web.join().unwrap();
    assert!(requests[0].starts_with("GET /html?"));
    assert!(requests[1].starts_with("GET /article "));
    let model_requests = model.join().unwrap();
    assert!(
        model_requests[2]["messages"]
            .to_string()
            .contains("Owned document evidence")
    );
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["toolCallId"], "search-native");
    assert_eq!(results[1]["toolCallId"], "fetch-native");
    assert_eq!(results[0]["details"]["compressionMode"], "normal");
    assert_eq!(results[1]["details"]["compressionMode"], "ultra");
    let ids = [
        results[0]["details"]["networkEvidence"]["artifactIds"][0]
            .as_str()
            .unwrap()
            .to_owned(),
        results[1]["details"]["networkEvidence"]["artifactId"]
            .as_str()
            .unwrap()
            .to_owned(),
    ];
    assert!(fixture.host.settled().unwrap());
    let Fixture {
        _directory: directory,
        host,
        files,
        store,
        ..
    } = fixture;
    drop(host);
    drop(files);
    drop(store);
    let reopened = Store::open(&directory.path().join("store")).unwrap();
    let artifacts = ag1_storage::context::ContextStore::new(
        &reopened,
        SessionId::new("session").unwrap(),
        "session",
        "web-reader",
        NOW,
        Default::default(),
    )
    .unwrap();
    for (id, body) in ids.iter().zip(expected) {
        let metadata = artifacts.get(id).unwrap();
        let raw = artifacts.read_json(&metadata).unwrap();
        assert_eq!(raw["trust"], "untrusted");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(raw["bodyBase64"].as_str().unwrap())
                .unwrap(),
            body.as_bytes()
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_web_cancellation_drains_http_before_committing_the_tool_result() {
    for tool in ["webFetch", "webSearch"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let web_origin = format!("http://{}", listener.local_addr().unwrap());
        let entered = Arc::new(Notify::new());
        let signal = entered.clone();
        let web = thread::spawn(move || {
            let mut socket = web_accept(&listener);
            read_web_request(&mut socket);
            signal.notify_one();
            let mut byte = [0];
            // A drained cancelled fetch closes the owned socket without waiting for
            // the fixture to release a response or the five-second tool deadline.
            assert_eq!(socket.read(&mut byte).unwrap(), 0);
        });
        let (origin, model) = server(vec![calls(vec![(
            "cancel-web",
            tool,
            if tool == "webFetch" {
                json!({"url":format!("{web_origin}/slow")})
            } else {
                json!({"query":"native cancellation"})
            },
        )])]);
        let fixture = fixture(&origin, "disabled");
        let runner = native_web_runner(&fixture, &web_origin, 1).await;
        let host = fixture.host.clone();
        let run = tokio::spawn(async move {
            host.run(input(), &Resolver::new(false), runner, &ignore)
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .unwrap();
        fixture.host.cancel().unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .unwrap()
                .unwrap(),
            Err(DomainError::Cancelled)
        ));
        web.join().unwrap();
        assert_eq!(model.join().unwrap().len(), 1);
        let results = session_tool_results(&fixture);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0]["details"]["error"],
            if tool == "webFetch" {
                "Fetch cancelled."
            } else {
                "Web search cancelled."
            }
        );
        assert!(fixture.host.settled().unwrap());
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_web_rejects_a_service_from_another_epoch_without_network_access() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let web_origin = format!("http://{}", listener.local_addr().unwrap());
    let (origin, model) = server(vec![
        calls(vec![(
            "stale-web",
            "webFetch",
            json!({"url":format!("{web_origin}/article")}),
        )]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_web_runner(&fixture, &web_origin, 2).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(model.join().unwrap().len(), 2);
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["isError"], true);
    assert!(
        results[0]["content"]
            .to_string()
            .contains("PermissionDenied")
    );
    assert!(fixture.host.settled().unwrap());
}

async fn native_memory_runner(
    fixture: &Fixture,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{memory_tools::MemoryTools, session::adapters::NativeAdapters};
    let adapters = NativeAdapters {
        memory: Some(Arc::new(MemoryTools::new(
            fixture.workspace.clone(),
            fixture.store.clone(),
        ))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    let names = ["memory".into(), "write".into()];
    registry
        .activate(&names, &adapters.available_names().into_iter().collect())
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| names.iter().any(|name| tool["name"] == *name))
        .collect();
    fixture
        .host
        .select_tools(
            "native-memory-tools",
            "native-memory-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_actions_feed_the_next_request_and_report_actual_injected_ids() {
    let (origin, model) = server(vec![
        calls(vec![(
            "memory-add",
            "memory",
            json!({"action":"add","kind":"decision","priority":"core","text":"Preserve tuned native runtimes"}),
        )]),
        calls(vec![
            ("memory-list", "memory", json!({"action":"list"})),
            (
                "memory-search",
                "memory",
                json!({"action":"search","query":"native runtimes"}),
            ),
        ]),
        calls(vec![
            (
                "memory-update",
                "memory",
                json!({"action":"update","id":"m1","text":"Preserve native model settings"}),
            ),
            (
                "memory-add-two",
                "memory",
                json!({"action":"add","kind":"fact","text":"Temporary native note"}),
            ),
        ]),
        calls(vec![(
            "memory-remove",
            "memory",
            json!({"action":"remove","id":"m2"}),
        )]),
        calls(vec![("memory-clear", "memory", json!({"action":"clear"}))]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_memory_runner(&fixture).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let requests = model.join().unwrap();
    let system = |index: usize| {
        requests[index]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "system")
            .map(|m| m["content"].as_str().unwrap())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(!system(0).contains("Preserve tuned native runtimes"));
    assert!(system(1).contains("Preserve tuned native runtimes"));
    assert!(system(3).contains("Preserve native model settings"));
    assert!(!system(3).contains("Preserve tuned native runtimes"));
    assert!(!system(5).contains("Preserve native model settings"));
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 7);
    assert!(results.iter().all(|result| result["isError"] != true));
    assert!(results[1]["content"].to_string().contains("last turn: 1."));
    assert!(
        results[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Preserve tuned native runtimes *")
    );
    assert_eq!(results[2]["details"]["count"], 1);
    assert_eq!(results[6]["content"][0]["text"], "Memory cleared.");
    let (memory, _) =
        ag1_core::memory::MemoryService::load(&*fixture.store, fixture.workspace.id().as_str())
            .unwrap();
    assert!(memory.all().is_empty());
    assert!(fixture.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_revalidates_provenance_after_a_file_mutation_in_the_same_batch() {
    use sha2::{Digest, Sha256};
    let source = format!(
        "file:source.ts@sha256:{:x}",
        Sha256::digest(b"owned source\n")
    );
    let (origin, model) = server(vec![
        calls(vec![(
            "memory-source",
            "memory",
            json!({"action":"add","kind":"fact","text":"Source component behavior","component":"Native","sources":[source]}),
        )]),
        calls(vec![
            ("current-memory", "memory", json!({"action":"list"})),
            (
                "change-source",
                "write",
                json!({"path":"source.ts","content":"changed source\n"}),
            ),
            (
                "stale-memory",
                "memory",
                json!({"action":"search","query":"component behavior"}),
            ),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("source.ts"), "owned source\n").unwrap();
    let runner = native_memory_runner(&fixture).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(model.join().unwrap().len(), 3);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 4);
    assert!(
        results[1]["content"]
            .to_string()
            .contains("source_status: current")
    );
    assert!(
        results[3]["content"]
            .to_string()
            .contains("source_status: stale")
    );
    let (memory, _) =
        ag1_core::memory::MemoryService::load(&*fixture.store, fixture.workspace.id().as_str())
            .unwrap();
    assert_eq!(memory.all()[0]["sources"][0], source);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_receipt_replay_preserves_later_edits_without_reexecuting_the_old_action() {
    use ag1_core::memory::MemoryService;
    use ag1_platform::session::{ToolRunner, ToolSelection};
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let runner = native_memory_runner(&fixture).await;
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let revision = std::sync::atomic::AtomicU64::new(scope.revision);
    let cancel = AtomicBool::new(false);
    let selection = ToolSelection {
        journal: None,
        names: vec!["memory".into()],
        research_profile: ag1_core::tools::research::profile(90000., 0.),
        loaded_memory: Default::default(),
    };
    let call = ag1_core::runtime::session::PendingCall {
        id: "once".into(),
        name: "memory".into(),
        block: json!({"type":"toolCall","id":"once","name":"memory","arguments":{"action":"add","kind":"fact","text":"Original memory"}}),
    };
    let first = runner
        .execute(
            &scope,
            std::slice::from_ref(&call),
            &selection,
            "owned-memory-receipt",
            &cancel,
            &revision,
        )
        .unwrap();
    assert!(!first[0].outcome.replayed);
    let (_, current) = MemoryService::load(&*fixture.store, &scope.workspace).unwrap();
    MemoryService::mutate(
        &*fixture.store,
        &scope.workspace,
        current.as_deref(),
        |memory| {
            Ok(memory
                .update("m1", &json!({"text":"Later correction"}), 2000)?
                .store)
        },
    )
    .unwrap();
    let replay = runner
        .execute(
            &scope,
            std::slice::from_ref(&call),
            &selection,
            "owned-memory-receipt",
            &cancel,
            &revision,
        )
        .unwrap();
    assert!(replay[0].outcome.replayed);
    assert!(replay[0].execution_settled);
    let (memory, _) = MemoryService::load(&*fixture.store, &scope.workspace).unwrap();
    assert_eq!(memory.all().len(), 1);
    assert_eq!(memory.all()[0]["text"], "Later correction");
    assert_eq!(
        first[0].outcome.record.result,
        replay[0].outcome.record.result
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_rejects_stale_preparation_instead_of_overwriting_a_new_revision() {
    use ag1_core::{
        memory::MemoryService,
        tools::approval::{Authorizer, Preflight, PreflightContext},
    };
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let runner = native_memory_runner(&fixture).await;
    let service = runner.adapters.memory.as_ref().unwrap();
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let call = runner
        .registry
        .prepare(
            "old-memory",
            "old-memory",
            "memory",
            &json!({"action":"clear"}),
            true,
        )
        .unwrap();
    let plan = service.prepare(&call, &scope).unwrap();
    let Preflight::Ready(ticket) = Authorizer::new("memory-race")
        .unwrap()
        .preflight(
            call,
            PreflightContext {
                scope: &scope,
                policy: &runner.policy,
                policy_revision: 1,
                target_revision: plan.target_revision(),
                now: 1000,
                ttl: 5000,
                untrusted_extensions: true,
            },
        )
        .unwrap()
    else {
        panic!("expected native memory authorization")
    };
    MemoryService::mutate(&*fixture.store, &scope.workspace, None, |memory| {
        Ok(memory
            .add(
                "decision",
                "New revision must survive",
                1000,
                "core",
                &json!({}),
            )?
            .store)
    })
    .unwrap();
    let cancel = AtomicBool::new(false);
    let rejected = service
        .execute(
            plan,
            ticket,
            ag1_platform::file_tools::FileExecution {
                registry: &runner.registry,
                scope: &scope,
                policy_revision: 1,
                now: || 1000,
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(5),
            },
            &Default::default(),
        )
        .unwrap();
    assert_eq!(rejected.effect, ag1_core::tools::executor::Effect::None);
    assert_eq!(rejected.result["isError"], true);
    assert!(rejected.result["content"].to_string().contains("Conflict"));
    let (memory, _) = MemoryService::load(&*fixture.store, &scope.workspace).unwrap();
    assert_eq!(memory.all()[0]["text"], "New revision must survive");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_clear_cannot_replace_corrupt_imported_bytes_with_an_empty_store() {
    use ag1_platform::session::{ToolRunner, ToolSelection};
    let raw = b"{broken memory preserved";
    let fixture =
        fixture_with_imported_memory("http://127.0.0.1:1", "disabled", vec![], None, Some(raw));
    let runner = native_memory_runner(&fixture).await;
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let revision = std::sync::atomic::AtomicU64::new(scope.revision);
    let cancel = AtomicBool::new(false);
    let call = ag1_core::runtime::session::PendingCall {
        id: "clear-corrupt".into(),
        name: "memory".into(),
        block: json!({"type":"toolCall","id":"clear-corrupt","name":"memory","arguments":{"action":"clear"}}),
    };
    let outputs = runner
        .execute(
            &scope,
            &[call],
            &ToolSelection {
                journal: None,
                names: vec!["memory".into()],
                research_profile: ag1_core::tools::research::profile(90000., 0.),
                loaded_memory: Default::default(),
            },
            "corrupt-memory-clear",
            &cancel,
            &revision,
        )
        .unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(
        outputs[0].outcome.record.effect,
        ag1_core::tools::executor::Effect::None
    );
    assert_eq!(outputs[0].outcome.result["isError"], true);
    assert_eq!(
        outputs[0].outcome.result["details"]["rejectedBeforeExecution"],
        true
    );
    assert!(outputs[0].execution_settled);
    let memory = fixture
        .store
        .load_memory(&scope.workspace)
        .unwrap()
        .unwrap();
    assert_eq!(memory.bytes, raw);
    assert!(memory.diagnostic.is_some());
    assert!(
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await
            .is_err()
    );
    assert_eq!(
        fixture
            .store
            .load_memory(&scope.workspace)
            .unwrap()
            .unwrap()
            .bytes,
        raw
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_semantic_errors_are_settled_without_stopping_following_tool_calls() {
    let (origin, model) = server(vec![
        calls(vec![
            (
                "invalid-memory-update",
                "memory",
                json!({"action":"update","id":"missing","text":"No entry exists"}),
            ),
            (
                "valid-memory-add",
                "memory",
                json!({"action":"add","kind":"fact","text":"Valid follow-up memory"}),
            ),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let runner = native_memory_runner(&fixture).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(model.join().unwrap().len(), 2);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["isError"], true);
    assert_eq!(results[0]["details"]["executionEffect"], "none");
    assert!(results[1]["isError"] != true);
    let (memory, _) =
        ag1_core::memory::MemoryService::load(&*fixture.store, fixture.workspace.id().as_str())
            .unwrap();
    assert_eq!(memory.all().len(), 1);
    assert_eq!(memory.all()[0]["text"], "Valid follow-up memory");
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_memory_storage_failures_remain_unsettled_until_the_actual_revision_is_inspected() {
    use ag1_core::tools::approval::{Authorizer, Preflight, PreflightContext};
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let fixture = fixture_seeded_with_fault(
            "http://127.0.0.1:1",
            "disabled",
            vec![],
            Some(fault.clone()),
        );
        let runner = native_memory_runner(&fixture).await;
        let service = runner.adapters.memory.as_ref().unwrap();
        let scope = ag1_core::tools::registry::Scope {
            workspace: fixture.workspace.id().as_str().into(),
            session: "session".into(),
            epoch: 1,
            revision: fixture.host.journal_revision().unwrap(),
        };
        let call = runner.registry.prepare("faulted-memory", "faulted-memory", "memory", &json!({"action":"add","kind":"fact","text":"Retain committed memory despite lost acknowledgment"}), true).unwrap();
        let plan = service.prepare(&call, &scope).unwrap();
        let Preflight::Ready(ticket) = Authorizer::new("memory-fault")
            .unwrap()
            .preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &runner.policy,
                    policy_revision: 1,
                    target_revision: plan.target_revision(),
                    now: 1000,
                    ttl: 5000,
                    untrusted_extensions: true,
                },
            )
            .unwrap()
        else {
            panic!("expected native memory authorization")
        };
        fault.armed.store(true, Ordering::SeqCst);
        let cancel = AtomicBool::new(false);
        // A commit/ack failure is propagated, so NativeToolRunner retains an
        // unknown effect and stops automatic continuation. It is not converted
        // into the settled, no-effect semantic errors above.
        assert!(
            service
                .execute(
                    plan,
                    ticket,
                    ag1_platform::file_tools::FileExecution {
                        registry: &runner.registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 1000,
                        cancel: &cancel,
                        deadline: Instant::now() + Duration::from_secs(5),
                    },
                    &Default::default()
                )
                .is_err()
        );
        let document = fixture.store.load_memory(&scope.workspace).unwrap();
        assert_eq!(
            document.is_some(),
            point == ag1_storage::Boundary::BeforeAck
        );
        if let Some(document) = document {
            let memory: Value = serde_json::from_slice(&document.bytes).unwrap();
            assert_eq!(memory["entries"].as_array().unwrap().len(), 1);
            assert_eq!(
                memory["entries"][0]["text"],
                "Retain committed memory despite lost acknowledgment"
            );
        }
    }
}

async fn native_artifact_runner(
    fixture: &Fixture,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{artifact_tools::ArtifactTools, session::adapters::NativeAdapters};
    let adapters = NativeAdapters {
        artifact: Some(Arc::new(ArtifactTools::new(
            fixture.workspace.clone(),
            fixture.store.clone(),
        ))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["artifact".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "artifact")
        .collect();
    fixture
        .host
        .select_tools(
            "native-artifact-tools",
            "native-artifact-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
fn retained_artifact(fixture: &Fixture, id: u32, text: &str, kind: &str) -> String {
    let id = format!("art_{id:032x}");
    fixture.store.put_artifact(&ag1_storage::ArtifactInput {session:"session".into(),origin:"session".into(),bytes:text.as_bytes().to_vec(),record:serde_json::to_vec(&json!({"version":1,"id":id,"sessionId":"session","kind":kind,"mimeType":"text/plain","encoding":"utf8","createdAt":NOW,"sha256":ag1_storage::digest(text.as_bytes()),"bytes":text.len(),"producer":{"type":"system","name":"owned-artifact"}})).unwrap()}).unwrap()
}
fn artifact_envelope(result: &Value) -> Value {
    serde_json::from_str(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap()
}
fn execute_artifact(
    fixture: &Fixture,
    runner: &ag1_platform::session::files::NativeToolRunner,
    id: &str,
    args: Value,
) -> ag1_platform::session::ToolOutcome {
    use ag1_platform::session::{ToolRunner, ToolSelection};
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let pending = ag1_core::runtime::session::PendingCall {
        id: id.into(),
        name: "artifact".into(),
        block: json!({"type":"toolCall","id":id,"name":"artifact","arguments":args}),
    };
    runner
        .execute(
            &scope,
            &[pending],
            &ToolSelection {
                journal: None,
                names: vec!["artifact".into()],
                research_profile: ag1_core::tools::research::profile(90000., 0.),
                loaded_memory: Default::default(),
            },
            id,
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(scope.revision),
        )
        .unwrap()
        .remove(0)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_artifact_actions_recover_evidence_without_catalog_self_matches() {
    let id = format!("art_{:032x}", 1);
    let (origin, model) = server(vec![
        calls(vec![
            ("art-list", "artifact", json!({"action":"list"})),
            (
                "art-search",
                "artifact",
                json!({"action":"search","query":"decisive evidence"}),
            ),
            (
                "art-self-search",
                "artifact",
                json!({"action":"search","query":"query-only-custody"}),
            ),
            ("art-meta", "artifact", json!({"action":"meta","id":id})),
            (
                "art-grep",
                "artifact",
                json!({"action":"grep","id":id,"query":"decisive evidence"}),
            ),
            ("art-read", "artifact", json!({"action":"read","id":id})),
            (
                "art-lease",
                "artifact",
                json!({"action":"lease","id":id,"reason":"Keep the decisive evidence until verification"}),
            ),
            ("art-leases", "artifact", json!({"action":"leases"})),
            (
                "art-release",
                "artifact",
                json!({"action":"release","id":id}),
            ),
            ("art-no-leases", "artifact", json!({"action":"leases"})),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let text = "before\ndecisive evidence β 🦙\nafter\n";
    assert_eq!(retained_artifact(&fixture, 1, text, "tool-output"), id);
    let runner = native_artifact_runner(&fixture).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let requests = model.join().unwrap();
    assert_eq!(requests.len(), 2);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 10);
    assert!(results.iter().all(|r| r["isError"] != true), "{results:?}");
    assert_eq!(results[0]["details"]["count"], 1);
    assert_eq!(
        artifact_envelope(&results[1])["artifactSearch"]["matches"][0]["id"],
        id
    );
    assert_eq!(
        artifact_envelope(&results[2])["artifactSearch"]["matches"],
        json!([])
    );
    assert!(
        results[3]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains(&ag1_storage::digest(text.as_bytes()))
    );
    let context = artifact_envelope(&results[4])["artifactGrep"]["matches"][0].clone();
    assert_eq!(context["matchedText"], "decisive evidence β 🦙");
    let cursor = context["readCall"]["cursor"].as_str().unwrap();
    let decoded = fixture.store.decode_cursor("session", cursor).unwrap();
    assert!(decoded.context);
    assert_eq!(decoded.anchor_line, Some(2));
    assert!(
        results[5]["content"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with(text)
    );
    assert_eq!(
        artifact_envelope(&results[6])["artifactLease"]["status"],
        "active"
    );
    assert_eq!(
        artifact_envelope(&results[7])["artifactLeases"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        artifact_envelope(&results[8])["artifactLeaseRelease"]["count"],
        1
    );
    assert_eq!(artifact_envelope(&results[9])["artifactLeases"], json!([]));
    // Real durable tool results reach the next provider request.
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("decisive evidence")
    );
    assert!(fixture.host.settled().unwrap());
    let Fixture {
        _directory: directory,
        host,
        files,
        store,
        ..
    } = fixture;
    drop(host);
    drop(files);
    drop(store);
    let reopened = Store::open(&directory.path().join("store")).unwrap();
    assert_eq!(reopened.decode_cursor("session", cursor).unwrap(), decoded);
    assert_eq!(
        reopened
            .page(
                "session",
                &decoded.origin,
                &decoded.artifact,
                decoded.offset,
                decoded.max_bytes
            )
            .unwrap()
            .bytes,
        text.as_bytes()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_artifact_cursor_pages_are_exact_and_incomplete_context_stays_navigable() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let text = format!("{}needle\nlast\n", "x".repeat(40000));
    let id = retained_artifact(&fixture, 1, &text, "text");
    let runner = native_artifact_runner(&fixture).await;
    let search = execute_artifact(
        &fixture,
        &runner,
        "long-grep",
        json!({"action":"grep","id":id,"query":"needle"}),
    );
    assert!(search.execution_settled);
    let context = artifact_envelope(&search.outcome.result)["artifactGrep"]["matches"][0].clone();
    assert_eq!(context["readCallPurpose"], "boundedContextPage");
    assert_eq!(context["contextComplete"], false);
    assert_eq!(context["matchedTextTruncated"], true);
    let page = execute_artifact(&fixture, &runner, "long-page", context["readCall"].clone());
    let page_result = &page.outcome.result;
    let envelope = artifact_envelope(page_result)["artifactPage"].clone();
    assert!(envelope.get("complete").is_none());
    let first = page_result["content"][0]["text"]
        .as_str()
        .unwrap()
        .split_once('\n')
        .unwrap()
        .1;
    assert_eq!(first.len(), 32768);
    assert_eq!(first, &text[..32768]);
    // Large artifact reads survive B01 intact instead of externalizing again.
    assert!(page_result["details"].get("artifact").is_none());
    let next = execute_artifact(&fixture, &runner, "long-next", envelope["nextCall"].clone());
    let next_text = next.outcome.result["content"][0]["text"]
        .as_str()
        .unwrap()
        .split_once('\n')
        .unwrap()
        .1;
    assert_eq!(format!("{first}{next_text}"), text);
    assert!(next_text.contains("needle"));
    let short_id = retained_artifact(&fixture, 2, "before\nneedle\nafter\n", "text");
    let context = fixture
        .store
        .context_cursor("session", "session", &short_id, 0..20, 2)
        .unwrap();
    let small = execute_artifact(
        &fixture,
        &runner,
        "small-context",
        json!({"action":"read","cursor":context,"maxBytes":3}),
    );
    let small_envelope = artifact_envelope(&small.outcome.result)["artifactPage"].clone();
    assert!(small_envelope.get("complete").is_none());
    assert!(small_envelope.get("nextCall").is_some());
    let exact = execute_artifact(
        &fixture,
        &runner,
        "exact-context",
        json!({"action":"read","id":context}),
    );
    assert_eq!(
        artifact_envelope(&exact.outcome.result)["artifactPage"]["complete"],
        true
    );
    for (index, args) in [
        json!({"action":"read","cursor":context,"offset":0}),
        json!({"action":"read","cursor":context,"id":id}),
        json!({"action":"read","cursor":"ac2.tampered.invalid"}),
    ]
    .into_iter()
    .enumerate()
    {
        let invalid = execute_artifact(&fixture, &runner, &format!("invalid-cursor-{index}"), args);
        assert_eq!(invalid.outcome.result["isError"], true);
        assert_eq!(
            invalid.outcome.record.effect,
            ag1_core::tools::executor::Effect::None
        );
        assert!(invalid.execution_settled);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_artifact_lease_receipts_do_not_recreate_released_evidence() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let id = retained_artifact(&fixture, 1, "lease evidence", "text");
    let runner = native_artifact_runner(&fixture).await;
    let args = json!({"action":"lease","id":id,"reason":"Hold through verification"});
    let first = execute_artifact(&fixture, &runner, "lease-once", args.clone());
    assert_eq!(
        first.outcome.record.effect,
        ag1_core::tools::executor::Effect::Committed
    );
    let lease = artifact_envelope(&first.outcome.result)["artifactLease"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let release = execute_artifact(
        &fixture,
        &runner,
        "release-once",
        json!({"action":"release","id":lease}),
    );
    assert_eq!(
        artifact_envelope(&release.outcome.result)["artifactLeaseRelease"]["count"],
        1
    );
    let replay = execute_artifact(&fixture, &runner, "lease-once", args);
    assert!(replay.outcome.replayed);
    assert!(replay.execution_settled);
    assert!(
        fixture
            .store
            .evidence_lease_records("session")
            .unwrap()
            .is_empty()
    );
    assert_eq!(replay.outcome.result, first.outcome.result);
    let invalid = execute_artifact(
        &fixture,
        &runner,
        "empty-reason",
        json!({"action":"lease","id":id,"reason":"   "}),
    );
    assert_eq!(invalid.outcome.result["isError"], true);
    assert_eq!(
        invalid.outcome.record.effect,
        ag1_core::tools::executor::Effect::None
    );
    assert!(invalid.execution_settled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_artifact_lease_commit_and_ack_faults_require_inspection() {
    use ag1_core::tools::approval::{Authorizer, Preflight, PreflightContext};
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        for action in ["lease", "release"] {
            let fault = Arc::new(ArmedStorageFault {
                point,
                armed: AtomicBool::new(false),
            });
            let fixture = fixture_seeded_with_fault(
                "http://127.0.0.1:1",
                "disabled",
                vec![],
                Some(fault.clone()),
            );
            let id = retained_artifact(&fixture, 1, "decisive evidence", "text");
            let runner = native_artifact_runner(&fixture).await;
            if action == "release" {
                fixture
                    .store
                    .lease("session", "session", &id, "existing lease", 120000, true)
                    .unwrap();
            }
            let service = runner.adapters.artifact.as_ref().unwrap();
            let scope = ag1_core::tools::registry::Scope {
                workspace: fixture.workspace.id().as_str().into(),
                session: "session".into(),
                epoch: 1,
                revision: fixture.host.journal_revision().unwrap(),
            };
            let call = runner
                .registry
                .prepare(
                    "faulted-artifact",
                    "faulted-artifact",
                    "artifact",
                    &json!({"action":action,"id":id,"reason":"Retain acknowledged evidence"}),
                    true,
                )
                .unwrap();
            let plan = service.prepare(&call, &scope).unwrap();
            let Preflight::Ready(ticket) = Authorizer::new("artifact-fault")
                .unwrap()
                .preflight(
                    call,
                    PreflightContext {
                        scope: &scope,
                        policy: &runner.policy,
                        policy_revision: 1,
                        target_revision: plan.target_revision(),
                        now: 1000,
                        ttl: 5000,
                        untrusted_extensions: true,
                    },
                )
                .unwrap()
            else {
                panic!("expected artifact authorization")
            };
            fault.armed.store(true, Ordering::SeqCst);
            assert!(
                service
                    .execute(
                        plan,
                        ticket,
                        ag1_platform::file_tools::FileExecution {
                            registry: &runner.registry,
                            scope: &scope,
                            policy_revision: 1,
                            now: || 1000,
                            cancel: &AtomicBool::new(false),
                            deadline: Instant::now() + Duration::from_secs(5)
                        }
                    )
                    .is_err()
            );
            let applied = point == ag1_storage::Boundary::BeforeAck;
            assert_eq!(
                !fixture
                    .store
                    .evidence_lease_records("session")
                    .unwrap()
                    .is_empty(),
                if action == "lease" { applied } else { !applied }
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_artifact_prepared_authority_cannot_move_between_sessions_or_calls() {
    use ag1_core::tools::approval::{Authorizer, Preflight, PreflightContext};
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let id = retained_artifact(&fixture, 1, "private evidence", "text");
    let runner = native_artifact_runner(&fixture).await;
    let service = runner.adapters.artifact.as_ref().unwrap();
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let other = ag1_core::tools::registry::Scope {
        session: "other".into(),
        ..scope.clone()
    };
    for swap_scope in [true, false] {
        let call = runner
            .registry
            .prepare(
                "artifact-scope",
                "artifact-scope",
                "artifact",
                &json!({"action":"lease","id":id,"reason":"Authorized evidence"}),
                true,
            )
            .unwrap();
        let plan = service.prepare(&call, &scope).unwrap();
        let authorization_call = if swap_scope {
            call
        } else {
            runner
                .registry
                .prepare(
                    "different-call",
                    "different-call",
                    "artifact",
                    &json!({"action":"release","id":id}),
                    true,
                )
                .unwrap()
        };
        let Preflight::Ready(ticket) = Authorizer::new("artifact-scope")
            .unwrap()
            .preflight(
                authorization_call,
                PreflightContext {
                    scope: &scope,
                    policy: &runner.policy,
                    policy_revision: 1,
                    target_revision: plan.target_revision(),
                    now: 1000,
                    ttl: 5000,
                    untrusted_extensions: true,
                },
            )
            .unwrap()
        else {
            panic!("expected artifact authorization")
        };
        let result = service
            .execute(
                plan,
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &runner.registry,
                    scope: if swap_scope { &other } else { &scope },
                    policy_revision: 1,
                    now: || 1000,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(5),
                },
            )
            .unwrap();
        assert_eq!(result.result["isError"], true);
        assert_eq!(result.effect, ag1_core::tools::executor::Effect::None);
        assert!(
            fixture
                .store
                .evidence_lease_records("session")
                .unwrap()
                .is_empty()
        );
    }
}

fn normalized_artifact_value(value: Value, store: &Store) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|v| normalized_artifact_value(v, store))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, v)| (key, normalized_artifact_value(v, store)))
                .collect(),
        ),
        Value::String(text) if text.starts_with("ac1.") || text.starts_with("ac2.") => {
            let cursor = store.decode_cursor("session", &text).unwrap();
            json!(format!(
                "cursor:{}:{}:{}:{}:{}",
                cursor.artifact,
                cursor.offset,
                cursor.max_bytes,
                if cursor.context { "context" } else { "page" },
                cursor.anchor_line.unwrap_or(0)
            ))
        }
        Value::String(text) if text.starts_with("lease_") => json!("lease_normalized"),
        other => other,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_artifact_tool_presentation_matches_thirteen_unchanged_sdk_actions() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/artifact-tool-oracle.json"
    ))
    .unwrap();
    let fault = Arc::new(ArmedStorageFault {
        point: ag1_storage::Boundary::BeforeCommit,
        armed: AtomicBool::new(false),
    });
    let fixture = fixture_seeded_with_fault("http://127.0.0.1:1", "disabled", vec![], Some(fault));
    fixture
        .store
        .put_artifact(&ag1_storage::ArtifactInput {
            session: "session".into(),
            origin: "session".into(),
            record: serde_json::to_vec(&oracle["record"]).unwrap(),
            bytes: oracle["text"].as_str().unwrap().as_bytes().to_vec(),
        })
        .unwrap();
    let runner = native_artifact_runner(&fixture).await;
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 13);
    for row in rows {
        let outcome = execute_artifact(
            &fixture,
            &runner,
            row["id"].as_str().unwrap(),
            row["args"].clone(),
        );
        assert!(outcome.execution_settled);
        let mut result = outcome.outcome.result;
        for part in result["content"].as_array_mut().unwrap() {
            if part["type"] != "text" {
                continue;
            }
            let text = part["text"].as_str().unwrap().to_owned();
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                part["text"] = value;
            } else if let Some((envelope, body)) = text.split_once('\n')
                && let Ok(envelope) = serde_json::from_str::<Value>(envelope)
            {
                part["text"] = json!({"envelope":envelope,"body":body});
            }
        }
        if row["args"]["action"] == "search" {
            for key in ["scanned", "candidates", "bytesScanned"] {
                result["details"].as_object_mut().unwrap().remove(key);
            }
        }
        assert_eq!(
            normalized_artifact_value(result, &fixture.store),
            row["expected"],
            "{}",
            row["id"]
        );
    }
}

fn owned_repo_git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("/usr/bin/git")
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-09-06T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-09-06T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
async fn native_repo_runner(
    fixture: &Fixture,
    git: &std::path::Path,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{git::GitService, repo_tools::RepoTools, session::adapters::NativeAdapters};
    let spools = fixture._directory.path().join("repo-spools");
    std::fs::create_dir(&spools).unwrap();
    let adapters = NativeAdapters {
        repo: Some(Arc::new(RepoTools::new(
            fixture.workspace.clone(),
            GitService::new(
                env!("CARGO_BIN_EXE_ag1-process-host").as_ref(),
                git,
                &spools,
            )
            .unwrap(),
            fixture.store.clone(),
        ))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["repo".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "repo")
        .collect();
    fixture
        .host
        .select_tools(
            "native-repo-tools",
            "native-repo-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
fn execute_repo(
    fixture: &Fixture,
    runner: &ag1_platform::session::files::NativeToolRunner,
    id: &str,
    view: &str,
) -> Value {
    use ag1_platform::session::{ToolRunner, ToolSelection};
    let scope = ag1_core::tools::registry::Scope {
        workspace: fixture.workspace.id().as_str().into(),
        session: "session".into(),
        epoch: 1,
        revision: fixture.host.journal_revision().unwrap(),
    };
    let pending = ag1_core::runtime::session::PendingCall {
        id: id.into(),
        name: "repo".into(),
        block: json!({"type":"toolCall","id":id,"name":"repo","arguments":{"view":view}}),
    };
    let mut outputs = runner
        .execute(
            &scope,
            &[pending],
            &ToolSelection {
                journal: None,
                names: vec!["repo".into()],
                research_profile: ag1_core::tools::research::profile(90000., 0.),
                loaded_memory: Default::default(),
            },
            id,
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(scope.revision),
        )
        .unwrap();
    assert!(outputs[0].execution_settled);
    outputs.remove(0).outcome.result
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_repo_views_match_six_sdk_cases_and_feed_the_next_request() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/repo-oracle.json"
    ))
    .unwrap();
    for state in ["nonrepo", "dirty"] {
        let (origin, model) = server(vec![
            calls(vec![
                ("repo-status", "repo", json!({"view":"status"})),
                ("repo-map", "repo", json!({"view":"map"})),
                ("repo-all", "repo", json!({"view":"all"})),
            ]),
            stop(),
        ]);
        let fixture = fixture(&origin, "disabled");
        for (name, text) in oracle["files"].as_object().unwrap() {
            let target = fixture.root.join(name);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::write(target, text.as_str().unwrap()).unwrap();
        }
        std::fs::create_dir(fixture.root.join(".ag1")).unwrap();
        std::fs::write(
            fixture.root.join(".ag1/memory.json"),
            oracle["memory"].to_string(),
        )
        .unwrap();
        ag1_core::memory::MemoryService::mutate(
            &*fixture.store,
            fixture.workspace.id().as_str(),
            None,
            |_| ag1_core::memory::MemoryStore::parse(&oracle["memory"]),
        )
        .unwrap();
        if state == "dirty" {
            owned_repo_git(&fixture.root, &["init", "-b", "main"]);
            owned_repo_git(&fixture.root, &["config", "core.abbrev", "7"]);
            owned_repo_git(
                &fixture.root,
                &[
                    "add",
                    "package.json",
                    "Cargo.toml",
                    "src/main.ts",
                    "README.md",
                ],
            );
            owned_repo_git(&fixture.root, &["commit", "-m", "Owned baseline"]);
            std::fs::write(
                fixture.root.join("src/main.ts"),
                "export const answer = 43;\n",
            )
            .unwrap();
            std::fs::write(fixture.root.join("new.txt"), "untracked\n").unwrap();
        }
        let runner = native_repo_runner(&fixture, "/usr/bin/git".as_ref()).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await
            .unwrap();
        let requests = model.join().unwrap();
        assert_eq!(requests.len(), 2);
        let results = session_tool_results(&fixture);
        assert_eq!(results.len(), 3);
        for (result, row) in results.iter().zip(
            oracle["rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["state"] == state),
        ) {
            assert!(result["isError"] != true, "{result}");
            assert_eq!(
                result["content"], row["expected"]["content"],
                "{state} {}",
                row["view"]
            );
            assert_eq!(result["details"], row["expected"]["details"]);
        }
        assert!(
            requests[1]["messages"]
                .to_string()
                .contains("Preserve tuned runtimes")
        );
        assert!(fixture.host.settled().unwrap());
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_repo_preserves_full_branch_names_and_handles_unborn_and_detached_heads() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    owned_repo_git(&fixture.root, &["init", "-b", "feature.native"]);
    let runner = native_repo_runner(&fixture, "/usr/bin/git".as_ref()).await;
    let unborn = execute_repo(&fixture, &runner, "repo-unborn", "status");
    assert_eq!(unborn["details"]["branch"], "feature.native");
    assert!(
        unborn["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Git: feature.native\n")
    );
    std::fs::write(fixture.root.join("file.txt"), "baseline\n").unwrap();
    owned_repo_git(&fixture.root, &["add", "file.txt"]);
    owned_repo_git(&fixture.root, &["commit", "-m", "Baseline"]);
    let branch = execute_repo(&fixture, &runner, "repo-branch", "status");
    assert_eq!(branch["details"]["branch"], "feature.native");
    owned_repo_git(&fixture.root, &["checkout", "--detach"]);
    let detached = execute_repo(&fixture, &runner, "repo-detached", "status");
    assert!(detached["details"].get("branch").is_none());
    assert!(
        detached["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Git: DETACHED\n")
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_repo_does_not_read_outside_manifest_symlinks() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    let secret = fixture._directory.path().join("external.json");
    std::fs::write(&secret, "{\"name\":\"outside-private-marker\"}").unwrap();
    std::os::unix::fs::symlink(secret, fixture.root.join("package.json")).unwrap();
    let runner = native_repo_runner(&fixture, "/usr/bin/git".as_ref()).await;
    let result = execute_repo(&fixture, &runner, "repo-contained", "map");
    assert!(result["isError"] != true);
    assert!(!result.to_string().contains("outside-private-marker"));
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("stack: node")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_repo_revalidates_source_memory_after_working_tree_changes() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    std::fs::write(
        fixture.root.join("source.ts"),
        "export const behavior = 1;\n",
    )
    .unwrap();
    let reference = format!(
        "file:source.ts@sha256:{}",
        ag1_storage::digest(b"export const behavior = 1;\n")
    );
    ag1_core::memory::MemoryService::mutate(
        &*fixture.store,
        fixture.workspace.id().as_str(),
        None,
        |memory| {
            Ok(memory
                .add(
                    "fact",
                    "The current behavior",
                    1000,
                    "normal",
                    &json!({"component":"Source","sources":[reference]}),
                )?
                .store)
        },
    )
    .unwrap();
    let runner = native_repo_runner(&fixture, "/usr/bin/git".as_ref()).await;
    let current = execute_repo(&fixture, &runner, "repo-current", "map");
    assert!(
        current["content"]
            .to_string()
            .contains("source_status: current")
    );
    std::fs::write(
        fixture.root.join("source.ts"),
        "export const behavior = 2;\n",
    )
    .unwrap();
    let stale = execute_repo(&fixture, &runner, "repo-stale", "all");
    assert!(
        stale["content"]
            .to_string()
            .contains("source_status: stale")
    );
    assert!(
        !stale["content"]
            .to_string()
            .contains("source_status: current")
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_repo_cancellation_drains_owned_git_before_session_settlement() {
    use std::os::unix::fs::PermissionsExt;
    let (origin, model) = server(vec![calls(vec![(
        "repo-cancel",
        "repo",
        json!({"view":"all"}),
    )])]);
    let fixture = fixture(&origin, "disabled");
    let executable = fixture._directory.path().join("owned-slow-git");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s' \"$$\" > git-started\nsleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runner = native_repo_runner(&fixture, &executable).await;
    let host = fixture.host.clone();
    let running = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), runner, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fixture.root.join("git-started").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let pid = std::fs::read_to_string(fixture.root.join("git-started")).unwrap();
    fixture.host.cancel().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err() || fixture.host.settled().unwrap());
    assert!(fixture.host.settled().unwrap());
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert_eq!(model.join().unwrap().len(), 1);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["isError"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_repo_preserves_package_bin_order_without_global_json_changes() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    std::fs::write(
        fixture.root.join("package.json"),
        r#"{"main":"main.js","bin":{"z":"last.js","10":"ten.js","2":"two.js","a":"alpha.js"}}"#,
    )
    .unwrap();
    let runner = native_repo_runner(&fixture, "/usr/bin/git".as_ref()).await;
    let result = execute_repo(&fixture, &runner, "repo-bin-order", "map");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("entry: main.js, two.js, ten.js, last.js, alpha.js")
    );
}

async fn native_task_runner(
    fixture: &Fixture,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{session::adapters::NativeAdapters, task_tools::TaskTools};
    let adapters = NativeAdapters {
        task: Some(Arc::new(TaskTools::new(fixture.workspace.clone()))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["taskState".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "taskState")
        .collect();
    fixture
        .host
        .select_tools(
            "native-task-tools",
            "native-task-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_task_actions_commit_in_source_order_with_one_blocking_worker() {
    let (origin, model) = server(vec![
        calls(vec![
            ("task-show", "taskState", json!({"action":"show"})),
            (
                "task-set",
                "taskState",
                json!({"action":"set","currentPhase":"native integration","nextAction":"Keep implementing"}),
            ),
            (
                "task-record",
                "taskState",
                json!({"action":"record","kind":"DECISION","statement":"Preserve tuned runtimes","reason":"Required by the user"}),
            ),
            ("task-observe", "taskState", json!({"action":"show"})),
            (
                "task-update",
                "taskState",
                json!({"action":"update","id":"ts_00000001","statement":"Revised seeded hypothesis","entryStatus":"resolved"}),
            ),
            (
                "task-remove",
                "taskState",
                json!({"action":"remove","id":"ts_00000001"}),
            ),
            (
                "task-reset",
                "taskState",
                json!({"action":"reset","objective":"Next authorized objective","currentSubgoal":"Verify continuity"}),
            ),
            ("task-final-show", "taskState", json!({"action":"show"})),
        ]),
        stop(),
    ]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    let (state, _) = ag1_core::task::TaskState::reset("Initial objective", NOW)
        .unwrap()
        .record(
            &json!({"kind":"HYPOTHESIS","statement":"Seeded hypothesis"}),
            "ts_00000001",
            NOW,
        )
        .unwrap();
    fixture
        .host
        .append_task_state("task-seed", "task-seed", NOW, state, &ignore)
        .await
        .unwrap();
    let runner = native_task_runner(&fixture).await;
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    let requests = model.join().unwrap();
    assert_eq!(requests.len(), 2);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 8);
    assert!(results.iter().all(|r| r["isError"] != true), "{results:?}");
    assert_eq!(
        results[3]["details"]["snapshot"]["currentPhase"],
        "native integration"
    );
    assert_eq!(
        results[3]["details"]["snapshot"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        results[4]["details"]["snapshot"]["entries"][0]["status"],
        "resolved"
    );
    assert_eq!(
        results[5]["details"]["snapshot"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        results[7]["details"]["snapshot"]["objective"],
        "Next authorized objective"
    );
    assert_eq!(results[7]["details"]["snapshot"]["entries"], json!([]));
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("Next authorized objective")
    );
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let positions = branch
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row["customType"] == "task_state" || row["message"]["role"] == "toolResult"
        })
        .collect::<Vec<_>>();
    let set_result = positions
        .iter()
        .position(|(_, row)| row["message"]["toolCallId"] == "task-set")
        .unwrap();
    assert_eq!(positions[set_result - 1].1["customType"], "task_state");
    assert_eq!(positions[set_result + 1].1["customType"], "task_state");
    assert_eq!(
        positions[set_result + 2].1["message"]["toolCallId"],
        "task-record"
    );
    assert!(fixture.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_task_invalid_provenance_and_checkpoints_do_not_commit_or_stop_later_calls() {
    let (origin, model) = server(vec![
        calls(vec![
            (
                "task-bad-fact",
                "taskState",
                json!({"action":"record","kind":"FACT","statement":"Unsupported claim"}),
            ),
            (
                "task-bad-checkpoint",
                "taskState",
                json!({"action":"set","checkpointId":"ts_00000002"}),
            ),
            (
                "task-good-set",
                "taskState",
                json!({"action":"set","nextAction":"Collect actual verification"}),
            ),
            ("task-good-show", "taskState", json!({"action":"show"})),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let (state,_)=ag1_core::task::TaskState::reset("Actual verification",NOW).unwrap().record(&json!({"kind":"RESULT","statement":"A shell exited successfully","sources":["tool-call:shell-only"]}),"ts_00000002",NOW).unwrap();
    fixture
        .host
        .append_task_state("task-seed", "task-seed", NOW, state, &ignore)
        .await
        .unwrap();
    let runner = native_task_runner(&fixture).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(model.join().unwrap().len(), 2);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 4);
    assert_eq!(results[0]["isError"], true);
    assert_eq!(results[1]["isError"], true);
    assert!(
        results[1]["content"]
            .to_string()
            .contains("successful check")
    );
    assert_eq!(
        results[3]["details"]["snapshot"]["nextAction"],
        "Collect actual verification"
    );
    assert!(
        results[3]["details"]["snapshot"]
            .get("checkpointId")
            .is_none()
    );
    assert_eq!(
        results[3]["details"]["snapshot"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

struct FaultedTaskJournal {
    registry: Arc<Registry>,
    fault: Arc<ArmedStorageFault>,
}
impl ag1_platform::session::ToolRunner for FaultedTaskJournal {
    fn execute(
        &self,
        scope: &ag1_core::tools::registry::Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ag1_platform::session::ToolSelection,
        operation: &str,
        _cancel: &AtomicBool,
        _revision: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
        assert_eq!(calls.len(), 1);
        let call = self.registry.prepare(
            operation,
            &calls[0].id,
            &calls[0].name,
            &calls[0].block["arguments"],
            true,
        )?;
        let journal = selection.journal.as_ref().unwrap();
        let plan = ag1_core::tools::task_state::prepare(
            &journal.snapshot(scope)?,
            call.args(),
            journal.timestamp(),
            "ts_00000001",
        )?;
        let next = plan.next.unwrap().value().clone();
        let mut stale = scope.clone();
        stale.revision += 1;
        assert!(matches!(
            journal.commit_state(&stale, &call, "task_state", next.clone()),
            Err(
                ag1_platform::session::tool_journal::JournalMutationError::BeforeEffect(
                    DomainError::Conflict
                )
            )
        ));
        self.fault.armed.store(true, Ordering::SeqCst);
        // Exercise the exact owner write/ack boundary independently of earlier
        // P09 intent/artifact transactions (those have separate fault tests).
        journal.commit_state(scope, &call, "task_state", next)?;
        panic!("injected journal write must fail");
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_task_journal_commit_failures_block_next_generation_and_preserve_actual_disk_state()
{
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, model) = server(vec![calls(vec![(
            "task-journal-fault",
            "taskState",
            json!({"action":"set","currentPhase":"durable phase marker"}),
        )])]);
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        let native = native_task_runner(&fixture).await;
        let runner = Arc::new(FaultedTaskJournal {
            registry: native.registry.clone(),
            fault,
        });
        assert!(
            fixture
                .host
                .run(input(), &Resolver::new(false), runner, &ignore)
                .await
                .is_err()
        );
        assert_eq!(model.join().unwrap().len(), 1);
        let journal = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        let branch = ag1_core::task::branch_values(&journal).unwrap();
        let latest = branch
            .iter()
            .rev()
            .find(|entry| entry["customType"] == "task_state")
            .unwrap();
        assert_eq!(
            latest["data"]["currentPhase"] == "durable phase marker",
            point == ag1_storage::Boundary::BeforeAck
        );
        let resolver = Resolver::new(false);
        assert!(
            fixture
                .host
                .run(
                    TurnInput {
                        command: "after-failed-journal".into(),
                        ..input()
                    },
                    &resolver,
                    native,
                    &ignore
                )
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    }
}

async fn native_source_runner(
    fixture: &Fixture,
    rg: &std::path::Path,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{
        artifact_tools::ArtifactTools, git::GitService, process::ProcessSupervisor,
        retrieval::SourceHost, session::adapters::NativeAdapters, source::SourceService,
        source_tools::SourceTools,
    };
    let spools = fixture._directory.path().join("source-spools");
    std::fs::create_dir_all(&spools).unwrap();
    let helper = std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host"));
    let adapters = NativeAdapters {
        source: Some(Arc::new(SourceTools::new(
            SourceHost {
                workspace: fixture.workspace.clone(),
                source: SourceService::new(fixture.workspace.clone()),
                recent: vec![],
                tree: Default::default(),
                cancelled: Arc::new(AtomicBool::new(false)),
                supervisor: ProcessSupervisor::new(helper).unwrap(),
                rg: Some(rg.into()),
                spools: spools.clone(),
            },
            GitService::new(helper, "/usr/bin/git".as_ref(), &spools).unwrap(),
            fixture.store.clone(),
            "session".into(),
        ))),
        artifact: Some(Arc::new(ArtifactTools::new(
            fixture.workspace.clone(),
            fixture.store.clone(),
        ))),
        ..Default::default()
    };
    let names: Vec<String> = ["sourceContext", "write", "read", "artifact"]
        .map(String::from)
        .into();
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(&names, &adapters.available_names().into_iter().collect())
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|t| names.iter().any(|n| t["name"] == *n))
        .collect();
    fixture
        .host
        .select_tools(
            "native-source-tools",
            "native-source-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
fn source_files(fixture: &Fixture) -> Value {
    let input: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/source-inputs.json"
    ))
    .unwrap();
    for (path, text) in input["files"].as_object().unwrap() {
        let target = fixture.root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text.as_str().unwrap()).unwrap();
    }
    input
}
fn normalized_source_result(message: &Value) -> Value {
    let output = json!({"content":message["content"],"details":message["details"]});
    let artifact = message["details"]["artifactId"].as_str().unwrap();
    serde_json::from_str(
        &output
            .to_string()
            .replace(artifact, "art_00000000000000000000000000000001"),
    )
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_builds_match_six_unchanged_sdk_capsules_through_model_loop() {
    let retrieval_input: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/source-inputs.json"
    ))
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/source-oracle.json"
    ))
    .unwrap();
    for (scenario, expected) in retrieval_input["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["groups"].as_array().unwrap())
    {
        if scenario["recent"].is_string() || scenario["changed"].is_array() {
            continue;
        }
        let (origin, model) = server(vec![
            calls(vec![(
                "source-build",
                "sourceContext",
                scenario["input"].clone(),
            )]),
            stop(),
        ]);
        let fixture = fixture(&origin, "disabled");
        source_files(&fixture);
        if scenario["long"] == true {
            std::fs::write(
                fixture.root.join("src/long.ts"),
                format!(
                    "export const longSource = \"{}\";\n{}",
                    "x".repeat(5000),
                    (0..100)
                        .map(|i| format!("// line {i}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            )
            .unwrap();
        }
        let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await
            .unwrap();
        assert_eq!(model.join().unwrap().len(), 2);
        let results = session_tool_results(&fixture);
        assert_eq!(results.len(), 1);
        assert_ne!(
            results[0]["isError"], true,
            "{}: {:?}",
            scenario["id"], results
        );
        assert_eq!(
            normalized_source_result(&results[0]),
            expected["result"],
            "{}",
            scenario["id"]
        );
        let branch = ag1_core::task::branch_values(
            &fixture
                .store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(ag1_core::context::capsule::list(&branch, 50).len(), 1);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_catalog_and_status_match_sdk_and_preserve_historical_bytes() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/source-context-tool-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    let capsule = rows[2]["output"]["details"]["capsuleId"].as_str().unwrap();
    let (origin, model) = server(vec![
        calls(vec![
            ("empty", "sourceContext", rows[0]["args"].clone()),
            ("unknown", "sourceContext", rows[1]["args"].clone()),
            ("build", "sourceContext", rows[2]["args"].clone()),
            ("status", "sourceContext", rows[3]["args"].clone()),
            ("list", "sourceContext", rows[4]["args"].clone()),
        ]),
        calls(vec![
            (
                "change",
                "write",
                json!({"path":"src/compiler.ts","content":"export class ChangedCompiler {}\n"}),
            ),
            ("stale", "sourceContext", rows[5]["args"].clone()),
            ("stale-list", "sourceContext", rows[6]["args"].clone()),
        ]),
        calls(vec![(
            "rehydrate",
            "sourceContext",
            json!({"action":"rehydrate","query":"current objective flow","paths":["src/compiler.ts","src/task.ts"]}),
        )]),
        stop(),
        calls(vec![("missing", "sourceContext", rows[7]["args"].clone())]),
        stop(),
    ]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    source_files(&fixture);
    let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
    tokio::time::timeout(
        Duration::from_secs(15),
        fixture
            .host
            .run(input(), &Resolver::new(false), runner.clone(), &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 9);
    assert_eq!(results[1]["isError"], true);
    let id = results[2]["details"]["artifactId"].as_str().unwrap();
    for (index, row) in [(0, 0), (2, 2), (3, 3), (4, 4), (6, 5), (7, 6)] {
        let output =
            json!({"content":results[index]["content"],"details":results[index]["details"]});
        let normalized: Value = serde_json::from_str(
            &output
                .to_string()
                .replace(id, "art_00000000000000000000000000000001"),
        )
        .unwrap();
        assert_eq!(normalized, rows[row]["output"], "{}", rows[row]["id"]);
    }
    assert_ne!(results[8]["isError"], true, "{:?}", results[8]);
    assert_ne!(results[8]["details"]["capsuleId"], capsule);
    assert_eq!(results[8]["details"]["status"], "current");
    assert_eq!(
        results[8]["details"]["historicalCapsules"][0]["id"],
        capsule
    );
    assert_eq!(
        results[8]["details"]["historicalCapsules"][0]["status"],
        "stale"
    );
    assert!(
        results[8]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ChangedCompiler")
    );
    std::fs::remove_file(fixture.root.join("src/compiler.ts")).unwrap();
    let mut next = input();
    next.command = "source-missing".into();
    fixture
        .host
        .run(next, &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(model.join().unwrap().len(), 6);
    let missing = session_tool_results(&fixture).pop().unwrap();
    assert_eq!(normalized_source_result(&missing), rows[7]["output"]);
    let bytes = fixture
        .store
        .page("session", "session", id, 0, 32768)
        .unwrap()
        .bytes;
    let saved = String::from_utf8(bytes.clone()).unwrap();
    assert!(saved.contains("class ContextCompiler"));
    assert!(!saved.contains("class ChangedCompiler"));
    let Fixture {
        _directory: directory,
        host,
        files,
        store,
        ..
    } = fixture;
    drop(host);
    drop(files);
    drop(store);
    let reopened = Store::open(&directory.path().join("store")).unwrap();
    assert_eq!(
        reopened
            .page("session", "session", id, 0, 32768)
            .unwrap()
            .bytes,
        bytes
    );
    let branch = ag1_core::task::branch_values(
        &reopened
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let records = ag1_core::context::capsule::list(&branch, 50);
    assert_eq!(records.len(), 2);
    let metadata = branch
        .iter()
        .position(|r| r["customType"] == "context_capsule")
        .unwrap();
    let result = branch
        .iter()
        .position(|r| r["message"]["toolCallId"] == "build")
        .unwrap();
    assert!(metadata < result);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_rejects_escaped_and_empty_queries_without_blocking_valid_calls() {
    use std::os::unix::fs::symlink;
    let (origin, model) = server(vec![
        calls(vec![
            (
                "escape",
                "sourceContext",
                json!({"query":"outside","paths":["../outside"]}),
            ),
            (
                "symlink",
                "sourceContext",
                json!({"query":"outside","paths":["linked.ts"]}),
            ),
            ("empty", "sourceContext", json!({"query":"  "})),
            (
                "valid",
                "sourceContext",
                json!({"query":"RuntimeObservation","paths":["src/observe.py"]}),
            ),
            ("catalog", "sourceContext", json!({"action":"list"})),
        ]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    source_files(&fixture);
    let outside = fixture._directory.path().join("outside");
    std::fs::write(&outside, "private outside sentinel").unwrap();
    symlink(&outside, fixture.root.join("linked.ts")).unwrap();
    let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    let requests = model.join().unwrap();
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 5);
    assert!(results[..3].iter().all(|r| r["isError"] == true));
    assert_ne!(results[3]["isError"], true);
    assert_eq!(results[4]["details"]["count"], 1);
    assert!(
        !serde_json::to_string(&requests)
            .unwrap()
            .contains("private outside sentinel")
    );
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "private outside sentinel"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_cancellation_drains_owned_search_before_settlement() {
    use std::os::unix::fs::PermissionsExt;
    let (origin, model) = server(vec![calls(vec![(
        "cancel-source",
        "sourceContext",
        json!({"query":"ContextCompiler"}),
    )])]);
    let fixture = fixture(&origin, "disabled");
    source_files(&fixture);
    let executable = fixture._directory.path().join("owned-slow-rg");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s' \"$$\" > rg-started\nsleep 30\n",
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runner = native_source_runner(&fixture, &executable).await;
    let host = fixture.host.clone();
    let running = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), runner, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fixture.root.join("rg-started").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let pid = std::fs::read_to_string(fixture.root.join("rg-started")).unwrap();
    fixture.host.cancel().unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap();
    assert!(fixture.host.settled().unwrap());
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert_eq!(model.join().unwrap().len(), 1);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["isError"], true);
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert!(ag1_core::context::capsule::list(&branch, 50).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_vague_queries_use_acknowledged_accesses_and_actual_git_changes() {
    for changed in [false, true] {
        let mut batch = vec![];
        if !changed {
            batch.push(("prior-read", "read", json!({"path":"src/task.ts"})));
        }
        batch.push((
            "vague",
            "sourceContext",
            json!({"query":"continue from before"}),
        ));
        let (origin, model) = server(vec![calls(batch), stop()]);
        let fixture = fixture(&origin, "disabled");
        source_files(&fixture);
        if changed {
            owned_repo_git(&fixture.root, &["init", "-b", "main"]);
            owned_repo_git(&fixture.root, &["add", "."]);
            owned_repo_git(&fixture.root, &["commit", "-m", "owned source fixture"]);
            std::fs::write(
                fixture.root.join("src/cache.rs"),
                "pub fn changed_cache() -> bool { false }\n",
            )
            .unwrap();
            std::fs::write(fixture.root.join("src/new.rs"), "pub struct NewSource;\n").unwrap();
            // The source reader must not invoke repository text conversion.
            std::fs::write(fixture.root.join(".gitattributes"), "*.rs diff=owned\n").unwrap();
            owned_repo_git(
                &fixture.root,
                &["config", "diff.owned.textconv", "touch unexpected-textconv"],
            );
        }
        let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await
            .unwrap();
        assert_eq!(model.join().unwrap().len(), 2);
        let results = session_tool_results(&fixture);
        let result = results.last().unwrap();
        assert_ne!(result["isError"], true, "{result}");
        let selected = result["details"]["files"].as_array().unwrap();
        let required = if changed {
            "src/cache.rs"
        } else {
            "src/task.ts"
        };
        assert!(selected.iter().any(|f| f["path"] == required), "{result}");
        if changed {
            assert!(selected.iter().any(|f| f["path"] == "src/new.rs"));
            assert!(
                result["details"]["retrieval"]["workingTreeChangedFiles"]
                    .as_u64()
                    .unwrap()
                    >= 2
            );
            assert!(!fixture.root.join("unexpected-textconv").exists());
        } else {
            assert_eq!(result["details"]["retrieval"]["recentAccessPaths"], 1);
            assert!(
                selected[0]["signals"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("recent-access"))
            );
        }
        assert!(!selected.iter().any(|f| f["path"] == "docs/old.md"));
    }
}

struct SourceCustodyBoundary {
    paths: Mutex<Option<(std::path::PathBuf, std::path::PathBuf)>>,
    point: ag1_storage::Boundary,
    fail: bool,
    observed: AtomicBool,
}
impl ag1_storage::FaultInjector for SourceCustodyBoundary {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point != self.point || self.observed.load(Ordering::Acquire) {
            return Ok(());
        }
        let paths = self.paths.lock().unwrap();
        let Some((blob, source)) = paths.as_ref() else {
            return Ok(());
        };
        if !blob.exists() {
            return Ok(());
        }
        self.observed.store(true, Ordering::Release);
        if self.fail {
            Err(ag1_storage::StorageError::Injected(point))
        } else {
            std::fs::write(source, "export class ChangedDuringCustody {}\n").unwrap();
            Ok(())
        }
    }
}
fn source_custody_fault(fixture: &Fixture, fault: &SourceCustodyBoundary) {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/source-context-tool-oracle.json"
    ))
    .unwrap();
    let body = oracle["rows"][2]["output"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .split("\ncapsule_artifact:")
        .next()
        .unwrap();
    *fault.paths.lock().unwrap() = Some((
        fixture
            .store
            .blob_path(&ag1_storage::digest(body.as_bytes()))
            .unwrap(),
        fixture.root.join("src/compiler.ts"),
    ));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_custody_failure_rejects_later_mutations_and_next_generation() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, model) = server(vec![calls(vec![
            (
                "capsule-fault",
                "sourceContext",
                json!({"query":"current objective flow","paths":["src/compiler.ts","src/task.ts"]}),
            ),
            (
                "must-not-write",
                "write",
                json!({"path":"unwanted.txt","content":"must not execute after uncertain custody"}),
            ),
        ])]);
        let fault = Arc::new(SourceCustodyBoundary {
            paths: Mutex::new(None),
            point,
            fail: true,
            observed: AtomicBool::new(false),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        source_files(&fixture);
        source_custody_fault(&fixture, &fault);
        let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
        let resolver = Resolver::new(false);
        assert!(matches!(
            fixture.host.run(input(), &resolver, runner, &ignore).await,
            Err(DomainError::DurabilityUnknown)
        ));
        assert!(fault.observed.load(Ordering::Acquire));
        assert_eq!(resolver.calls.load(Ordering::Acquire), 1);
        assert_eq!(model.join().unwrap().len(), 1);
        assert!(!fixture.root.join("unwanted.txt").exists());
        let results = session_tool_results(&fixture);
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r["isError"] == true));
        let branch = ag1_core::task::branch_values(
            &fixture
                .store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert!(ag1_core::context::capsule::list(&branch, 50).is_empty());
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_does_not_claim_current_after_source_changes_during_custody() {
    let (origin, model) = server(vec![
        calls(vec![(
            "capsule-change",
            "sourceContext",
            json!({"query":"current objective flow","paths":["src/compiler.ts","src/task.ts"]}),
        )]),
        stop(),
    ]);
    let fault = Arc::new(SourceCustodyBoundary {
        paths: Mutex::new(None),
        point: ag1_storage::Boundary::BeforeAck,
        fail: false,
        observed: AtomicBool::new(false),
    });
    let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
    source_files(&fixture);
    source_custody_fault(&fixture, &fault);
    let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert!(fault.observed.load(Ordering::Acquire));
    assert_eq!(model.join().unwrap().len(), 2);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["details"]["status"], "stale");
    assert!(
        results[0]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Treat these slices as historical evidence")
    );
    let id = results[0]["details"]["artifactId"].as_str().unwrap();
    let saved = String::from_utf8(
        fixture
            .store
            .page("session", "session", id, 0, 32768)
            .unwrap()
            .bytes,
    )
    .unwrap();
    assert!(saved.contains("class ContextCompiler"));
    assert!(!saved.contains("ChangedDuringCustody"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_source_reuses_recoverable_capsule_metadata_in_the_same_batch() {
    let args = json!({"query":"TaskState","paths":["src/task.ts"]});
    let (origin, model) = server(vec![
        calls(vec![
            ("first-capsule", "sourceContext", args.clone()),
            ("second-capsule", "sourceContext", args),
            ("capsules", "sourceContext", json!({"action":"list"})),
        ]),
        stop(),
    ]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    source_files(&fixture);
    let runner = native_source_runner(&fixture, "/usr/bin/rg".as_ref()).await;
    fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(model.join().unwrap().len(), 2);
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r["isError"] != true), "{results:?}");
    assert_eq!(
        results[0]["details"]["capsuleId"],
        results[1]["details"]["capsuleId"]
    );
    assert_eq!(
        results[0]["details"]["artifactId"],
        results[1]["details"]["artifactId"]
    );
    assert_eq!(results[2]["details"]["count"], 1);
    let branch = ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        branch
            .iter()
            .filter(|r| r["customType"] == "context_capsule")
            .count(),
        1
    );
}

async fn native_todo_runner(
    fixture: &Fixture,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{session::adapters::NativeAdapters, todo_tools::TodoTools};
    let adapters = NativeAdapters {
        todo: Some(Arc::new(TodoTools::new(fixture.workspace.clone()))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["todo".into(), "write".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "todo" || tool["name"] == "write")
        .collect();
    fixture
        .host
        .select_tools(
            "native-todo-tools",
            "native-todo-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_todo_lifecycle_archives_and_replaces_plans_on_one_worker() {
    let first = json!([{"id":"t1","title":"Inspect source","status":"in_progress"},{"id":"t2","title":"Implement changes","status":"pending"}]);
    let (origin, model) = server(vec![
        calls(vec![
            ("query", "todo", json!({})),
            (
                "first-plan",
                "todo",
                json!({"planAction":"new-plan","phase":"Alpha","todos":first}),
            ),
            ("invalid-clear", "todo", json!({"clear":true})),
            (
                "advance",
                "todo",
                json!({"todos":[{"id":"t1","title":"Inspect source","status":"completed"},{"id":"t2","title":"Implement changes","status":"in_progress"}]}),
            ),
            (
                "replace",
                "todo",
                json!({"planAction":"new-plan","phase":"Beta","todos":[{"title":"Verify new plan","status":"in_progress"}]}),
            ),
        ]),
        calls(vec![
            ("query-current", "todo", json!({})),
            ("archive", "todo", json!({"planAction":"archive"})),
            (
                "fresh",
                "todo",
                json!({"todos":[{"title":"Final check","status":"in_progress"}]}),
            ),
            ("second-invalid-clear", "todo", json!({"clear":true})),
            (
                "work",
                "write",
                json!({"path":"continued.txt","content":"Bookkeeping errors do not block productive work"}),
            ),
            (
                "finish",
                "todo",
                json!({"todos":[{"id":"t1","title":"Final check","status":"completed"}]}),
            ),
            ("clear", "todo", json!({"clear":true})),
        ]),
        stop(),
    ]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    let runner = native_todo_runner(&fixture).await;
    tokio::time::timeout(
        Duration::from_secs(10),
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    let requests = model.join().unwrap();
    assert_eq!(requests.len(), 3);
    let user = |request: &Value| {
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .unwrap()
            .to_string()
    };
    assert!(user(&requests[1]).contains("phase=Beta"));
    assert!(user(&requests[1]).contains("todo_focus: t1 Verify new plan"));
    assert!(!user(&requests[1]).contains("phase=Alpha"));
    assert!(!user(&requests[2]).contains("Checklist reconciliation"));
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 12);
    for (index, result) in results.iter().enumerate() {
        assert_eq!(
            result["isError"] == true,
            [2, 8].contains(&index),
            "{index}: {result}"
        );
    }
    assert!(fixture.root.join("continued.txt").exists());
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&journal).unwrap();
    let snapshots: Vec<_> = branch
        .iter()
        .filter(|r| r["customType"] == "todo_list")
        .collect();
    let archives: Vec<_> = branch
        .iter()
        .filter(|r| r["customType"] == "todo_archive")
        .collect();
    assert_eq!(snapshots.len(), 7);
    assert_eq!(archives.len(), 2);
    assert_eq!(archives[0]["data"]["snapshot"], snapshots[1]["data"]);
    assert_eq!(archives[0]["data"]["reason"], "new-plan");
    assert_eq!(archives[1]["data"]["snapshot"], snapshots[2]["data"]);
    assert_eq!(archives[1]["data"]["reason"], "archive");
    assert_eq!(snapshots[0]["data"]["phase"], "Alpha");
    assert_eq!(snapshots[2]["data"]["phase"], "Beta");
    assert_ne!(
        snapshots[0]["data"]["planId"],
        snapshots[2]["data"]["planId"]
    );
    assert!(snapshots[3]["data"].get("planId").is_none());
    assert!(snapshots[3]["data"].get("phase").is_none());
    for archive in &archives {
        let index = branch
            .iter()
            .position(|r| r["id"] == archive["id"])
            .unwrap();
        assert_eq!(branch[index + 1]["customType"], "todo_list");
        assert_eq!(branch[index + 1]["parentId"], archive["id"]);
    }
    let latest = ag1_core::tools::todo::latest(&branch).unwrap();
    assert_eq!(latest["items"], json!([]));
    let Fixture {
        _directory: directory,
        host,
        files,
        store,
        ..
    } = fixture;
    drop(host);
    drop(files);
    drop(store);
    let reopened = Store::open(&directory.path().join("store")).unwrap();
    let recovered = ag1_core::task::branch_values(
        &reopened
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ag1_core::tools::todo::latest(&recovered), Some(latest));
    assert_eq!(recovered, branch);
}

struct FaultedTodoJournal {
    registry: Arc<Registry>,
    fault: Arc<ArmedStorageFault>,
}
impl ag1_platform::session::ToolRunner for FaultedTodoJournal {
    fn execute(
        &self,
        scope: &ag1_core::tools::registry::Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ag1_platform::session::ToolSelection,
        operation: &str,
        _cancel: &AtomicBool,
        _revision: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
        assert_eq!(calls.len(), 1);
        let call = self.registry.prepare(
            operation,
            &calls[0].id,
            &calls[0].name,
            &calls[0].block["arguments"],
            true,
        )?;
        let journal = selection.journal.as_ref().unwrap();
        let mut stale = scope.clone();
        stale.revision += 1;
        assert!(matches!(
            journal.apply_todo(&stale, &call),
            Err(
                ag1_platform::session::tool_journal::JournalMutationError::BeforeEffect(
                    DomainError::Conflict
                )
            )
        ));
        self.fault.armed.store(true, Ordering::SeqCst);
        journal.apply_todo(scope, &call)?;
        panic!("todo journal fault must prevent successful acknowledgement");
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_todo_archive_and_replacement_are_atomic_at_commit_and_ack_failures() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, model) = server(vec![
            calls(vec![(
                "first",
                "todo",
                json!({"planAction":"new-plan","phase":"First","todos":[{"title":"Initial work","status":"in_progress"}]}),
            )]),
            stop(),
            calls(vec![(
                "replace",
                "todo",
                json!({"planAction":"new-plan","phase":"Second","todos":[{"title":"Replacement work","status":"in_progress"}]}),
            )]),
        ]);
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        let native = native_todo_runner(&fixture).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), native.clone(), &ignore)
            .await
            .unwrap();
        let before = ag1_core::task::branch_values(
            &fixture
                .store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let previous = ag1_core::tools::todo::latest(&before).unwrap();
        let runner = Arc::new(FaultedTodoJournal {
            registry: native.registry.clone(),
            fault,
        });
        let mut next = input();
        next.command = "todo-replace".into();
        let resolver = Resolver::new(false);
        assert!(
            fixture
                .host
                .run(next, &resolver, runner, &ignore)
                .await
                .is_err()
        );
        let mut after = input();
        after.command = "after-todo-fault".into();
        assert!(
            fixture
                .host
                .run(after, &resolver, native.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 1);
        assert_eq!(model.join().unwrap().len(), 3);
        drop(native);
        let Fixture {
            _directory: directory,
            host,
            files,
            store,
            ..
        } = fixture;
        drop(host);
        drop(files);
        drop(store);
        let reopened = Store::open(&directory.path().join("store")).unwrap();
        let branch = ag1_core::task::branch_values(
            &reopened
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let current = ag1_core::tools::todo::latest(&branch).unwrap();
        let archives: Vec<_> = branch
            .iter()
            .filter(|r| r["customType"] == "todo_archive")
            .collect();
        if point == ag1_storage::Boundary::BeforeCommit {
            assert_eq!(current, previous);
            assert!(archives.is_empty());
        } else {
            assert_eq!(current["phase"], "Second");
            assert_eq!(archives.len(), 1);
            assert_eq!(archives[0]["data"]["snapshot"], previous);
            let at = branch
                .iter()
                .position(|r| r["id"] == archives[0]["id"])
                .unwrap();
            assert_eq!(branch[at + 1]["customType"], "todo_list");
            assert_eq!(branch[at + 1]["parentId"], archives[0]["id"]);
        }
    }
}

async fn native_observation_runner(
    fixture: &Fixture,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{
        observation_tools::ObservationTools, session::adapters::NativeAdapters,
        task_tools::TaskTools,
    };
    let adapters = NativeAdapters {
        observation: Some(Arc::new(ObservationTools::new(fixture.workspace.clone()))),
        task: Some(Arc::new(TaskTools::new(fixture.workspace.clone()))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["taskState".into(), "runtimeObservation".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| tool["name"] == "taskState" || tool["name"] == "runtimeObservation")
        .collect();
    fixture
        .host
        .select_tools(
            "native-observation-tools",
            "native-observation-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}

fn observation_args(result: &str, feature: &str) -> Value {
    json!({"feature":feature,"result":result,"environment":"owned fixture","revision":"workspace:fixture","reporter":"agent","reproduction":"Open the fixture view","notes":"Observed fixture behavior"})
}
async fn completed_observation_task(fixture: &Fixture) {
    let task = ag1_core::task::TaskState::reset("Repair compiler startup", NOW)
        .unwrap()
        .set(&json!({"status":"completed"}), NOW)
        .unwrap();
    fixture
        .host
        .append_task_state("completed-seed", "completed-seed", NOW, task, &ignore)
        .await
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_observation_preserves_feedback_and_updates_related_task_before_next_request() {
    for (result, feature, related) in [
        ("pass", "Compiler startup", true),
        ("fail", "Compiler startup", true),
        ("blocked", "Compiler startup", true),
        ("partial", "Animation timing", false),
    ] {
        let mut invalid = observation_args(result, feature);
        invalid["notes"] = json!("  ");
        let (origin, model) = server(vec![
            calls(vec![
                ("invalid-observation", "runtimeObservation", invalid),
                (
                    "feedback",
                    "runtimeObservation",
                    observation_args(result, feature),
                ),
                ("task-show", "taskState", json!({"action":"show"})),
            ]),
            stop(),
        ]);
        let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
        completed_observation_task(&fixture).await;
        let runner = native_observation_runner(&fixture).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await
            .unwrap();
        let requests = model.join().unwrap();
        assert_eq!(requests.len(), 2);
        let results = session_tool_results(&fixture);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0]["isError"], true);
        assert_ne!(results[1]["isError"], true, "{:?}", results);
        assert_ne!(results[2]["isError"], true);
        let task = &results[2]["details"]["snapshot"];
        assert_eq!(
            task["status"],
            if related && result != "pass" {
                "active"
            } else {
                "completed"
            }
        );
        assert_eq!(
            task["entries"].as_array().unwrap().len(),
            usize::from(related)
        );
        let branch = ag1_core::task::branch_values(
            &fixture
                .store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let observations: Vec<_> = branch
            .iter()
            .filter(|r| r["customType"] == "runtime_observation")
            .collect();
        assert_eq!(observations.len(), 1);
        let row = observations[0];
        assert_eq!(row["id"], results[1]["details"]["entryId"]);
        assert_eq!(row["data"]["sources"], json!(["feedback"]));
        assert_eq!(row["data"]["reporter"], "agent");
        if related {
            assert_eq!(task["entries"][0]["kind"], "OBSERVATION");
            assert_eq!(
                task["entries"][0]["sources"],
                json!([format!("session-entry:{}", row["id"].as_str().unwrap())])
            );
            assert_eq!(
                task["entries"][0]["id"],
                results[1]["details"]["taskStateEntryId"]
            );
            assert_eq!(
                task["entries"][0]["confidence"],
                if result == "pass" { "medium" } else { "low" }
            );
            let index = branch.iter().position(|r| r["id"] == row["id"]).unwrap();
            assert_eq!(branch[index + 1]["customType"], "task_state");
            assert_eq!(branch[index + 1]["parentId"], row["id"]);
        } else {
            assert!(results[1]["details"].get("taskStateEntryId").is_none());
        }
        let observation = ag1_core::evidence::RuntimeObservation::parse(&row["data"]).unwrap();
        assert!(
            !observation.verified_for(feature, "owned fixture", "workspace:fixture", |_| {
                ag1_core::evidence::Freshness::Current
            })
        );
        let user = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|m| m["role"] == "user")
            .unwrap()
            .to_string();
        assert!(user.contains("Runtime verification observations"));
        assert!(user.contains(feature));
        assert!(user.contains("reporter=agent"));
    }
}
struct FaultedObservationJournal {
    registry: Arc<Registry>,
    fault: Arc<ArmedStorageFault>,
}
impl ag1_platform::session::ToolRunner for FaultedObservationJournal {
    fn execute(
        &self,
        scope: &ag1_core::tools::registry::Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ag1_platform::session::ToolSelection,
        operation: &str,
        _cancel: &AtomicBool,
        _revision: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
        let call = self.registry.prepare(
            operation,
            &calls[0].id,
            &calls[0].name,
            &calls[0].block["arguments"],
            true,
        )?;
        self.fault.armed.store(true, Ordering::SeqCst);
        selection
            .journal
            .as_ref()
            .unwrap()
            .apply_observation(scope, &call)?;
        panic!("observation journal fault must prevent acknowledgement");
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_observation_and_related_task_change_commit_atomically_under_faults() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, model) = server(vec![calls(vec![(
            "failure-feedback",
            "runtimeObservation",
            observation_args("fail", "Compiler startup"),
        )])]);
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let fixture = fixture_seeded_with_fault(&origin, "disabled", vec![], Some(fault.clone()));
        completed_observation_task(&fixture).await;
        let native = native_observation_runner(&fixture).await;
        let runner = Arc::new(FaultedObservationJournal {
            registry: native.registry.clone(),
            fault,
        });
        let resolver = Resolver::new(false);
        assert!(
            fixture
                .host
                .run(input(), &resolver, runner, &ignore)
                .await
                .is_err()
        );
        let mut next = input();
        next.command = "after-observation-fault".into();
        assert!(
            fixture
                .host
                .run(next, &resolver, native.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 1);
        assert_eq!(model.join().unwrap().len(), 1);
        drop(native);
        let Fixture {
            _directory: directory,
            host,
            files,
            store,
            ..
        } = fixture;
        drop(host);
        drop(files);
        drop(store);
        let reopened = Store::open(&directory.path().join("store")).unwrap();
        let branch = ag1_core::task::branch_values(
            &reopened
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let task = ag1_core::runtime::context::latest_task(&branch).unwrap();
        let records: Vec<_> = branch
            .iter()
            .filter(|r| r["customType"] == "runtime_observation")
            .collect();
        if point == ag1_storage::Boundary::BeforeCommit {
            assert!(records.is_empty());
            assert_eq!(task.value()["status"], "completed");
            assert_eq!(task.value()["entries"], json!([]));
        } else {
            assert_eq!(records.len(), 1);
            assert_eq!(task.value()["status"], "active");
            assert_eq!(
                task.value()["entries"][0]["sources"],
                json!([format!(
                    "session-entry:{}",
                    records[0]["id"].as_str().unwrap()
                )])
            );
            let index = branch
                .iter()
                .position(|r| r["id"] == records[0]["id"])
                .unwrap();
            assert_eq!(branch[index + 1]["customType"], "task_state");
            assert_eq!(branch[index + 1]["parentId"], records[0]["id"]);
        }
    }
}

async fn native_episode_runner(
    fixture: &Fixture,
) -> Arc<ag1_platform::session::files::NativeToolRunner> {
    use ag1_platform::{episode_tools::EpisodeTools, session::adapters::NativeAdapters};
    let adapters = NativeAdapters {
        episode: Some(Arc::new(EpisodeTools::new(
            fixture.workspace.clone(),
            fixture.store.clone(),
            "session".into(),
        ))),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["episode".into(), "write".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry.public_tools();
    fixture
        .host
        .select_tools(
            "native-episode-tools",
            "native-episode-tools",
            NOW,
            tools,
            &ignore,
        )
        .await
        .unwrap();
    Arc::new(ag1_platform::session::files::NativeToolRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    })
}
fn candidate_reply(text: &str) -> Value {
    json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":123,"completion_tokens":45}})
}
fn episode_branch(fixture: &Fixture) -> Vec<Value> {
    ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_draft_is_isolated_and_only_explicit_close_publishes_memory_on_one_worker() {
    use ag1_core::context::ContextArtifactPort;
    let candidate = json!({"version":1,"authority":"model-authored-candidate","synopsis":"Compiler startup diagnosed", "claims":[],"fileRevisions":[],"verificationSources":[]});
    let (origin, model) = server(vec![
        calls(vec![
            (
                "open",
                "episode",
                json!({"action":"begin","goal":"Diagnose compiler startup","title":"Compiler diagnosis"}),
            ),
            ("draft", "episode", json!({"action":"draft"})),
            ("still-open", "episode", json!({"action":"list"})),
            (
                "unsupported-close",
                "episode",
                json!({"action":"close","outcome":"completed","synopsis":"Premature success"}),
            ),
            (
                "close",
                "episode",
                json!({"action":"close","outcome":"partial","synopsis":"Compiler startup diagnosed"}),
            ),
            ("list", "episode", json!({"action":"list"})),
            (
                "search",
                "episode",
                json!({"action":"search","query":"compiler startup"}),
            ),
        ]),
        candidate_reply(&candidate.to_string()),
        json!({"fixtureEpisodeShow":true}),
        stop(),
    ]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    completed_observation_task(&fixture).await;
    let runner = native_episode_runner(&fixture).await;
    let resolver = Resolver::new(false);
    let turn = tokio::time::timeout(
        Duration::from_secs(8),
        fixture
            .host
            .run(input(), &resolver, runner.clone(), &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(turn.generations, 3); // the isolated draft is not a parent generation
    let requests = model.join().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[1]["model"], requests[0]["model"]);
    assert!(requests[1].get("tools").is_none());
    assert_eq!(requests[1]["messages"].as_array().unwrap().len(), 2); // isolated system + user only
    assert!(
        requests[1]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("index, not authoritative evidence")
    );
    let results = session_tool_results(&fixture);
    assert_eq!(results.len(), 8, "{results:?}");
    assert_eq!(
        results[1]["details"]["status"], "candidate-only",
        "{results:?}"
    );
    assert_eq!(results[1]["details"]["inputTokens"], 123);
    assert_eq!(results[1]["details"]["outputTokens"], 45);
    assert_eq!(results[2]["details"]["count"], 0);
    assert_eq!(results[2]["details"]["id"], results[0]["details"]["id"]);
    assert_eq!(results[3]["isError"], true);
    assert_eq!(results[4]["details"]["status"], "partial");
    assert_eq!(results[5]["details"]["count"], 1);
    assert_eq!(results[6]["details"]["count"], 1);
    assert_eq!(results[7]["details"]["id"], results[0]["details"]["id"]);
    assert!(
        results[7]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("MODEL-AUTHORED INDEX; NOT EVIDENCE")
    );
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = episode_branch(&fixture);
    assert!(ag1_core::context::episode::find_open(&branch).is_none());
    let episodes = ag1_core::context::episode::list_closed(&branch, 100);
    assert_eq!(episodes.len(), 1);
    let record = episodes[0].value();
    let artifacts = ag1_storage::context::ContextStore::new(
        &fixture.store,
        SessionId::new("session").unwrap(),
        "session",
        "inspect-episode",
        NOW,
        Default::default(),
    )
    .unwrap();
    let trajectory = artifacts
        .get(record["trajectoryArtifactId"].as_str().unwrap())
        .unwrap();
    let start = branch
        .iter()
        .position(|e| e["id"] == record["startEntryId"])
        .unwrap();
    let end = branch
        .iter()
        .position(|e| e["id"] == record["endEntryId"])
        .unwrap();
    let mut expected = String::new();
    for entry in &branch[start..=end] {
        let raw = &journal
            .entries
            .iter()
            .find(|r| r.id == entry["id"].as_str().unwrap())
            .unwrap()
            .raw;
        expected.push_str(std::str::from_utf8(raw).unwrap());
        if !expected.ends_with('\n') {
            expected.push('\n');
        }
    }
    assert!(
        std::fs::read_to_string(fixture.store.blob_path(&trajectory.sha256).unwrap())
            .unwrap()
            .ends_with(&expected)
    );
    let draft_trajectory = artifacts
        .get(results[1]["details"]["artifactId"].as_str().unwrap())
        .unwrap();
    assert!(
        !std::fs::read_to_string(fixture.store.blob_path(&draft_trajectory.sha256).unwrap())
            .unwrap()
            .contains("candidate-only")
    );
    let candidate_artifact = artifacts
        .get(
            results[1]["details"]["candidateArtifactId"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert!(
        std::fs::read_to_string(fixture.store.blob_path(&candidate_artifact.sha256).unwrap())
            .unwrap()
            .contains("Compiler startup diagnosed")
    );
    assert!(fixture.host.pending_transport().unwrap().is_none());
    drop(artifacts);
    drop(runner);
    let Fixture {
        _directory,
        store,
        host,
        files,
        ..
    } = fixture;
    drop(host);
    drop(files);
    drop(store);
    let reopened = Store::open(&_directory.path().join("store")).unwrap();
    assert_eq!(
        ag1_core::task::branch_values(
            &reopened
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap()
        )
        .unwrap(),
        branch
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_invalid_truncated_and_interrupted_drafts_retain_custody_without_closing() {
    for mode in ["invalid", "truncated", "interrupted"] {
        let mut reply = candidate_reply("not a candidate");
        if mode == "truncated" {
            reply["choices"][0]["finish_reason"] = json!("length");
        }
        if mode == "interrupted" {
            reply = json!({"fixtureDisconnect":true});
        }
        let mut replies = vec![
            calls(vec![
                (
                    "open",
                    "episode",
                    json!({"action":"begin","goal":"Investigate compiler startup"}),
                ),
                ("draft", "episode", json!({"action":"draft"})),
                (
                    "after-draft",
                    "write",
                    json!({"path":"after-draft.txt","content":"Allowed only after known settlement"}),
                ),
            ]),
            reply,
        ];
        if mode != "interrupted" {
            replies.push(stop());
        }
        let (origin, model) = server(replies);
        let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
        let runner = native_episode_runner(&fixture).await;
        let outcome = fixture
            .host
            .run(input(), &Resolver::new(false), runner, &ignore)
            .await;
        if mode == "interrupted" {
            assert!(
                matches!(outcome, Err(DomainError::DurabilityUnknown)),
                "{mode}"
            );
            assert!(fixture.host.pending_transport().unwrap().is_some());
            assert!(!fixture.root.join("after-draft.txt").exists());
        } else {
            outcome.unwrap();
            assert!(fixture.host.pending_transport().unwrap().is_none());
            assert!(fixture.root.join("after-draft.txt").exists());
        }
        assert_eq!(
            model.join().unwrap().len(),
            if mode == "interrupted" { 2 } else { 3 }
        );
        let results = session_tool_results(&fixture);
        assert_eq!(results.len(), 3);
        assert_eq!(results[1]["isError"], true);
        let branch = episode_branch(&fixture);
        assert!(ag1_core::context::episode::find_open(&branch).is_some());
        assert!(ag1_core::context::episode::list_closed(&branch, 100).is_empty());
        let snapshots = fixture
            .store
            .artifact_catalog("session", None, 100, Some("episode-draft-output"))
            .unwrap();
        assert_eq!(snapshots.len(), 1, "{mode}");
        let page = fixture
            .store
            .page(
                "session",
                &snapshots[0].position.origin,
                &snapshots[0].position.id,
                0,
                32768,
            )
            .unwrap();
        let snapshot: Value = serde_json::from_slice(&page.bytes).unwrap();
        assert_eq!(
            snapshot["content"][0]["text"],
            if mode == "interrupted" {
                "partial"
            } else {
                "not a candidate"
            }
        );
        if mode == "interrupted" {
            assert_eq!(snapshot["settled"], false);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_cancellation_during_resolution_unblocks_the_only_worker_without_dispatch() {
    let (origin, model) = server(vec![calls(vec![
        (
            "open",
            "episode",
            json!({"action":"begin","goal":"Investigate compiler startup"}),
        ),
        ("draft", "episode", json!({"action":"draft"})),
        (
            "after-draft",
            "write",
            json!({"path":"after-draft.txt","content":"Must not run"}),
        ),
    ])]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    let runner = native_episode_runner(&fixture).await;
    let mut resolver = Resolver::new(true);
    resolver.pause_index = 1;
    let resolver = Arc::new(resolver);
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let run = tokio::spawn(async move {
        host.run(input(), worker_resolver.as_ref(), runner, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
        .await
        .unwrap();
    fixture.host.cancel().unwrap();
    let output = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap();
    assert!(output.is_err());
    // Deliberately never release the resolver: dropping unadmitted preparation is safe.
    assert_eq!(model.join().unwrap().len(), 1);
    assert_eq!(resolver.calls.load(Ordering::Acquire), 2);
    assert!(fixture.host.pending_transport().unwrap().is_none());
    assert!(!fixture.root.join("after-draft.txt").exists());
    assert_eq!(session_tool_results(&fixture).len(), 3);
    assert!(ag1_core::context::episode::list_closed(&episode_branch(&fixture), 100).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_queue_uses_shared_scheduler_and_rejects_changed_configuration_before_dispatch()
 {
    let (origin, model) = server(vec![
        calls(vec![
            (
                "open",
                "episode",
                json!({"action":"begin","goal":"Investigate compiler startup"}),
            ),
            ("draft", "episode", json!({"action":"draft"})),
            (
                "after-draft",
                "write",
                json!({"path":"after-draft.txt","content":"Must not run"}),
            ),
        ]),
        stop(),
    ]);
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    let runner = native_episode_runner(&fixture).await;
    let mut resolver = Resolver::new(true);
    resolver.pause_index = 1;
    let resolver = Arc::new(resolver);
    let worker_resolver = resolver.clone();
    let host = fixture.host.clone();
    let run = tokio::spawn(async move {
        host.run(input(), worker_resolver.as_ref(), runner, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
        .await
        .unwrap();
    let ticket = fixture
        .scheduler
        .submit(
            &origin,
            ag1_core::OperationId::new("other-host-owned-work").unwrap(),
        )
        .unwrap();
    let permit = fixture.scheduler.start(&ticket).unwrap().unwrap();
    resolver.release.notify_one();
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture.scheduler.status(&origin).unwrap().waiting == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 1);
    let mut model_config = fixture.model.clone();
    model_config["id"] = json!("replacement");
    fixture
        .host
        .select_model(
            "change-draft-model",
            "change-draft-model",
            NOW,
            model_config,
            &ignore,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while resolver.calls.load(Ordering::Acquire) < 3 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(session_tool_results(&fixture).len(), 3);
    assert_eq!(fixture.scheduler.status(&origin).unwrap().active, 1);
    permit.terminal().unwrap();
    tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let requests = model.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1]["model"], "replacement");
    assert_eq!(fixture.scheduler.status(&origin).unwrap().waiting, 0);
    assert!(fixture.host.pending_transport().unwrap().is_none());
    assert!(!fixture.root.join("after-draft.txt").exists());
    assert!(ag1_core::context::episode::list_closed(&episode_branch(&fixture), 100).is_empty());
}

struct EpisodeBlobBoundary {
    root: Mutex<Option<std::path::PathBuf>>,
    marker: String,
    point: ag1_storage::Boundary,
    observed: AtomicBool,
}
impl ag1_storage::FaultInjector for EpisodeBlobBoundary {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point != self.point || self.observed.load(Ordering::Acquire) {
            return Ok(());
        }
        let root = self.root.lock().unwrap();
        let Some(root) = root.as_ref() else {
            return Ok(());
        };
        let found = std::fs::read_dir(root)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| {
                std::fs::read_to_string(e.path()).is_ok_and(|text| text.contains(&self.marker))
            });
        if found {
            self.observed.store(true, Ordering::Release);
            Err(ag1_storage::StorageError::Injected(point))
        } else {
            Ok(())
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_custody_commit_and_ack_failures_stop_close_and_later_effects() {
    for action in ["draft", "close"] {
        for point in [
            ag1_storage::Boundary::BeforeCommit,
            ag1_storage::Boundary::BeforeAck,
        ] {
            let fault = Arc::new(EpisodeBlobBoundary {
                root: Mutex::new(None),
                marker: if action == "draft" {
                    "draft-custody-marker".into()
                } else {
                    "\"type\":\"ag1_episode_trajectory\"".into()
                },
                point,
                observed: AtomicBool::new(false),
            });
            let args = if action == "draft" {
                json!({"action":"draft"})
            } else {
                json!({"action":"close","outcome":"partial","synopsis":"Diagnosed"})
            };
            let mut replies = vec![calls(vec![
                (
                    "open",
                    "episode",
                    json!({"action":"begin","goal":"Investigate compiler startup"}),
                ),
                ("custody", "episode", args),
                (
                    "after-custody",
                    "write",
                    json!({"path":"forbidden.txt","content":"Must not run"}),
                ),
            ])];
            if action == "draft" {
                replies.push(candidate_reply(&json!({"version":1,"authority":"model-authored-candidate","synopsis":"draft-custody-marker","claims":[],"fileRevisions":[],"verificationSources":[]}).to_string()));
            }
            let (origin, model) = server(replies);
            let fixture =
                fixture_configured(&origin, "disabled", vec![], Some(fault.clone()), None, 1);
            *fault.root.lock().unwrap() = Some(fixture._directory.path().join("store/blobs"));
            let runner = native_episode_runner(&fixture).await;
            let output = fixture
                .host
                .run(input(), &Resolver::new(false), runner, &ignore)
                .await;
            assert!(
                matches!(output, Err(DomainError::DurabilityUnknown)),
                "{action} {point:?}"
            );
            assert!(fault.observed.load(Ordering::Acquire));
            assert_eq!(
                model.join().unwrap().len(),
                if action == "draft" { 2 } else { 1 }
            );
            assert!(!fixture.root.join("forbidden.txt").exists());
            let branch = episode_branch(&fixture);
            assert!(ag1_core::context::episode::find_open(&branch).is_some());
            assert!(ag1_core::context::episode::list_closed(&branch, 100).is_empty());
            assert!(fixture.host.pending_transport().unwrap().is_none());
        }
    }
}

struct StalledEpisode {
    origin: String,
    ready: tokio::sync::oneshot::Receiver<()>,
    release: std::sync::mpsc::SyncSender<()>,
    model: thread::JoinHandle<Vec<Value>>,
}
fn stalled_episode() -> StalledEpisode {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::sync_channel(1);
    let model = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let first = read_request(&mut socket);
        let reply = calls(vec![
            (
                "open",
                "episode",
                json!({"action":"begin","goal":"Investigate compiler startup"}),
            ),
            ("draft", "episode", json!({"action":"draft"})),
            (
                "after-draft",
                "write",
                json!({"path":"forbidden.txt","content":"Must not run"}),
            ),
        ]);
        let body = format!("data: {reply}\n\ndata: [DONE]\n\n");
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().unwrap();
        let second = read_request(&mut socket);
        let frame = format!(
            "data: {}\n\n",
            json!({"choices":[{"delta":{"content":"partial owned draft"}}]})
        );
        write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{frame}\r\n",frame.len()).unwrap();
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        drop(socket);
        vec![first, second]
    });
    StalledEpisode {
        origin,
        ready,
        release,
        model,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_cancellation_during_transport_retains_partial_output_and_exact_quarantine()
{
    let StalledEpisode {
        origin,
        ready,
        release,
        model,
    } = stalled_episode();
    let fixture = fixture_configured(&origin, "disabled", vec![], None, None, 1);
    let runner = native_episode_runner(&fixture).await;
    let host = fixture.host.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), runner, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), ready)
        .await
        .unwrap()
        .unwrap();
    fixture.host.cancel().unwrap();
    let output = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap();
    assert!(output.is_err());
    let ticket = fixture.host.pending_transport().unwrap().unwrap();
    assert_eq!(fixture.scheduler.status(&origin).unwrap().quarantined, 1);
    assert!(!fixture.root.join("forbidden.txt").exists());
    assert!(ag1_core::context::episode::list_closed(&episode_branch(&fixture), 100).is_empty());
    let snapshots = fixture
        .store
        .artifact_catalog("session", None, 100, Some("episode-draft-output"))
        .unwrap();
    assert_eq!(snapshots.len(), 1);
    release.send(()).unwrap();
    assert_eq!(model.join().unwrap().len(), 2);
    assert!(fixture.host.pending_transport().unwrap().is_some());
    fixture.scheduler.reconcile(&ticket).unwrap();
    assert!(fixture.host.pending_transport().unwrap().is_none());
}

struct FaultedEpisodeJournal {
    registry: Arc<Registry>,
    fault: Arc<ArmedStorageFault>,
    store: Arc<Store>,
}
impl ag1_platform::session::ToolRunner for FaultedEpisodeJournal {
    fn execute(
        &self,
        scope: &ag1_core::tools::registry::Scope,
        calls: &[ag1_core::runtime::session::PendingCall],
        selection: &ag1_platform::session::ToolSelection,
        operation: &str,
        _: &AtomicBool,
        _: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
        let call = self.registry.prepare(
            operation,
            &calls[0].id,
            &calls[0].name,
            &calls[0].block["arguments"],
            true,
        )?;
        let journal = selection.journal.as_ref().unwrap();
        let stored = journal.stored_snapshot(scope)?;
        let mut artifacts = ag1_storage::context::ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            "session",
            operation,
            NOW,
            Default::default(),
        )?;
        let record = ag1_core::context::episode::close_record(
            &stored,
            &mut artifacts,
            NOW,
            call.call_id(),
            call.args(),
        )?;
        let mut stale = scope.clone();
        stale.revision += 1;
        assert!(matches!(
            journal.commit_state(&stale, &call, "context_episode", record.value().clone()),
            Err(
                ag1_platform::session::tool_journal::JournalMutationError::BeforeEffect(
                    DomainError::Conflict
                )
            )
        ));
        let mut wrong = record.value().clone();
        wrong["synopsis"] = json!("Substituted acceptance");
        assert!(matches!(
            journal.commit_state(scope, &call, "context_episode", wrong),
            Err(
                ag1_platform::session::tool_journal::JournalMutationError::BeforeEffect(
                    DomainError::Conflict
                )
            )
        ));
        self.fault.armed.store(true, Ordering::Release);
        journal.commit_state(scope, &call, "context_episode", record.value().clone())?;
        panic!("episode metadata fault must prevent acknowledgement");
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_close_metadata_has_one_owner_and_recovers_commit_or_lost_ack_exactly() {
    use ag1_core::context::ContextArtifactPort;
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let (origin, model) = server(vec![
            calls(vec![(
                "open",
                "episode",
                json!({"action":"begin","goal":"Diagnose compiler startup"}),
            )]),
            stop(),
            calls(vec![(
                "close",
                "episode",
                json!({"action":"close","outcome":"partial","synopsis":"Accepted diagnosis"}),
            )]),
        ]);
        let fixture = fixture_configured(&origin, "disabled", vec![], Some(fault.clone()), None, 1);
        let native = native_episode_runner(&fixture).await;
        fixture
            .host
            .run(input(), &Resolver::new(false), native.clone(), &ignore)
            .await
            .unwrap();
        let runner = Arc::new(FaultedEpisodeJournal {
            registry: native.registry.clone(),
            fault,
            store: fixture.store.clone(),
        });
        let mut close = input();
        close.command = "close-episode".into();
        assert!(
            fixture
                .host
                .run(close, &Resolver::new(false), runner, &ignore)
                .await
                .is_err()
        );
        let resolver = Resolver::new(false);
        let mut next = input();
        next.command = "after-episode-fault".into();
        assert!(
            fixture
                .host
                .run(next, &resolver, native.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
        assert_eq!(model.join().unwrap().len(), 3);
        drop(native);
        let Fixture {
            _directory,
            store,
            host,
            files,
            ..
        } = fixture;
        drop(host);
        drop(files);
        drop(store);
        let reopened = Store::open(&_directory.path().join("store")).unwrap();
        let branch = ag1_core::task::branch_values(
            &reopened
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let closed = ag1_core::context::episode::list_closed(&branch, 100);
        assert_eq!(
            closed.len(),
            usize::from(point == ag1_storage::Boundary::BeforeAck)
        );
        assert_eq!(
            ag1_core::context::episode::find_open(&branch).is_some(),
            point == ag1_storage::Boundary::BeforeCommit
        );
        if let Some(record) = closed.first() {
            let artifacts = ag1_storage::context::ContextStore::new(
                &reopened,
                SessionId::new("session").unwrap(),
                "session",
                "recovered-episode",
                NOW,
                Default::default(),
            )
            .unwrap();
            assert!(
                artifacts
                    .get(record.value()["trajectoryArtifactId"].as_str().unwrap())
                    .is_ok()
            );
            assert_eq!(record.value()["synopsis"], "Accepted diagnosis");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_episode_control_commit_failure_retires_worker_and_retains_drained_output() {
    struct ControlFault {
        point: ag1_storage::Boundary,
        hit: AtomicBool,
    }
    impl ag1_storage::FaultInjector for ControlFault {
        fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            Ok(())
        }
        fn check_journal_command(
            &self,
            command: &str,
            point: ag1_storage::Boundary,
        ) -> ag1_storage::StorageResult<()> {
            if command == "failed-draft-control"
                && point == self.point
                && !self.hit.swap(true, Ordering::SeqCst)
            {
                return Err(ag1_storage::StorageError::Injected(point));
            }
            Ok(())
        }
    }
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let StalledEpisode {
            origin,
            ready,
            release,
            model,
        } = stalled_episode();
        let fault = Arc::new(ControlFault {
            point,
            hit: AtomicBool::new(false),
        });
        let fixture = fixture_configured(&origin, "disabled", vec![], Some(fault.clone()), None, 1);
        let runner = native_episode_runner(&fixture).await;
        let host = fixture.host.clone();
        let run = tokio::spawn(async move {
            host.run(input(), &Resolver::new(false), runner, &ignore)
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), ready)
            .await
            .unwrap()
            .unwrap();
        let mut replacement = fixture.model.clone();
        replacement["id"] = json!("replacement");
        let commit = tokio::time::timeout(
            Duration::from_secs(3),
            fixture.host.select_model(
                "failed-draft-control",
                "failed-draft-control",
                NOW,
                replacement,
                &ignore,
            ),
        )
        .await
        .unwrap();
        assert!(commit.is_err());
        assert!(fault.hit.load(Ordering::SeqCst));
        let output = tokio::time::timeout(Duration::from_secs(3), run)
            .await
            .unwrap()
            .unwrap();
        assert!(output.is_err());
        release.send(()).unwrap();
        assert_eq!(model.join().unwrap().len(), 2);
        assert!(!fixture.root.join("forbidden.txt").exists());
        assert!(fixture.host.pending_transport().unwrap().is_some());
        let snapshots = fixture
            .store
            .artifact_catalog("session", None, 100, Some("episode-draft-output"))
            .unwrap();
        assert_eq!(snapshots.len(), 1, "{point:?}");
        let resolver = Resolver::new(false);
        let mut next = input();
        next.command = "after-control-failure".into();
        assert!(
            fixture
                .host
                .run(next, &resolver, fixture.files.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
        let Fixture {
            _directory,
            host,
            files,
            store,
            ..
        } = fixture;
        drop(host);
        drop(files);
        drop(store);
        let reopened = Store::open(&_directory.path().join("store")).unwrap();
        let branch = ag1_core::task::branch_values(
            &reopened
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            branch.iter().any(|row| row["id"] == "failed-draft-control"),
            point == ag1_storage::Boundary::BeforeAck
        );
        assert!(ag1_core::context::episode::find_open(&branch).is_some());
        assert!(ag1_core::context::episode::list_closed(&branch, 100).is_empty());
    }
}

fn silent_overflow_reply() -> Value {
    json!({"choices":[{"delta":{"content":"silent-overflow-marker"},"finish_reason":"stop"}],"usage":{"prompt_tokens":50000,"completion_tokens":2}})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_usage_overflow_compacts_and_retries_once_without_accepting_repeated_overflow() {
    for repeated in [false, true] {
        let (origin, model) = server(vec![
            silent_overflow_reply(),
            stop(),
            if repeated {
                silent_overflow_reply()
            } else {
                stop()
            },
        ]);
        let fixture = fixture_seeded(&origin, "disabled", summary_history());
        let mut turn = input();
        turn.compaction = compact_input().settings;
        let events = Mutex::new(vec![]);
        let listen = |event: &SessionEvent| {
            if let SessionEvent::Recovery(v) = event {
                events.lock().unwrap().push(v.clone());
            }
        };
        let output = fixture
            .host
            .run(turn, &Resolver::new(false), fixture.files.clone(), &listen)
            .await;
        assert_eq!(output.is_err(), repeated);
        if let Ok(output) = output {
            assert_eq!(output.generations, 2);
        }
        let requests = model.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(!requests[2].to_string().contains("silent-overflow-marker"));
        let journal = fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        let originals = journal
            .entries
            .iter()
            .filter(|entry| String::from_utf8_lossy(&entry.raw).contains("silent-overflow-marker"))
            .count();
        assert_eq!(originals, if repeated { 2 } else { 1 });
        assert_eq!(
            episode_branch(&fixture)
                .iter()
                .filter(|e| e["type"] == "compaction")
                .count(),
            1
        );
        assert!(fixture.host.pending_transport().unwrap().is_none());
        if repeated {
            assert!(events.lock().unwrap().iter().any(|e| {
                e["type"] == "compaction_end"
                    && e["willRetry"] == false
                    && e["errorMessage"]
                        .as_str()
                        .unwrap_or("")
                        .contains("one compact-and-retry")
            }));
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn silent_usage_overflow_preserves_prior_tool_effects_without_reexecution() {
    let (origin, model) = server(vec![
        calls(vec![(
            "once",
            "append",
            json!({"path":"effects.txt","content":"once\n"}),
        )]),
        silent_overflow_reply(),
        stop(),
        stop(),
    ]);
    let fixture = fixture_seeded(&origin, "disabled", summary_history());
    std::fs::write(fixture.root.join("effects.txt"), "").unwrap();
    let mut turn = input();
    turn.compaction = compact_input().settings;
    let result = fixture
        .host
        .run(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 3);
    assert_eq!(model.join().unwrap().len(), 4);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("effects.txt")).unwrap(),
        "once\n"
    );
    let journal = fixture
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let results = journal
        .entries
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.normalized).unwrap())
        .filter(|r| r["message"]["role"] == "toolResult" && r["message"]["toolCallId"] == "once")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 1);
    assert_ne!(results[0]["message"]["isError"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_project_check_uses_native_session_approval_and_durable_result() {
    use ag1_platform::{
        projects::{CheckPort, ProjectPort, Runtime, Toolchains},
        session::adapters::NativeAdapters,
    };
    let (origin, server) = server(vec![
        calls(vec![("compile-project", "check", json!({}))]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(
        fixture.root.join("Cargo.toml"),
        "[package]\nname='session-check-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(
        fixture.root.join("Cargo.lock"),
        "version = 4\n[[package]]\nname = \"session-check-fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir(fixture.root.join("src")).unwrap();
    std::fs::write(
        fixture.root.join("src/main.rs"),
        "fn main() { println!(\"native check\"); }",
    )
    .unwrap();
    let state = fixture._directory.path().join("project-state");
    std::fs::create_dir(&state).unwrap();
    let cargo = std::path::PathBuf::from(env!("CARGO"))
        .canonicalize()
        .unwrap();
    let root = cargo.parent().unwrap().parent().unwrap().to_path_buf();
    let runtime = Runtime::load(
        fixture.workspace.clone(),
        WorkspaceService::new().unwrap().register(&state).unwrap(),
        env!("CARGO_BIN_EXE_ag1-process-host").as_ref(),
        Toolchains {
            node: None,
            npm: None,
            cargo: Some(cargo),
            rustc: Some(root.join("bin/rustc")),
            godot: None,
            mounts: vec![root],
        },
    )
    .unwrap();
    let project = Arc::new(Mutex::new(ProjectPort::Check(Box::new(CheckPort {
        runtime,
        kind: ag1_core::projects::Kind::Rust,
    }))));
    let adapters = NativeAdapters {
        project: Some(project.clone()),
        ..Default::default()
    };
    let mut registry = Registry::builtins().unwrap();
    registry
        .activate(
            &["check".into()],
            &adapters.available_names().into_iter().collect(),
        )
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|t| t["name"] == "check")
        .collect();
    fixture
        .host
        .select_tools("project-target", "project-target", NOW, tools, &ignore)
        .await
        .unwrap();
    let runner = Arc::new(FileRunner {
        files: fixture.files.files.clone(),
        adapters,
        registry: Arc::new(registry),
        policy: fixture.files.policy.clone(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: fixture.store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(60),
    });
    let result = fixture
        .host
        .run(input(), &Resolver::new(false), runner, &ignore)
        .await
        .unwrap();
    assert_eq!(result.generations, 2);
    let requests = server.join().unwrap();
    let tool = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    assert!(
        tool["content"].as_str().unwrap().contains("exited"),
        "{tool}"
    );
    let port = project.lock().unwrap();
    let ProjectPort::Check(port) = &*port else {
        panic!()
    };
    let history = port.runtime.history().unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].exits, vec![Some(0)]);
    let branch = fixture.store.branch("session", None).unwrap();
    assert!(
        branch
            .iter()
            .any(|e| String::from_utf8_lossy(&e.raw).contains("compile-project"))
    );
}
