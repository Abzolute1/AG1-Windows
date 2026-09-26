//! Real P05 transactions and P08 custody/compilation at the native session boundary.
//! Provider dispatch is deliberately observed, not sent to a live endpoint.
#[path = "runtime_checkpoint/mod.rs"]
mod checkpoint;
#[path = "runtime_fork/mod.rs"]
mod runtime_fork;
#[path = "runtime_selection/mod.rs"]
mod selection;
use ag1_core::{
    Clock, DomainError, SessionId,
    context::{
        compiler::{Compilation, CompileOptions},
        features::Features,
    },
    runtime::{
        SessionRuntime,
        session::{Configuration, PreparationAction},
    },
    storage::StoragePort,
};
use ag1_storage::{
    Boundary, FaultInjector, StorageError, StorageResult, Store, context::ContextStore,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
const NOW: &str = "2026-09-06T00:00:00.000Z";
fn model(id: &str, window: u64) -> Value {
    json!({"id":id,"provider":"owned-fixture","api":"openai-completions","contextWindow":window})
}
fn tool(name: &str) -> Value {
    json!({"name":name,"description":format!("Fixture {name}"),"parameters":{"type":"object","properties":{}}})
}
fn config() -> Configuration {
    Configuration {
        model: model("initial", 32768),
        thinking: "medium".into(),
        tools: vec![tool("read")],
        system_prompt: "Owned fixture".into(),
        prompt: None,
        compiler: CompileOptions::default(),
    }
}
fn create(store: &Store, id: &str) {
    store.create_session(id, &serde_json::to_vec(&json!({"type":"session","version":3,"id":id,"cwd":"/owned-fixture","timestamp":NOW})).unwrap()).unwrap();
    store.register_origin(id, id, &[8; 32]).unwrap();
}

#[test]
fn native_command_admission_binds_revision_and_payload_but_replays_after_later_commits() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    create(&store, "other");
    let payload = json!({"id":"client-command","scope":{"workspace":"workspace","session":"session","instance":"instance","epoch":"1"},"command":{"kind":"prompt","input":{"kind":"inline","text":"Inspect"}}});
    let accepted = store
        .record_session_intent("session", "admission", 0, &payload)
        .unwrap();
    assert_eq!(accepted.state, "intent");
    assert!(accepted.result.is_none());
    let wrapper: Value = serde_json::from_slice(&accepted.intent).unwrap();
    assert_eq!(wrapper["payload"], payload);
    assert_eq!(wrapper["expectedRevision"], "0");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "later", "Later instruction");
    assert_eq!(
        store
            .record_session_intent("session", "admission", 0, &payload)
            .unwrap(),
        accepted
    );
    let mut changed = payload.clone();
    changed["extension"] = Value::Null;
    for (session, expected, input) in [
        ("session", 0, &changed),
        ("session", 1, &payload),
        ("other", 0, &payload),
    ] {
        assert!(matches!(
            store.record_session_intent(session, "admission", expected, input),
            Err(StorageError::Conflict(_))
        ));
    }
    let terminal = br#"{"operation":"native-op","status":"completed","evidence":"owned-result"}"#;
    let completed = store
        .settle_intent("admission", &accepted.intent, terminal)
        .unwrap();
    assert_eq!(
        store
            .record_session_intent("session", "admission", 0, &payload)
            .unwrap(),
        completed
    );
    assert_eq!(
        store
            .command_result("admission")
            .unwrap()
            .unwrap()
            .result
            .as_deref(),
        Some(terminal.as_slice())
    );
    assert!(matches!(
        store.settle_intent("admission", &accepted.intent, b"different terminal"),
        Err(StorageError::Conflict(_))
    ));
    assert_eq!(
        store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap()
            .revision,
        1
    );
}

#[test]
fn native_command_admission_faults_reopen_as_uncertain_without_inventing_completion() {
    for point in [
        Boundary::BeforeTransaction,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        create(&store, "session");
        let payload = json!({"id":"request","extension":null,"command":{"kind":"prompt","input":{"kind":"inline","text":"Retain exactly"}}});
        fault.armed.store(true, Ordering::SeqCst);
        assert!(
            store
                .record_session_intent("session", "admission", 0, &payload)
                .is_err()
        );
        let persisted = matches!(point, Boundary::AfterCommit | Boundary::BeforeAck);
        assert_eq!(
            store.command_result("admission").unwrap().is_some(),
            persisted
        );
        assert_eq!(
            store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap()
                .revision,
            0
        );
        drop(store);
        let reopened = Store::open(directory.path()).unwrap();
        let receipt = reopened
            .record_session_intent("session", "admission", 0, &payload)
            .unwrap();
        assert_eq!(
            receipt.state,
            if persisted { "uncertain" } else { "intent" }
        );
        assert!(receipt.result.is_none());
        assert_eq!(
            serde_json::from_slice::<Value>(&receipt.intent).unwrap()["payload"],
            payload
        );
        let snapshot = reopened
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        assert!(
            snapshot.entries.is_empty(),
            "admission is not prompt execution"
        );
        let terminal =
            br#"{"status":"cancelled","reason":"explicit reconciliation before execution"}"#;
        let done = reopened
            .settle_intent("admission", &receipt.intent, terminal)
            .unwrap();
        drop(reopened);
        let reopened = Store::open(directory.path()).unwrap();
        assert_eq!(reopened.command_result("admission").unwrap(), Some(done));
    }
}

#[test]
fn rejected_native_admission_does_not_write_intent_or_advance_sequence() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let session = SessionId::new("session").unwrap();
    let before = store.journal_snapshot(&session).unwrap();
    assert!(matches!(
        store.record_session_intent("session", "stale", 1, &json!({"command":"inspect"})),
        Err(StorageError::RevisionConflict {
            expected: 1,
            actual: 0
        })
    ));
    assert!(
        store
            .record_session_intent("missing", "missing", 0, &json!({}))
            .is_err()
    );
    assert!(
        store
            .record_session_intent(
                "session",
                "oversize",
                0,
                &json!({"text":"x".repeat(64*1024)})
            )
            .is_err()
    );
    assert!(
        store
            .record_session_intent("session", "scalar", 0, &json!(1))
            .is_err()
    );
    for id in ["stale", "missing", "oversize", "scalar"] {
        assert!(store.command_result(id).unwrap().is_none());
    }
    let after = store.journal_snapshot(&session).unwrap();
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.committed_sequence, after.committed_sequence);
    assert_eq!(before.entries, after.entries);
}

