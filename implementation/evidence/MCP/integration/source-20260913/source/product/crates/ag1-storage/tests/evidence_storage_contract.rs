use ag1_core::{
    DomainError, SessionId,
    evidence::{EvidencePort, EvidenceRequest},
    memory::{MemoryService, MemoryStore},
    storage::StoragePort,
    task::TaskStateService,
};
use ag1_storage::{ArtifactInput, Store, digest};
use serde_json::{Value, json};
const NOW: &str = "2026-09-05T00:00:00.000Z";
fn create(store: &Store, id: &str) {
    store
        .create_session(
            id,
            &serde_json::to_vec(
                &json!({"type":"session","version":3,"id":id,"cwd":"/fixture","timestamp":NOW}),
            )
            .unwrap(),
        )
        .unwrap();
}
#[test]
fn task_commands_commit_once_resume_objective_and_reject_lost_updates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let store = Store::open(&path).unwrap();
    create(&store, "session");
    let session = SessionId::new("session").unwrap();
    let first = TaskStateService::prepare(&store, session.clone(), "cmd1", "e1", NOW, |s| {
        s.capture_instruction("build desktop", &[], NOW)
    })
    .unwrap();
    let ack = first.commit(&store).unwrap();
    assert_eq!(first.commit(&store).unwrap(), ack);
    let second = TaskStateService::prepare(&store, session.clone(), "cmd2", "e2", NOW, |s| {
        s.capture_instruction("fix settings", &[], NOW)
    })
    .unwrap();
    let stale = TaskStateService::prepare(&store, session.clone(), "cmd3", "e3", NOW, |s| {
        s.capture_instruction("stale request", &[], NOW)
    })
    .unwrap();
    second.commit(&store).unwrap();
    assert!(matches!(stale.commit(&store), Err(DomainError::Conflict)));
    drop(store);
    let store = Store::open(&path).unwrap();
    let third = TaskStateService::prepare(&store, session.clone(), "cmd4", "e4", NOW, |s| {
        assert_eq!(s.value()["objective"], "build desktop");
        assert_eq!(s.value()["latestInstruction"], "fix settings");
        s.capture_instruction("continue", &[], NOW)
    })
    .unwrap();
    third.commit(&store).unwrap();
    assert_eq!(store.journal_snapshot(&session).unwrap().entries.len(), 3);
}
#[test]
fn task_service_reads_selected_branch_and_preserves_unknown_journal_records() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("s")).unwrap();
    create(&store, "parent");
    let id = SessionId::new("parent").unwrap();
    TaskStateService::prepare(&store, id.clone(), "c1", "e1", NOW, |s| {
        s.capture_instruction("first objective", &[], NOW)
    })
    .unwrap()
    .commit(&store)
    .unwrap();
    TaskStateService::prepare(&store, id.clone(), "c2", "e2", NOW, |s| {
        s.set(&json!({"objective":"later branch"}), NOW)
    })
    .unwrap()
    .commit(&store)
    .unwrap();
    store.fork("parent", "child", Some("e1")).unwrap();
    let child = SessionId::new("child").unwrap();
    TaskStateService::prepare(&store, child, "c3", "e3", NOW, |s| {
        assert_eq!(s.value()["objective"], "first objective");
        s.capture_instruction("resume child", &[], NOW)
    })
    .unwrap()
    .commit(&store)
    .unwrap();
    assert_eq!(store.snapshot("parent").unwrap().entries.len(), 2);
}
#[test]
fn memory_commands_use_one_cas_store_and_reopen_with_archive_intact() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let store = Store::open(&path).unwrap();
    let mut version = None;
    for n in 0..45 {
        version = Some(
            MemoryService::mutate(&store, "project", version.as_deref(), |s| {
                Ok(
                    s.add("fact", &format!("entry {n}"), n, "normal", &json!({}))?
                        .store,
                )
            })
            .unwrap(),
        );
    }
    assert!(matches!(
        MemoryService::mutate(&store, "project", None, |_| Ok(MemoryStore::default())),
        Err(DomainError::Conflict)
    ));
    drop(store);
    let store = Store::open(&path).unwrap();
    let (loaded, current) = MemoryService::load(&store, "project").unwrap();
    assert_eq!(current, version);
    assert_eq!(loaded.value()["entries"].as_array().unwrap().len(), 40);
    assert_eq!(loaded.value()["archived"].as_array().unwrap().len(), 5);
    let old = current.unwrap();
    MemoryService::mutate(&store, "project", Some(&old), |s| s.remove("m1")).unwrap();
    assert!(MemoryService::mutate(&store, "project", Some(&old), |s| s.remove("m2")).is_err());
}
fn put(store: &Store, binary: bool) -> EvidenceRequest {
    let bytes = if binary {
        let mut b = vec![0; 24];
        b[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        b[19] = 1;
        b[23] = 1;
        b
    } else {
        "first\r\nneedle β 🌙\nlast".as_bytes().to_vec()
    };
    let id = if binary {
        "art_00000000000000000000000000000002"
    } else {
        "art_00000000000000000000000000000001"
    };
    let mut record = json!({"version":1,"id":id,"sessionId":"parent","kind":if binary{"godot-visual"}else{"text"},"mimeType":if binary{"image/png"}else{"text/plain"},"encoding":if binary{"binary"}else{"utf8"},"createdAt":NOW,"sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"source-read"},"unknown":{"retained":true}});
    if binary {
        record["binaryMetadata"] = json!({"width":1,"height":1,"purpose":"fixture","source":"frozen","retentionState":"retained"});
    }
    store
        .put_artifact(&ArtifactInput {
            origin: "origin".into(),
            session: "parent".into(),
            record: serde_json::to_vec(&record).unwrap(),
            bytes,
        })
        .unwrap();
    EvidenceRequest {
        requester: SessionId::new("parent").unwrap(),
        origin: "origin".into(),
        artifact: id.into(),
    }
}
#[test]
fn typed_evidence_pages_cursors_leases_and_binary_provenance_survive_fork_and_restart() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let store = Store::open(&path).unwrap();
    create(&store, "parent");
    create(&store, "stranger");
    store.register_origin("origin", "parent", &[7; 32]).unwrap();
    let mut request = put(&store, false);
    let binary = put(&store, true);
    let cursor = EvidencePort::cursor(&store, &request, 0, 8).unwrap();
    let first = EvidencePort::read_cursor(&store, &request.requester, &cursor).unwrap();
    assert!(!first.bytes.is_empty());
    assert_eq!(
        EvidencePort::search(&store, &request.requester, "needle", 10)
            .unwrap()
            .len(),
        1
    );
    EvidencePort::lease(&store, &request, "retain evidence", 60000, true).unwrap();
    assert_eq!(store.leases("parent").unwrap().len(), 1);
    let meta = EvidencePort::record(&store, &binary).unwrap();
    assert_eq!(meta["binaryMetadata"]["width"], 1);
    assert_eq!(meta["unknown"]["retained"], true);
    assert!(
        EvidencePort::read_cursor(&store, &SessionId::new("stranger").unwrap(), &cursor).is_err()
    );
    let mut denied = request.clone();
    denied.requester = SessionId::new("stranger").unwrap();
    assert!(EvidencePort::page(&store, &denied, 0, 8).is_err());
    assert!(EvidencePort::record(&store, &denied).is_err());
    store.fork("parent", "child", None).unwrap();
    request.requester = SessionId::new("child").unwrap();
    store.archive_session("parent").unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        EvidencePort::read_cursor(&store, &request.requester, &cursor)
            .unwrap()
            .bytes,
        first.bytes
    );
    assert!(EvidencePort::record(&store, &request).unwrap()["producer"].is_object());
    assert!(EvidencePort::page(&store, &request, 0, 0).is_err());
}
#[test]
fn migrated_memory_preserves_unknown_fields_and_never_normalizes_corruption_to_empty() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("s")).unwrap();
    let v = json!({"version":1,"entries":[{"id":"m1","kind":"fact","text":"legacy","createdAt":0,"extra":{"preserve":true}}],"foreign":"opaque"});
    let memory = MemoryStore::parse(&v).unwrap();
    assert_eq!(memory.value()["version"], 3);
    assert_eq!(memory.value()["foreign"], "opaque");
    assert_eq!(memory.value()["entries"][0]["extra"]["preserve"], true);
    MemoryService::mutate(&store, "project", None, |_| Ok(memory)).unwrap();
    let (loaded, _) = MemoryService::load(&store, "project").unwrap();
    assert_eq!(loaded.value()["foreign"], "opaque");
    assert!(MemoryStore::parse(&json!({"version":3,"entries":"corrupt"})).is_err());
    let raw: Value =
        serde_json::from_slice(&store.load_memory("project").unwrap().unwrap().bytes).unwrap();
    assert_eq!(raw["entries"][0]["text"], "legacy");
}

