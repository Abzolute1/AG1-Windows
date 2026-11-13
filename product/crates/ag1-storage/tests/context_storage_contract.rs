use ag1_core::{
    DomainError, SessionId,
    context::{
        ContextArtifactPort,
        compiler::CompileOptions,
        episode::{self, EpisodeCommand},
        features::Features,
    },
    storage::StoragePort,
};
use ag1_storage::{Store, context::ContextStore};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
const NOW: &str = "2026-09-05T00:00:00.000Z";
fn create(store: &Store, id: &str) {
    store
        .create_session(
            id,
            &json!({"type":"session","version":3,"id":id,"cwd":"/fixture","timestamp":NOW})
                .to_string()
                .into_bytes(),
        )
        .unwrap();
    store.register_origin(id, id, &[7; 32]).unwrap();
}
fn adapter<'a>(store: &'a Store, id: &str, op: &str) -> ContextStore<'a> {
    ContextStore::new(
        store,
        SessionId::new(id).unwrap(),
        id,
        op,
        NOW,
        Features::default(),
    )
    .unwrap()
}
fn read(store: &Store, session: &str, origin: &str, id: &str) -> Vec<u8> {
    let mut offset = 0;
    let mut bytes = vec![];
    loop {
        let page = store.page(session, origin, id, offset, 32768).unwrap();
        bytes.extend(page.bytes);
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    bytes
}
#[test]
fn small_history_archives_reopen_without_becoming_fresh_search_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store");
    let store = Store::open(&path).unwrap();
    create(&store, "session");
    let mut messages = vec![];
    for index in 0..100 {
        messages.push(json!({"role":"user","content":format!("request-custody-marker {index}: {}", "historical instruction ".repeat(20))}));
        messages.push(json!({"role":"assistant","content":[{"type":"text","text":"historical result ".repeat(20)}]}));
    }
    let result = adapter(&store, "session", "history-pressure")
        .compile(
            &messages,
            &CompileOptions {
                context_window: 32_768,
                max_working_tokens: Some(8192),
                ..Default::default()
            },
            0,
        )
        .unwrap();
    assert_eq!(result.projection.stats["overTargetTokens"], 0);
    let archive = store
        .artifact_catalog("session", None, 10, Some("context-history-segment"))
        .unwrap();
    assert_eq!(archive.len(), 1);
    let id = archive[0].record["id"].as_str().unwrap().to_owned();
    let bytes = read(&store, "session", "session", &id);
    assert!(String::from_utf8_lossy(&bytes).contains("request-custody-marker 0:"));
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(read(&store, "session", "session", &id), bytes);
    assert_eq!(
        serde_json::from_slice::<Value>(&read(
            &store,
            "session",
            "session",
            &result.original_request.id
        ))
        .unwrap(),
        json!(messages)
    );
    assert!(store.evidence_catalog("session", 100).unwrap().is_empty());
    assert!(store.page("foreign", "session", &id, 0, 32768).is_err());
    assert!(
        std::sync::Arc::new(store)
            .capture_evidence_search("session", None)
            .unwrap()
            .search("request-custody-marker", true, 12, 3, || Ok(()))
            .unwrap()
            .matches
            .is_empty()
    );
}
#[test]
fn shadow_compilation_retains_request_and_exact_original_journal_across_restart() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let store = Store::open(&path).unwrap();
    create(&store, "session");
    let original=b"{ \"type\":\"message\", \"id\":\"e1\",\"parentId\":null,\"timestamp\":\"2026-09-05T00:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"original\"},\"unknown\":{\"keep\":true}}";
    store
        .append_batch("session", "write", 0, &[original.to_vec()])
        .unwrap();
    let mut messages = vec![
        json!({"role":"user","content":"original"}),
        json!({"role":"assistant","content":[{"type":"thinking","thinking":"old thought ".repeat(2000)},{"type":"text","text":"historical reply"}]}),
        json!({"role":"user","content":"continue"}),
    ];
    for _ in 0..4 {
        messages
            .push(json!({"role":"assistant","content":[{"type":"text","text":"recent reply"}]}));
        messages.push(json!({"role":"user","content":"continue"}));
    }
    let options = CompileOptions {
        context_window: 32768,
        fixed_tokens: Some(32000),
        ..Default::default()
    };
    let mut context = adapter(&store, "session", "compile1");
    let result = context.compile(&messages, &options, 1).unwrap();
    assert_ne!(result.projection.messages, messages);
    let same = adapter(&store, "session", "compile1")
        .compile(&messages, &options, 1)
        .unwrap();
    assert_eq!(same.original_request.id, result.original_request.id);
    assert_eq!(same.original_journal.id, result.original_journal.id);
    let request_id = result.original_request.id;
    let journal_id = result.original_journal.id;
    drop(context);
    drop(store);
    let store = Store::open(&path).unwrap();
    let retained: Value =
        serde_json::from_slice(&read(&store, "session", "session", &journal_id)).unwrap();
    assert_eq!(
        B64.decode(retained["entries"][0]["rawBase64"].as_str().unwrap())
            .unwrap(),
        original
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&read(&store, "session", "session", &request_id)).unwrap(),
        json!(messages)
    );
    assert!(matches!(
        adapter(&store, "session", "stale").compile(&messages, &options, 0),
        Err(DomainError::Conflict)
    ));
}
#[test]
fn episode_close_commits_after_durable_original_trajectory_and_obeys_branch_cas() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let store = Store::open(&path).unwrap();
    create(&store, "session");
    let session = SessionId::new("session").unwrap();
    let command = |name, id| EpisodeCommand {
        session: session.clone(),
        command: name,
        entry_id: id,
        now: NOW,
    };
    episode::prepare_begin(
        &store,
        command("begin", "begin-entry"),
        "ep_000000000001",
        "parser bounds",
        None,
    )
    .unwrap()
    .commit(&store)
    .unwrap();
    assert!(
        episode::prepare_begin(
            &store,
            command("duplicate", "duplicate-entry"),
            "ep_000000000002",
            "duplicate",
            None
        )
        .is_err()
    );
    let raw=b"{ \"type\":\"message\",\"id\":\"result\",\"parentId\":\"begin-entry\",\"timestamp\":\"2026-09-05T00:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"exact original\"},\"future\":42}";
    store
        .append_batch("session", "result", 1, &[raw.to_vec()])
        .unwrap();
    let mut artifacts = adapter(&store, "session", "close-artifacts");
    let close=episode::prepare_close(&store,&mut artifacts,command("close","closed-entry"),&json!({"outcome":"completed","synopsis":"parser bounds reviewed","claims":[{"kind":"HYPOTHESIS","statement":"needs live validation","sources":[]}]})).unwrap();
    let record: Value = serde_json::from_slice(&close.records[0]).unwrap();
    let id = record["data"]["trajectoryArtifactId"]
        .as_str()
        .unwrap()
        .to_string();
    let trajectory = read(&store, "session", "session", &id);
    assert!(trajectory.windows(raw.len()).any(|v| v == raw));
    assert!(
        !trajectory
            .windows(b"closed-entry".len())
            .any(|v| v == b"closed-entry")
    );
    close.commit(&store).unwrap();
    close.commit(&store).unwrap();
    assert!(
        episode::find_open(
            &ag1_core::task::branch_values(&store.journal_snapshot(&session).unwrap()).unwrap()
        )
        .is_none()
    );
    store.fork("session", "child", Some("result")).unwrap();
    store.register_origin("child", "child", &[8; 32]).unwrap();
    let mut child = adapter(&store, "child", "child-close");
    assert!(child.get(&id).is_ok());
    let child_close = episode::prepare_close(
        &store,
        &mut child,
        EpisodeCommand {
            session: SessionId::new("child").unwrap(),
            command: "cc",
            entry_id: "ce",
            now: NOW,
        },
        &json!({"outcome":"partial","synopsis":"child independent close"}),
    )
    .unwrap();
    child_close.commit(&store).unwrap();
    drop(child);
    drop(artifacts);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(read(&store, "session", "session", &id), trajectory);
    create(&store, "stranger");
    assert!(adapter(&store, "stranger", "read").get(&id).is_err());
}
#[test]
fn disabled_and_expired_leases_do_not_protect_evidence_and_cursors_reject_foreign_readers() {
    use ag1_core::Clock;
    use ag1_storage::{Boundary, FaultInjector, StorageResult};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    struct TestClock(AtomicU64);
    impl Clock for TestClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    struct NoFault;
    impl FaultInjector for NoFault {
        fn check(&self, _: Boundary) -> StorageResult<()> {
            Ok(())
        }
    }
    let d = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1000)));
    let store = Store::open_with(&d.path().join("s"), clock.clone(), Arc::new(NoFault)).unwrap();
    create(&store, "session");
    create(&store, "stranger");
    let mut context = adapter(&store, "session", "put");
    let record = context
        .put_text("exact β\n", "tool-output", "read", "call", &[])
        .unwrap();
    assert_eq!(record.line_count, Some(1));
    store
        .lease(
            "session",
            "session",
            &record.id,
            "decisive proof",
            1000,
            false,
        )
        .unwrap();
    assert_eq!(context.leased_ids().unwrap(), vec![record.id.clone()]);
    let disabled = ContextStore::new(
        &store,
        SessionId::new("session").unwrap(),
        "session",
        "disabled",
        NOW,
        Features {
            evidence_leases: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(disabled.leased_ids().unwrap().is_empty());
    let cursor = store
        .cursor("session", "session", &record.id, 0, 32768)
        .unwrap();
    assert_eq!(
        store.read_cursor("session", &cursor).unwrap().bytes,
        b"exact \xce\xb2\n"
    );
    assert!(store.read_cursor("stranger", &cursor).is_err());
    let mut forged = cursor.into_bytes();
    let end = forged.len() - 1;
    forged[end] = if forged[end] == b'A' { b'B' } else { b'A' };
    assert!(
        store
            .read_cursor("session", std::str::from_utf8(&forged).unwrap())
            .is_err()
    );
    clock.0.store(100000, Ordering::SeqCst);
    assert!(context.leased_ids().unwrap().is_empty());
}

#[test]
fn capsule_journal_requires_recoverable_matching_artifact_and_survives_restart() {
    use ag1_core::context::{capsule, retrieval::CapsuleDraft};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/source-oracle.json"
    ))
    .unwrap();
    let expected = &oracle["groups"][0];
    let mut value = expected["record"].clone();
    value["body"] = expected["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .split("\ncapsule_artifact:")
        .next()
        .unwrap()
        .into();
    value["retrieval"] = expected["result"]["details"]["retrieval"].clone();
    let draft: CapsuleDraft = serde_json::from_value(value).unwrap();
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let store = Store::open(&path).unwrap();
    create(&store, "session");
    let mut context = adapter(&store, "session", "capsule");
    let command = |name, id| EpisodeCommand {
        session: SessionId::new("session").unwrap(),
        command: name,
        entry_id: id,
        now: NOW,
    };
    let prepared =
        capsule::prepare(&store, &mut context, command("capsule", "e1"), &draft).unwrap();
    let artifact = prepared.record.artifact_id.clone();
    assert_eq!(
        read(&store, "session", "session", &artifact),
        draft.body.as_bytes()
    );
    prepared.commit.unwrap().commit(&store).unwrap();
    let reused = capsule::prepare(&store, &mut context, command("reuse", "e2"), &draft).unwrap();
    assert!(reused.commit.is_none());
    assert_eq!(reused.record.artifact_id, artifact);
    let mut forged = draft.clone();
    forged.body.push_str("wrong body");
    assert!(capsule::prepare(&store, &mut context, command("bad", "bad"), &forged).is_err());
    drop(context);
    drop(store);
    let store = Store::open(&path).unwrap();
    let rows = ag1_core::task::branch_values(
        &store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let record = capsule::find(&rows, &draft.id).unwrap();
    assert_eq!(record.artifact_id, artifact);
    assert_eq!(
        read(&store, "session", "session", &artifact),
        draft.body.as_bytes()
    );
    // A forged display path cannot borrow a different file's valid digest.
    let mut forged = json!(record);
    forged["files"][0]["path"] = "src/other.rs".into();
    assert!(capsule::CapsuleRecord::parse(&forged).is_err());
}

#[test]
fn custody_failure_never_invokes_reduction_and_uncertain_commits_recover() {
    use ag1_core::Clock;
    use ag1_core::context::compiler::Compilation;
    use ag1_storage::{Boundary, FaultInjector, StorageError, StorageResult};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            1000
        }
    }
    struct Fault {
        point: Boundary,
        armed: AtomicBool,
    }
    impl FaultInjector for Fault {
        fn check(&self, point: Boundary) -> StorageResult<()> {
            if point == self.point && self.armed.swap(false, Ordering::SeqCst) {
                Err(StorageError::Injected(point))
            } else {
                Ok(())
            }
        }
    }
    for point in [
        Boundary::BeforeBlobSync,
        Boundary::AfterBlobSync,
        Boundary::BeforeRename,
        Boundary::AfterRename,
        Boundary::BeforeTransaction,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("store");
        let fault = Arc::new(Fault {
            point,
            armed: AtomicBool::new(false),
        });
        let store = Store::open_with(&path, Arc::new(FixedClock), fault.clone()).unwrap();
        create(&store, "session");
        fault.armed.store(true, Ordering::SeqCst);
        let messages = vec![json!({"role":"user","content":"Do not lose this exact objective"})];
        let mut called = false;
        let result = adapter(&store, "session", "custody").project(&messages, 0, |_, _| {
            called = true;
            unreachable!("reducer must not run before custody acknowledgement")
        });
        assert!(result.is_err(), "{point:?}");
        assert!(!called);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap()
                .revision,
            0
        );
        let result = adapter(&store, "session", "custody")
            .project(&messages, 0, |_, _| {
                Ok(Compilation {
                    messages: vec![json!({"role":"user","content":"short projection"})],
                    stats: json!({}),
                })
            })
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&read(
                &store,
                "session",
                "session",
                &result.original_request.id
            ))
            .unwrap(),
            json!(messages)
        );
    }
}