#[test]
fn concurrent_native_admissions_grant_one_new_owner_and_lost_acknowledgement_grants_none_on_retry()
{
    let directory = tempfile::tempdir().unwrap();
    let fault = Arc::new(FailOnce {
        point: Boundary::BeforeAck,
        armed: AtomicBool::new(false),
    });
    let store =
        Arc::new(Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap());
    create(&store, "session");
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut workers = vec![];
    for _ in 0..8 {
        let store = store.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            store
                .admit_session_command("session", "once", 0, &json!({"message":"same request"}))
                .unwrap()
        }));
    }
    let admissions = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        admissions
            .iter()
            .filter(|admission| admission.inserted)
            .count(),
        1
    );
    assert!(
        admissions
            .iter()
            .all(|admission| admission.record == admissions[0].record)
    );
    fault.armed.store(true, Ordering::SeqCst);
    assert!(
        store
            .admit_session_command("session", "lost", 0, &json!({"message":"lost reply"}))
            .is_err()
    );
    let retry = store
        .admit_session_command("session", "lost", 0, &json!({"message":"lost reply"}))
        .unwrap();
    assert!(
        !retry.inserted,
        "unknown earlier handoff must not grant another execution owner"
    );
    assert_eq!(retry.record.state, "intent");
    assert!(retry.record.result.is_none());
}
fn resume(
    store: &Store,
    id: &str,
    configuration: Configuration,
) -> Result<SessionRuntime, DomainError> {
    let session = SessionId::new(id).unwrap();
    SessionRuntime::resume(
        session.clone(),
        1,
        &store.journal_snapshot(&session)?,
        configuration,
    )
}
fn compile(store: &Store, preparation: &PreparationAction, operation: &str) -> Compilation {
    let session = preparation.stamp().session().clone();
    let mut context = ContextStore::new(
        store,
        session.clone(),
        session.as_str(),
        operation,
        NOW,
        Features::default(),
    )
    .unwrap();
    context
        .compile(
            &preparation.messages().clone().into_values(),
            &preparation.compile_options(),
            preparation.journal_revision(),
        )
        .unwrap()
        .projection
}
fn append_user(runtime: &mut SessionRuntime, store: &Store, id: &str, text: &str) {
    let action = runtime
        .append_message(
            id,
            id,
            NOW,
            json!({"role":"user","content":text,"timestamp":0}),
        )
        .unwrap();
    runtime.acknowledge(action.execute(store)).unwrap();
}

fn append_failure(runtime: &mut SessionRuntime, store: &Store) {
    let action=runtime.append_message("failed","failed",NOW,json!({"role":"assistant","content":[],"stopReason":"error","errorMessage":"server overloaded"})).unwrap();
    runtime.acknowledge(action.execute(store)).unwrap();
}

#[test]
fn retry_branch_ack_is_durable_idempotent_and_retains_the_abandoned_attempt() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "prompt", "inspect");
    append_failure(&mut runtime, &store);
    let stamp = runtime.stamp().clone();
    let action = runtime
        .branch_before_failure("retry", "failed", &stamp)
        .unwrap();
    assert!(runtime.prepare_request().is_err());
    let receipt = action.execute(&store);
    assert_eq!(runtime.branch().len(), 2);
    runtime.acknowledge(receipt).unwrap();
    assert_eq!(runtime.branch().len(), 1);
    let replay = store
        .select_leaf("session", "retry", 2, Some("prompt"))
        .unwrap();
    assert_eq!(replay.revision, 3);
    assert!(
        store
            .select_leaf("session", "retry", 2, Some("failed"))
            .is_err()
    );
    assert!(
        store
            .select_leaf("session", "stale", 2, Some("failed"))
            .is_err()
    );
    append_user(&mut runtime, &store, "continued", "continue");
    drop(store);
    let reopened = Store::open(directory.path()).unwrap();
    let journal = reopened
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(journal.entries.len(), 3);
    assert!(journal.entries.iter().any(|entry| entry.id == "failed"));
    let restored = resume(&reopened, "session", config()).unwrap();
    assert_eq!(restored.branch().len(), 2);
    reopened.fork("session", "forked", None).unwrap();
    let child = resume(&reopened, "forked", config()).unwrap();
    assert_eq!(child.branch().len(), 2);
    assert!(!child.branch().iter().any(|entry| entry["id"] == "failed"));
    assert!(
        !restored
            .branch()
            .iter()
            .any(|entry| entry["id"] == "failed")
    );
    assert!(
        reopened
            .select_leaf("session", "missing", 4, Some("missing"))
            .is_err()
    );
}

#[test]
fn lost_retry_branch_ack_blocks_replay_and_reopen_uses_the_committed_leaf() {
    for point in [
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        create(&store, "session");
        let mut runtime = resume(&store, "session", config()).unwrap();
        append_user(&mut runtime, &store, "prompt", "inspect");
        append_failure(&mut runtime, &store);
        let stamp = runtime.stamp().clone();
        let action = runtime
            .branch_before_failure("retry", "failed", &stamp)
            .unwrap();
        fault.armed.store(true, Ordering::SeqCst);
        assert!(runtime.acknowledge(action.execute(&store)).is_err());
        assert!(matches!(
            runtime.prepare_request(),
            Err(DomainError::DurabilityUnknown)
        ));
        drop(store);
        let reopened = Store::open(directory.path()).unwrap();
        let recovered = resume(&reopened, "session", config()).unwrap();
        assert_eq!(
            recovered.branch().len(),
            if point == Boundary::BeforeCommit {
                2
            } else {
                1
            }
        );
        assert_eq!(
            reopened
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap()
                .entries
                .len(),
            2
        );
    }
}

#[test]
fn retry_rollback_checkpoint_cannot_overwrite_new_work_or_a_replacement_owner() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "prompt", "inspect");
    append_failure(&mut runtime, &store);
    let before = runtime.stamp().clone();
    let action = runtime
        .branch_before_failure("retry", "failed", &before)
        .unwrap();
    let checkpoint = action.checkpoint();
    runtime.acknowledge(action.execute(&store)).unwrap();
    let selected = runtime.stamp().clone();
    let mut replacement = resume(&store, "session", config()).unwrap();
    let foreign_stamp = replacement.stamp().clone();
    assert!(matches!(
        replacement.restore_branch("foreign", &checkpoint, &foreign_stamp),
        Err(DomainError::Conflict)
    ));
    append_user(&mut runtime, &store, "new", "new work wins");
    assert!(matches!(
        runtime.restore_branch("restore", &checkpoint, &selected),
        Err(DomainError::Conflict)
    ));
    assert_eq!(runtime.branch().last().unwrap()["id"], "new");
}