#[test]
fn todo_and_runtime_observations_persist_as_distinct_branch_records() {
    use ag1_core::{
        evidence::{ObservationRecord, RuntimeObservation},
        todo::TodoStateService,
    };
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let store = Store::open(&path).unwrap();
    create(&store, "session");
    let session = SessionId::new("session").unwrap();
    TodoStateService::prepare(
        &store,
        session.clone(),
        "todo-cmd",
        "e1",
        0,
        &json!([{"title":"compile","status":"in_progress"}]),
    )
    .unwrap()
    .commit(&store)
    .unwrap();
    let observation=RuntimeObservation::parse(&json!({"version":1,"id":"obs_012345abcdef","feature":"settings","result":"fail","environment":"linux","revision":"target1","reporter":"user","reproduction":"open settings","notes":"layout incorrect","sources":["artifact:visual"],"observedAt":NOW})).unwrap();
    ObservationRecord::Runtime(observation)
        .prepare(&store, session.clone(), "observation-cmd", "e2", NOW)
        .unwrap()
        .commit(&store)
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let branch = ag1_core::task::branch_values(&store.journal_snapshot(&session).unwrap()).unwrap();
    assert_eq!(branch[0]["customType"], "todo_list");
    assert_eq!(branch[0]["data"]["items"][0]["status"], "in_progress");
    assert_eq!(branch[1]["customType"], "runtime_observation");
    assert_eq!(branch[1]["data"]["result"], "fail");
    assert!(TodoStateService::prepare(&store, session, "bad-cmd", "e3", 1, &json!([])).is_err());
}