#[test]
fn shadow_transaction_rejects_orphans_and_revision_changes_during_reduction() {
    use ag1_core::context::compiler::Compilation;
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("store")).unwrap();
    create(&store, "session");
    let messages = vec![json!({"role":"user","content":"exact original"})];
    let orphan = adapter(&store, "session", "orphan").project(&messages, 0, |_, _| {
        Ok(Compilation {
            messages: vec![json!({"role":"toolResult","toolCallId":"absent","content":[]})],
            stats: json!({}),
        })
    });
    assert!(matches!(orphan, Err(DomainError::InvalidInput(_))));
    let changed = adapter(&store,"session","changed").project(&messages,0,|messages,_| {
        store.append_batch("session","write",0,&[json!({"type":"message","id":"e1","parentId":null,"timestamp":NOW,"message":{"role":"user","content":"new steering"}}).to_string().into_bytes()]).unwrap();
        Ok(Compilation { messages:messages.to_vec(),stats:json!({}) })
    });
    assert!(matches!(changed, Err(DomainError::Conflict)));
}

#[test]
fn shadow_revision_guard_retains_archived_session_and_import_fences() {
    use ag1_core::context::compiler::Compilation;
    for archive in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("store");
        let store = Store::open(&path).unwrap();
        create(&store, "session");
        let messages = vec![json!({"role":"user","content":"exact original"})];
        let result = adapter(&store, "session", "guard").project(&messages, 0, |messages, _| {
            if archive {
                store.archive_session("session").unwrap();
            } else {
                let external = rusqlite::Connection::open(path.join("storage.sqlite")).unwrap();
                external.execute("INSERT INTO imports(id,manifest,state) VALUES('fixture',X'7b7d','staging')", []).unwrap();
            }
            Ok(Compilation { messages: messages.to_vec(), stats: json!({}) })
        });
        if archive {
            assert!(matches!(result, Err(DomainError::NotFound)));
        } else {
            assert!(
                matches!(result, Err(DomainError::Io(message)) if message == "ImportIncomplete")
            );
        }
    }
}