#[test]
fn paused_commit_is_invisible_until_ack_and_blocks_every_request() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    let action = runtime
        .append_message(
            "prompt",
            "prompt",
            NOW,
            json!({"role":"user","content":"inspect"}),
        )
        .unwrap();
    assert_eq!(action.write().expected, 0);
    assert!(runtime.branch().is_empty());
    assert!(!runtime.settled());
    assert!(runtime.prepare_request().is_err());
    let receipt = action.execute(&store);
    // SQL commit returning to its worker is still not acknowledgement by the
    // mutation owner. Public progress cannot admit a provider request either.
    assert_eq!(
        store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap()
            .revision,
        1
    );
    assert_eq!(runtime.journal_revision(), 0);
    assert!(runtime.prepare_request().is_err());
    runtime.acknowledge(receipt).unwrap();
    assert_eq!(runtime.journal_revision(), 1);
    assert_eq!(
        runtime.messages().values().next().unwrap()["content"],
        "inspect"
    );
    let preparation = runtime.prepare_request().unwrap();
    let projection = compile(&store, &preparation, "first-request");
    let generation = runtime.admit_request(preparation, Ok(projection)).unwrap();
    assert!(!runtime.settled());
    runtime.generation_terminal(&generation).unwrap();
    assert!(runtime.settled());
    assert_eq!(
        runtime.generation_terminal(&generation),
        Err(DomainError::Conflict)
    );
}

#[test]
fn model_tools_and_thinking_changes_rebuild_a_prepared_request_from_durable_state() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "prompt", "inspect");
    for (index, field) in ["model", "tools", "thinking"].iter().enumerate() {
        let prepared = runtime.prepare_request().unwrap();
        let projected = compile(&store, &prepared, &format!("before-{index}"));
        let id = format!("change-{index}");
        let write = match *field {
            "model" => runtime.select_model(&id, &id, NOW, model("replacement", 65536)),
            "tools" => runtime.select_tools(&id, &id, NOW, vec![tool("read"), tool("check")]),
            _ => runtime.select_thinking(&id, &id, NOW, "high".into()),
        }
        .unwrap();
        runtime.acknowledge(write.execute(&store)).unwrap();
        assert!(
            matches!(
                runtime.admit_request(prepared, Ok(projected)),
                Err(DomainError::Conflict)
            ),
            "{field}"
        );
        let rebuilt = runtime.prepare_request().unwrap();
        assert_eq!(rebuilt.journal_revision(), runtime.journal_revision());
        assert_eq!(rebuilt.branch().last().unwrap()["id"], id);
        let projected = compile(&store, &rebuilt, &format!("after-{index}"));
        let request = runtime.admit_request(rebuilt, Ok(projected)).unwrap();
        assert_eq!(request.configuration().model["id"], "replacement");
        if index >= 1 {
            assert_eq!(request.configuration().tools.len(), 2);
        }
        if index == 2 {
            assert_eq!(request.configuration().thinking, "high");
        }
        runtime.generation_terminal(&request).unwrap();
    }
    assert!(resume(&store, "session", config()).is_err());
    let mut resolved = config();
    resolved.model = model("replacement", 65536);
    resolved.tools.push(tool("check"));
    let resumed = resume(&store, "session", resolved).unwrap();
    assert_eq!(resumed.configuration().thinking, "high");
    assert_eq!(resumed.journal_revision(), 4);
}

#[test]
fn cancellation_during_preparation_and_generation_still_drains_accepted_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "prompt", "inspect");
    let prepared = runtime.prepare_request().unwrap();
    let projected = compile(&store, &prepared, "cancelled-prepare");
    runtime.cancel();
    assert!(runtime.admit_request(prepared, Ok(projected)).is_err());
    runtime.next_turn().unwrap();
    let prepared = runtime.prepare_request().unwrap();
    let projected = compile(&store, &prepared, "active-generation");
    let request = runtime.admit_request(prepared, Ok(projected)).unwrap();
    let write = runtime.append_message("partial","partial",NOW,json!({"role":"assistant","content":[{"type":"text","text":"partial"}],"stopReason":"aborted"})).unwrap();
    runtime.cancel();
    assert!(runtime.next_turn().is_err());
    runtime.generation_terminal(&request).unwrap();
    assert!(!runtime.settled());
    runtime.acknowledge(write.execute(&store)).unwrap();
    assert!(runtime.settled());
    assert!(matches!(
        runtime.prepare_request(),
        Err(DomainError::Cancelled)
    ));
    runtime.next_turn().unwrap();
    assert_eq!(
        runtime.branch().last().unwrap()["message"]["stopReason"],
        "aborted"
    );
}

struct FixedClock;
impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        0
    }
}
struct FailOnce {
    point: Boundary,
    armed: AtomicBool,
}
impl FaultInjector for FailOnce {
    fn check(&self, point: Boundary) -> StorageResult<()> {
        if point == self.point && self.armed.swap(false, Ordering::SeqCst) {
            Err(StorageError::Injected(point))
        } else {
            Ok(())
        }
    }
}
#[test]
fn failed_or_lost_append_ack_blocks_provider_until_explicit_reload() {
    for point in [
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        create(&store, "session");
        let mut runtime = resume(&store, "session", config()).unwrap();
        let write = runtime
            .append_message(
                "prompt",
                "prompt",
                NOW,
                json!({"role":"user","content":"must survive"}),
            )
            .unwrap();
        fault.armed.store(true, Ordering::SeqCst);
        assert!(runtime.acknowledge(write.execute(&store)).is_err());
        assert_eq!(runtime.journal_revision(), 0);
        assert!(matches!(
            runtime.prepare_request(),
            Err(DomainError::DurabilityUnknown)
        ));
        assert_eq!(runtime.next_turn(), Err(DomainError::DurabilityUnknown));
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        let mut recovered = resume(&store, "session", config()).unwrap();
        let committed = point != Boundary::BeforeCommit;
        assert_eq!(recovered.branch().len(), usize::from(committed));
        assert!(recovered.prepare_request().is_ok());
    }
}

#[test]
fn stale_same_session_receipts_and_requests_cannot_mutate_replacement_owner() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut old = resume(&store, "session", config()).unwrap();
    let prepared = old.prepare_request().unwrap();
    let projected = compile(&store, &prepared, "old-request");
    let receipt = old
        .append_message("old", "old", NOW, json!({"role":"user","content":"old"}))
        .unwrap()
        .execute(&store);
    let mut replacement = resume(&store, "session", config()).unwrap();
    let current = replacement.prepare_request().unwrap();
    let current_projection = compile(&store, &current, "new-request");
    assert_eq!(replacement.acknowledge(receipt), Err(DomainError::Conflict));
    assert!(matches!(
        replacement.admit_request(prepared, Ok(projected)),
        Err(DomainError::Conflict)
    ));
    let request = replacement
        .admit_request(current, Ok(current_projection))
        .unwrap();
    replacement.generation_terminal(&request).unwrap();
    assert_eq!(replacement.branch().len(), 1);
}

#[test]
fn admission_recounts_final_projection_and_failed_preparation_releases_its_slot() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut configuration = config();
    configuration.model = model("small", 100);
    configuration.system_prompt.clear();
    configuration.tools.clear();
    let mut runtime = resume(&store, "session", configuration).unwrap();
    let prepare = runtime.prepare_request().unwrap();
    assert!(matches!(
        runtime.admit_request(
            prepare,
            Err(DomainError::Unavailable("credential lookup failed".into()))
        ),
        Err(DomainError::Unavailable(_))
    ));
    let prepare = runtime.prepare_request().unwrap();
    // Deliberately false stats cannot bypass the estimate of the final messages.
    let projection = Compilation {
        messages: vec![json!({"role":"user","content":"x".repeat(273)})],
        stats: json!({"tokensAfter":0}),
    };
    assert!(matches!(
        runtime.admit_request(prepare, Ok(projection)),
        Err(DomainError::LimitExceeded(_))
    ));
    let prepare = runtime.prepare_request().unwrap();
    let projection = Compilation {
        messages: vec![json!({"role":"user","content":"x".repeat(270)})],
        stats: json!({}),
    };
    let request = runtime.admit_request(prepare, Ok(projection)).unwrap();
    assert_eq!(request.estimated_input_tokens(), 90);
    runtime.generation_terminal(&request).unwrap();
}

fn tool_assistant() -> Value {
    json!({"role":"assistant","stopReason":"toolUse","content":[
        {"type":"toolCall","id":"call-one","name":"read","arguments":{"path":"a"}},
        {"type":"toolCall","id":"call-two","name":"read","arguments":{"path":"b"}}
    ]})
}
fn tool_result(id: &str) -> Value {
    json!({"role":"toolResult","toolCallId":id,"toolName":"read","content":[{"type":"text","text":"fixture result"}],"isError":false})
}

#[test]
fn every_tool_result_commits_in_source_order_before_the_next_request() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "prompt", "inspect both files");
    let preparation = runtime.prepare_request().unwrap();
    let projection = compile(&store, &preparation, "tool-request");
    let generation = runtime.admit_request(preparation, Ok(projection)).unwrap();
    let assistant = runtime
        .append_message("assistant", "assistant", NOW, tool_assistant())
        .unwrap();
    // Transport completion precedes acknowledgement of the assistant record.
    runtime.generation_terminal(&generation).unwrap();
    assert!(runtime.pending_tools().is_empty());
    assert!(runtime.prepare_request().is_err());
    runtime.acknowledge(assistant.execute(&store)).unwrap();
    assert_eq!(runtime.pending_tools().len(), 2);
    assert!(!runtime.settled());
    assert!(
        runtime
            .append_message(
                "duplicate-assistant",
                "duplicate-assistant",
                NOW,
                tool_assistant()
            )
            .is_err()
    );
    // A concurrent worker may finish the second result first; the host retains
    // it until the first result has committed. No journal write is issued here.
    assert!(matches!(
        runtime.append_message("second", "second", NOW, tool_result("call-two")),
        Err(DomainError::Conflict)
    ));
    let first = runtime
        .append_message("first", "first", NOW, tool_result("call-one"))
        .unwrap();
    let first_receipt = first.execute(&store);
    assert_eq!(runtime.pending_tools().len(), 2);
    assert!(runtime.prepare_request().is_err());
    runtime.acknowledge(first_receipt).unwrap();
    assert_eq!(runtime.pending_tools().front().unwrap().id, "call-two");
    assert!(matches!(
        runtime.append_message("duplicate", "duplicate", NOW, tool_result("call-one")),
        Err(DomainError::Conflict)
    ));
    assert!(runtime.prepare_request().is_err());
    let second = runtime
        .append_message("second", "second", NOW, tool_result("call-two"))
        .unwrap();
    runtime.acknowledge(second.execute(&store)).unwrap();
    assert!(runtime.settled());
    let preparation = runtime.prepare_request().unwrap();
    let projection = compile(&store, &preparation, "after-tools");
    let generation = runtime.admit_request(preparation, Ok(projection)).unwrap();
    let outcomes: Vec<_> = generation
        .projection()
        .messages
        .iter()
        .filter(|m| m["role"] == "toolResult")
        .map(|m| m["toolCallId"].as_str().unwrap())
        .collect();
    assert_eq!(outcomes, ["call-one", "call-two"]);
    runtime.generation_terminal(&generation).unwrap();
}

#[test]
fn resumed_unresolved_calls_block_replay_and_cancel_requires_durable_outcomes() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    let assistant = runtime
        .append_message("assistant", "assistant", NOW, tool_assistant())
        .unwrap();
    runtime.acknowledge(assistant.execute(&store)).unwrap();
    drop(runtime);
    let mut runtime = resume(&store, "session", config()).unwrap();
    assert_eq!(runtime.pending_tools().len(), 2);
    assert!(runtime.prepare_request().is_err());
    runtime.cancel();
    assert_eq!(runtime.next_turn(), Err(DomainError::Conflict));
    for (index, id) in ["call-one", "call-two"].iter().enumerate() {
        // Host reconciliation, rather than replay, supplies each outcome.
        let mut result = tool_result(id);
        result["isError"] = json!(true);
        result["content"] =
            json!([{"type":"text","text":"Cancelled; external effect requires inspection"}]);
        let entry = format!("result-{index}");
        let append = runtime.append_message(&entry, &entry, NOW, result).unwrap();
        runtime.acknowledge(append.execute(&store)).unwrap();
    }
    assert!(runtime.settled());
    runtime.next_turn().unwrap();
    let preparation = runtime.prepare_request().unwrap();
    let projection = compile(&store, &preparation, "reconciled-request");
    let generation = runtime.admit_request(preparation, Ok(projection)).unwrap();
    runtime.generation_terminal(&generation).unwrap();
    let resumed = resume(&store, "session", config()).unwrap();
    assert!(resumed.pending_tools().is_empty());
}

fn compact_result(
    store: &Store,
    capture: &ag1_core::runtime::session::CompactionCapture,
    operation: &str,
    first: &str,
) -> Value {
    let messages = ag1_core::context::compaction::context_messages(capture.branch());
    let mut context = ContextStore::new(
        store,
        SessionId::new("session").unwrap(),
        "session",
        operation,
        NOW,
        Features::default(),
    )
    .unwrap();
    let custody = context
        .compile(
            &messages,
            &CompileOptions {
                context_window: 32768,
                ..Default::default()
            },
            capture.journal_revision(),
        )
        .unwrap();
    json!({"summary":"Earlier work retained in the original trajectory.","firstKeptEntryId":first,"tokensBefore":1000,"details":{"originalJournal":custody.original_journal,"originalRequest":custody.original_request}})
}

#[test]
fn compaction_commit_preserves_originals_and_a_new_suffix_once_then_restores_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "old", "original instruction");
    append_user(&mut runtime, &store, "kept", "current instruction");
    let token = runtime.reserve_compaction().unwrap();
    assert!(runtime.reserve_compaction().is_err());
    assert!(runtime.prepare_request().is_err());
    let capture = runtime.capture_compaction(&token).unwrap();
    let result = compact_result(&store, &capture, "summary-custody", "kept");
    let original = result["details"]["originalJournal"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    append_user(&mut runtime, &store, "suffix", "arrived during summary");
    let append = runtime
        .append_compaction(&capture, "compact", "compact", NOW, result)
        .unwrap();
    assert_eq!(runtime.stamp().epoch(), 1);
    let receipt = append.execute(&store);
    assert_eq!(runtime.stamp().epoch(), 1);
    assert!(!runtime.settled());
    runtime.acknowledge(receipt).unwrap();
    assert_eq!(runtime.stamp().epoch(), 2);
    assert!(runtime.settled());
    let messages = runtime.messages().clone().into_values();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["role"], "compactionSummary");
    assert_eq!(messages[1]["content"], "current instruction");
    assert_eq!(messages[2]["content"], "arrived during summary");
    assert_eq!(runtime.branch().len(), 4);
    assert_eq!(
        runtime.branch()[0]["message"]["content"],
        "original instruction"
    );
    let bytes = store
        .page("session", "session", &original, 0, 32768)
        .unwrap()
        .bytes;
    let saved: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved["entries"].as_array().unwrap().len(), 2);
    let mut restored = resume(&store, "session", config()).unwrap();
    assert_eq!(restored.stamp().epoch(), 2);
    assert_eq!(restored.messages().clone().into_values(), messages);
    let preparation = restored.prepare_request().unwrap();
    let projection = compile(&store, &preparation, "after-compaction");
    let generation = restored.admit_request(preparation, Ok(projection)).unwrap();
    assert_eq!(generation.projection().messages, messages);
    restored.generation_terminal(&generation).unwrap();
}

#[test]
fn compaction_waits_for_tool_results_and_rejects_split_pairs_or_stale_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let mut runtime = resume(&store, "session", config()).unwrap();
    append_user(&mut runtime, &store, "prompt", "inspect");
    let append = runtime
        .append_message("assistant", "assistant", NOW, tool_assistant())
        .unwrap();
    runtime.acknowledge(append.execute(&store)).unwrap();
    let token = runtime.reserve_compaction().unwrap();
    assert!(runtime.capture_compaction(&token).is_err());
    for (index, id) in ["call-one", "call-two"].iter().enumerate() {
        let entry = format!("result-{index}");
        let append = runtime
            .append_message(&entry, &entry, NOW, tool_result(id))
            .unwrap();
        runtime.acknowledge(append.execute(&store)).unwrap();
    }
    let capture = runtime.capture_compaction(&token).unwrap();
    let split = compact_result(&store, &capture, "split-custody", "result-0");
    assert!(matches!(
        runtime.append_compaction(&capture, "split", "split", NOW, split),
        Err(DomainError::InvalidInput(_))
    ));
    let result = compact_result(&store, &capture, "stale-custody", "assistant");
    let change = runtime
        .select_model("model", "model", NOW, model("changed", 32768))
        .unwrap();
    runtime.acknowledge(change.execute(&store)).unwrap();
    assert!(matches!(
        runtime.append_compaction(&capture, "stale", "stale", NOW, result.clone()),
        Err(DomainError::Conflict)
    ));
    runtime.abandon_compaction(&token).unwrap();
    assert_eq!(runtime.stamp().epoch(), 1);
    let next = runtime.reserve_compaction().unwrap();
    assert!(
        runtime
            .append_compaction(&capture, "obsolete", "obsolete", NOW, result)
            .is_err()
    );
    runtime.abandon_compaction(&next).unwrap();
    assert!(runtime.settled());
}

#[test]
fn cancellation_before_summary_commit_aborts_but_does_not_erase_a_commit_that_won_the_race() {
    for committed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        create(&store, "session");
        let mut runtime = resume(&store, "session", config()).unwrap();
        append_user(&mut runtime, &store, "prompt", "inspect");
        let token = runtime.reserve_compaction().unwrap();
        let capture = runtime.capture_compaction(&token).unwrap();
        let result = compact_result(&store, &capture, "custody", "prompt");
        if committed {
            let receipt = runtime
                .append_compaction(&capture, "compact", "compact", NOW, result)
                .unwrap()
                .execute(&store);
            runtime.cancel();
            assert!(!runtime.settled());
            runtime.acknowledge(receipt).unwrap();
        } else {
            runtime.cancel();
            assert!(
                runtime
                    .append_compaction(&capture, "compact", "compact", NOW, result)
                    .is_err()
            );
            runtime.abandon_compaction(&token).unwrap();
        }
        assert_eq!(runtime.stamp().epoch(), if committed { 2 } else { 1 });
        assert!(runtime.settled());
        assert!(matches!(
            runtime.prepare_request(),
            Err(DomainError::Cancelled)
        ));
        assert_eq!(
            resume(&store, "session", config()).unwrap().stamp().epoch(),
            if committed { 2 } else { 1 }
        );
    }
}

#[test]
fn compaction_lost_ack_requires_reload_and_recovers_the_durable_context_epoch() {
    for point in [
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        create(&store, "session");
        let mut runtime = resume(&store, "session", config()).unwrap();
        append_user(&mut runtime, &store, "prompt", "inspect");
        let token = runtime.reserve_compaction().unwrap();
        let capture = runtime.capture_compaction(&token).unwrap();
        let result = compact_result(&store, &capture, "custody", "prompt");
        let append = runtime
            .append_compaction(&capture, "compact", "compact", NOW, result)
            .unwrap();
        fault.armed.store(true, Ordering::SeqCst);
        assert!(runtime.acknowledge(append.execute(&store)).is_err());
        assert_eq!(runtime.stamp().epoch(), 1);
        assert!(matches!(
            runtime.prepare_request(),
            Err(DomainError::DurabilityUnknown)
        ));
        drop(store);
        let reopened = Store::open(directory.path()).unwrap();
        let restored = resume(&reopened, "session", config()).unwrap();
        assert_eq!(
            restored.stamp().epoch(),
            if point == Boundary::BeforeCommit {
                1
            } else {
                2
            }
        );
    }
}

#[test]
fn inspection_indexes_require_acknowledged_success_with_matching_call_and_valid_scope() {
    for case in ["valid", "error", "mismatched", "invalid"] {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        create(&store, "session");
        let mut runtime = resume(&store, "session", config()).unwrap();
        let assistant = runtime.append_message("assistant", "assistant", NOW,
            json!({"role":"assistant","content":[{"type":"toolCall","id":"read-one","name":"read","arguments":{"path":"a.ts"}}],"stopReason":"toolUse"})).unwrap();
        runtime.acknowledge(assistant.execute(&store)).unwrap();
        let mut evidence = json!({"version":1,"toolCallId":"read-one","source":format!("file:a.ts@sha256:{}","a".repeat(64)),"kind":"read_range","scope":{"startLine":1,"endLine":3}});
        if case == "mismatched" {
            evidence["toolCallId"] = json!("another-call");
        }
        if case == "invalid" {
            evidence["scope"]["endLine"] = json!(0);
        }
        let result = runtime.append_message("result", "result", NOW,
            json!({"role":"toolResult","toolCallId":"read-one","toolName":"read","content":[{"type":"text","text":"actual tool output"}],"isError":case=="error","details":{"inspectionEvidence":evidence}})).unwrap();
        let receipt = result.execute(&store);
        assert!(matches!(
            runtime.capture_inspection("inspection", "inspection", NOW, "result"),
            Err(DomainError::Conflict)
        ));
        runtime.acknowledge(receipt).unwrap();
        let action = runtime
            .capture_inspection("inspection", "inspection", NOW, "result")
            .unwrap();
        assert_eq!(action.is_some(), case == "valid");
        if let Some(action) = action {
            let receipt = action.execute(&store);
            assert!(ag1_core::evidence::inspection_history(runtime.branch()).is_empty());
            assert!(runtime.prepare_request().is_err());
            runtime.acknowledge(receipt).unwrap();
            assert_eq!(
                ag1_core::evidence::inspection_history(runtime.branch()).len(),
                1
            );
        }
        let restored = resume(&store, "session", config()).unwrap();
        assert_eq!(
            ag1_core::evidence::inspection_history(restored.branch()).len(),
            usize::from(case == "valid")
        );
        assert!(
            restored
                .branch()
                .iter()
                .any(|entry| entry["id"] == "result")
        );
    }
}

#[test]
fn resource_prompt_publication_is_acknowledged_and_stale_definitions_do_not_replace_it() {
    use ag1_core::{
        context::prompt::PromptInput, runtime::prompt::PromptConfiguration,
        tools::registry::Registry,
    };
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "session");
    let registry = Registry::builtins().unwrap();
    let mut configuration = config();
    configuration.tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| ["read", "write"].iter().any(|name| tool["name"] == *name))
        .collect();
    let mut runtime = resume(&store, "session", configuration).unwrap();
    append_user(&mut runtime, &store, "prompt", "continue");
    let prepared = runtime.prepare_request().unwrap();
    let projected = compile(&store, &prepared, "before-resources");
    let input = PromptInput {
        date: "2026-09-06".into(),
        cwd: "/owned-fixture".into(),
        custom_prompt: Some("Captured resource instructions".into()),
        ..Default::default()
    };
    let resources =
        PromptConfiguration::new(input.clone(), registry.clone(), "a".repeat(64)).unwrap();
    let action = runtime
        .install_prompt("resources", "resources", NOW, resources)
        .unwrap();
    let receipt = action.execute(&store);
    assert_eq!(runtime.configuration().system_prompt, "Owned fixture");
    runtime.acknowledge(receipt).unwrap();
    assert!(matches!(
        runtime.admit_request(prepared, Ok(projected)),
        Err(DomainError::Conflict)
    ));
    assert!(
        runtime
            .configuration()
            .system_prompt
            .contains("Captured resource instructions")
    );
    assert!(runtime.configuration().system_prompt.contains("write"));
    let before = runtime.stamp().clone();
    let mut mismatched = runtime.configuration().tools.clone();
    mismatched[0]["description"] = json!("Unregistered replacement");
    assert!(matches!(
        runtime.select_tools("mismatch", "mismatch", NOW, mismatched),
        Err(DomainError::Conflict)
    ));
    assert_eq!(*runtime.stamp(), before);
    let readonly = runtime
        .configuration()
        .tools
        .iter()
        .filter(|tool| tool["name"] == "read")
        .cloned()
        .collect();
    let action = runtime
        .select_tools("readonly", "readonly", NOW, readonly)
        .unwrap();
    runtime.acknowledge(action.execute(&store)).unwrap();
    assert!(
        !runtime
            .configuration()
            .system_prompt
            .contains("- change files:")
    );
    let mut local = runtime.configuration().model.clone();
    local["provider"] = json!("llamacpp");
    let action = runtime.select_model("local", "local", NOW, local).unwrap();
    runtime.acknowledge(action.execute(&store)).unwrap();
    let expected = runtime.configuration().system_prompt.clone();
    let mut configuration = runtime.configuration().clone();
    configuration.system_prompt = "stale host cache".into();
    let restored = resume(&store, "session", configuration).unwrap();
    assert_eq!(restored.configuration().system_prompt, expected);
    let invalid = PromptConfiguration::new(
        PromptInput {
            date: "invalid".into(),
            ..input
        },
        registry,
        "b".repeat(64),
    )
    .unwrap();
    let before = runtime.stamp().clone();
    assert!(
        runtime
            .install_prompt("bad-resource", "bad-resource", NOW, invalid)
            .is_err()
    );
    assert_eq!(*runtime.stamp(), before);
    assert_eq!(runtime.configuration().system_prompt, expected);
}

#[test]
fn silent_overflow_branch_requires_an_over_limit_terminal_leaf_and_preserves_all_originals() {
    for kind in [
        "overflow",
        "normal",
        "exact-limit",
        "later-task",
        "executed-tool",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        create(&store, "session");
        let mut runtime = resume(&store, "session", config()).unwrap();
        append_user(&mut runtime, &store, "prompt", "inspect");
        let content = if kind == "executed-tool" {
            json!([{"type":"toolCall","id":"read-call","name":"read","arguments":{}}])
        } else {
            json!([{"type":"text","text":"reported success"}])
        };
        let usage = match kind {
            "normal" => 100,
            "exact-limit" => 32768,
            _ => 32769,
        };
        let action=runtime.append_message("response","response",NOW,json!({"role":"assistant","content":content,"stopReason":"stop","usage":{"input":usage}})).unwrap();
        runtime.acknowledge(action.execute(&store)).unwrap();
        if kind == "later-task" {
            let action = runtime
                .append_task_state(
                    "new-task",
                    "new-task",
                    NOW,
                    ag1_core::task::TaskState::reset("Changed work", NOW).unwrap(),
                )
                .unwrap();
            runtime.acknowledge(action.execute(&store)).unwrap();
        }
        if kind == "executed-tool" {
            let action=runtime.append_message("read-result","read-result",NOW,json!({"role":"toolResult","toolCallId":"read-call","toolName":"read","content":[{"type":"text","text":"read evidence"}],"isError":false})).unwrap();
            runtime.acknowledge(action.execute(&store)).unwrap();
        }
        let before = store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        let stamp = runtime.stamp().clone();
        let action = runtime.branch_before_overflow("overflow", "response", &stamp);
        if kind == "overflow" {
            runtime
                .acknowledge(action.unwrap().execute(&store))
                .unwrap();
            let after = store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap();
            assert_eq!(after.leaf, Some("prompt".into()));
            assert_eq!(after.entries.len(), before.entries.len());
            assert!(after.entries.iter().any(|e| e.id == "response"));
        } else {
            assert!(matches!(action, Err(DomainError::Conflict)), "{kind}");
            assert_eq!(
                store
                    .journal_snapshot(&SessionId::new("session").unwrap())
                    .unwrap()
                    .leaf,
                before.leaf
            );
        }
    }
}

fn bootstrap_header(session: &str) -> Vec<u8> {
    format!("{{ \"type\":\"session\",\"version\":3,\"id\":\"{session}\",\"cwd\":\"/owned-fixture\",\"timestamp\":\"{NOW}\",\"opaque\":{{\"keep\":true}} }}").into_bytes()
}
#[test]
fn session_bootstrap_commits_header_origin_and_receipt_together_and_replays_without_key_rotation() {
    use ag1_core::context::ContextArtifactPort;
    use ag1_storage::{SessionBootstrap, SessionSeed};
    for point in [
        Boundary::BeforeTransaction,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        let header = bootstrap_header("new-session");
        let records = vec![
            serde_json::to_vec(&json!({"type":"model_change","id":"initial-model","parentId":null,"timestamp":NOW,"provider":"owned-fixture","modelId":"initial"})).unwrap(),
            serde_json::to_vec(&json!({"type":"thinking_level_change","id":"initial-thinking","parentId":"initial-model","timestamp":NOW,"thinkingLevel":"medium"})).unwrap(),
        ];
        fault.armed.store(true, Ordering::Release);
        assert!(
            store
                .bootstrap_session(SessionBootstrap {
                    command: "create-command",
                    session: "new-session",
                    origin: "new-origin",
                    cursor_key: &[31; 32],
                    seed: SessionSeed::Fresh {
                        header: &header,
                        records: &records
                    }
                })
                .is_err()
        );
        let committed = matches!(point, Boundary::AfterCommit | Boundary::BeforeAck);
        assert_eq!(store.snapshot("new-session").is_ok(), committed);
        assert_eq!(
            ContextStore::new(
                &store,
                SessionId::new("new-session").unwrap(),
                "new-origin",
                "inspect-origin",
                NOW,
                Features::default()
            )
            .is_ok(),
            committed
        );
        assert_eq!(
            store.command_result("create-command").unwrap().is_some(),
            committed
        );
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        let ack = store
            .bootstrap_session(SessionBootstrap {
                command: "create-command",
                session: "new-session",
                origin: "new-origin",
                cursor_key: &[32; 32],
                seed: SessionSeed::Fresh {
                    header: &header,
                    records: &records,
                },
            })
            .unwrap();
        assert_eq!(ack.revision, 2);
        let restored = resume(&store, "new-session", config()).unwrap();
        assert_eq!(restored.configuration().model["id"], "initial");
        assert_eq!(restored.configuration().thinking, "medium");
        assert_eq!(
            store
                .snapshot("new-session")
                .unwrap()
                .entries
                .iter()
                .map(|e| e.raw.clone())
                .collect::<Vec<_>>(),
            records
        );
        assert_eq!(store.snapshot("new-session").unwrap().header, header);
        let mut artifacts = ContextStore::new(
            &store,
            SessionId::new("new-session").unwrap(),
            "new-origin",
            "own-evidence",
            NOW,
            Features::default(),
        )
        .unwrap();
        let evidence = artifacts
            .put_text("surviving evidence", "text", "read", "read-call", &[])
            .unwrap();
        let cursor = store
            .cursor("new-session", "new-origin", &evidence.id, 0, 32768)
            .unwrap();
        let replay = store
            .bootstrap_session(SessionBootstrap {
                command: "create-command",
                session: "new-session",
                origin: "new-origin",
                cursor_key: &[33; 32],
                seed: SessionSeed::Fresh {
                    header: &header,
                    records: &records,
                },
            })
            .unwrap();
        assert_eq!(replay, ack);
        assert_eq!(
            store
                .cursor("new-session", "new-origin", &evidence.id, 0, 32768)
                .unwrap(),
            cursor
        );
        assert_eq!(
            store.read_cursor("new-session", &cursor).unwrap().bytes,
            b"surviving evidence"
        );
        let conflict = store.bootstrap_session(SessionBootstrap {
            command: "create-command",
            session: "other",
            origin: "new-origin",
            cursor_key: &[33; 32],
            seed: SessionSeed::Fresh {
                header: &bootstrap_header("other"),
                records: &[],
            },
        });
        assert!(matches!(conflict, Err(StorageError::Conflict(_))));
        assert!(store.snapshot("other").is_err());
        let intent = store
            .command_result("create-command")
            .unwrap()
            .unwrap()
            .intent;
        assert!(!String::from_utf8(intent).unwrap().contains("cursor"));
    }
}

#[test]
fn atomic_fork_preserves_exact_selected_branch_and_only_parent_artifact_authority() {
    use ag1_core::context::ContextArtifactPort;
    use ag1_storage::{SessionBootstrap, SessionSeed};
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "parent");
    create(&store, "foreign");
    let mut runtime = resume(&store, "parent", config()).unwrap();
    append_user(&mut runtime, &store, "first", "Selected parent evidence");
    append_user(&mut runtime, &store, "later", "Later branch entry");
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("parent").unwrap(),
        "parent",
        "parent-evidence",
        NOW,
        Features::default(),
    )
    .unwrap();
    let evidence = artifacts
        .put_text(
            "inherited exact evidence",
            "text",
            "read",
            "parent-read",
            &[],
        )
        .unwrap();
    let cursor = store
        .cursor("parent", "parent", &evidence.id, 0, 32768)
        .unwrap();
    store
        .lease(
            "parent",
            "parent",
            &evidence.id,
            "verification",
            60000,
            true,
        )
        .unwrap();
    let mut unrelated = ContextStore::new(
        &store,
        SessionId::new("foreign").unwrap(),
        "foreign",
        "foreign-evidence",
        NOW,
        Features::default(),
    )
    .unwrap();
    let foreign = unrelated
        .put_text("not inherited", "text", "read", "foreign-read", &[])
        .unwrap();
    let old = store.snapshot("parent").unwrap();
    let ack = store
        .bootstrap_session(SessionBootstrap {
            command: "fork-command",
            session: "child",
            origin: "child-origin",
            cursor_key: &[41; 32],
            seed: SessionSeed::Fork {
                parent: "parent",
                expected_revision: old.revision,
                leaf: Some("first"),
            },
        })
        .unwrap();
    assert_eq!(ack.revision, 1);
    let fork = store.snapshot("child").unwrap();
    assert_eq!(fork.leaf, Some("first".into()));
    assert_eq!(fork.entries.len(), 1);
    assert_eq!(fork.entries[0].raw, old.entries[0].raw);
    assert_eq!(
        store.read_cursor("child", &cursor).unwrap().bytes,
        b"inherited exact evidence"
    );
    assert_eq!(store.leases("child").unwrap().len(), 1);
    assert!(
        store
            .artifact_record("child", "foreign", &foreign.id)
            .is_err()
    );
    let mut child = ContextStore::new(
        &store,
        SessionId::new("child").unwrap(),
        "child-origin",
        "child-evidence",
        NOW,
        Features::default(),
    )
    .unwrap();
    let own = child
        .put_text("child-only", "text", "read", "child-read", &[])
        .unwrap();
    assert!(
        store
            .artifact_record("parent", "child-origin", &own.id)
            .is_err()
    );
    append_user(
        &mut runtime,
        &store,
        "newer",
        "Parent advanced after accepted fork",
    );
    assert_eq!(
        store
            .bootstrap_session(SessionBootstrap {
                command: "fork-command",
                session: "child",
                origin: "child-origin",
                cursor_key: &[42; 32],
                seed: SessionSeed::Fork {
                    parent: "parent",
                    expected_revision: old.revision,
                    leaf: Some("first")
                }
            })
            .unwrap(),
        ack
    );
    assert!(matches!(
        store.bootstrap_session(SessionBootstrap {
            command: "stale-fork",
            session: "stale-child",
            origin: "stale-origin",
            cursor_key: &[43; 32],
            seed: SessionSeed::Fork {
                parent: "parent",
                expected_revision: old.revision,
                leaf: None
            }
        }),
        Err(StorageError::Conflict(_))
    ));
    assert!(store.snapshot("stale-child").is_err());
    assert!(matches!(
        store.bootstrap_session(SessionBootstrap {
            command: "origin-conflict",
            session: "other-child",
            origin: "parent",
            cursor_key: &[44; 32],
            seed: SessionSeed::Fork {
                parent: "parent",
                expected_revision: runtime.journal_revision(),
                leaf: None
            }
        }),
        Err(StorageError::Conflict(_))
    ));
    assert!(store.snapshot("other-child").is_err());
    drop(child);
    drop(artifacts);
    drop(unrelated);
    drop(store);
    let store = Store::open(directory.path()).unwrap();
    assert_eq!(
        store.read_cursor("child", &cursor).unwrap().bytes,
        b"inherited exact evidence"
    );
}

#[test]
fn atomic_fork_failure_does_not_publish_a_partial_origin_or_artifact_closure() {
    use ag1_storage::{SessionBootstrap, SessionSeed};
    for point in [Boundary::BeforeCommit, Boundary::BeforeAck] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        create(&store, "parent");
        let mut runtime = resume(&store, "parent", config()).unwrap();
        append_user(&mut runtime, &store, "prompt", "fork this branch");
        fault.armed.store(true, Ordering::Release);
        assert!(
            store
                .bootstrap_session(SessionBootstrap {
                    command: "fork-command",
                    session: "child",
                    origin: "child-origin",
                    cursor_key: &[41; 32],
                    seed: SessionSeed::Fork {
                        parent: "parent",
                        expected_revision: 1,
                        leaf: None
                    }
                })
                .is_err()
        );
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        let committed = point == Boundary::BeforeAck;
        assert_eq!(store.snapshot("child").is_ok(), committed);
        assert_eq!(
            ContextStore::new(
                &store,
                SessionId::new("child").unwrap(),
                "child-origin",
                "inspect-child",
                NOW,
                Features::default()
            )
            .is_ok(),
            committed
        );
        assert_eq!(
            store.command_result("fork-command").unwrap().is_some(),
            committed
        );
        let ack = store
            .bootstrap_session(SessionBootstrap {
                command: "fork-command",
                session: "child",
                origin: "child-origin",
                cursor_key: &[42; 32],
                seed: SessionSeed::Fork {
                    parent: "parent",
                    expected_revision: 1,
                    leaf: None,
                },
            })
            .unwrap();
        assert_eq!(ack.revision, 1);
    }
}
