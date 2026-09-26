use super::*;
use ag1_core::context::ContextArtifactPort;
use ag1_storage::{SessionBootstrap, SessionSeed};
use std::collections::BTreeMap;

const FORK_TIME: &str = "2026-09-07T01:02:03.000Z";

#[test]
fn runtime_forks_preserve_resolved_source_labels_and_correct_removed_label_ancestry() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/session-fork-oracle.json"
    ))
    .unwrap();
    for case in fixture["rows"].as_array().unwrap() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        create(&store, "parent");
        let input = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| serde_json::to_vec(v).unwrap())
            .collect::<Vec<_>>();
        store.append_batch("parent", "populate", 0, &input).unwrap();
        let original = store.snapshot("parent").unwrap();
        let ack = store
            .bootstrap_session(SessionBootstrap {
                command: "fork",
                session: "child",
                origin: "child",
                cursor_key: &[19; 32],
                seed: SessionSeed::RuntimeFork {
                    parent: "parent",
                    expected_revision: original.revision,
                    leaf: Some("answer"),
                    timestamp: FORK_TIME,
                },
            })
            .unwrap();
        let saved = store.snapshot("child").unwrap();
        let branch = store.branch("child", None).unwrap();
        assert_eq!(branch, saved.entries, "{}", case["scenario"]);
        assert_eq!(ack.revision, branch.len() as u64);
        let mut names = BTreeMap::new();
        let mut labels = 0;
        for row in &branch {
            let value: Value = serde_json::from_slice(&row.normalized).unwrap();
            let name = if value["type"] == "label" {
                labels += 1;
                format!("fork-label-{labels}")
            } else {
                row.id.clone()
            };
            names.insert(row.id.clone(), name);
        }
        let normalized = branch
            .iter()
            .map(|row| {
                let mut value: Value = serde_json::from_slice(&row.normalized).unwrap();
                value["id"] = json!(names[&row.id]);
                value["parentId"] = json!(row.parent.as_ref().map(|p| &names[p]));
                value
            })
            .collect::<Vec<_>>();
        let mut expected = case["entries"].as_array().unwrap().clone();
        let mut parent = Value::Null;
        for value in &mut expected {
            value["parentId"] = parent;
            parent = value["id"].clone();
        }
        assert_eq!(normalized, expected, "{}", case["scenario"]);
        assert_eq!(
            normalized.iter().filter(|v| v["type"] == "message").count(),
            2
        );
        if case["scenario"] != "plain" {
            assert_eq!(case["danglingParents"], json!(["answer"]));
            assert!(
                !case["branch"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["id"] == "first")
            );
        }
        let header: Value = serde_json::from_slice(&saved.header).unwrap();
        assert_eq!(header["timestamp"], FORK_TIME);
        assert_eq!(header["id"], "child");
        assert_eq!(header["parentSession"], "parent");
        assert_eq!(store.snapshot("parent").unwrap().entries, original.entries);
    }
}

#[test]
fn empty_runtime_fork_has_a_parent_header_without_inheriting_unreferenced_artifact_access() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    create(&store, "parent");
    let row = serde_json::to_vec(&json!({"type":"message","id":"first","parentId":null,"timestamp":NOW,"message":{"role":"user","content":"First"}})).unwrap();
    store.append_batch("parent", "populate", 0, &[row]).unwrap();
    let mut parent = ContextStore::new(
        &store,
        SessionId::new("parent").unwrap(),
        "parent",
        "artifact",
        NOW,
        Features::default(),
    )
    .unwrap();
    let evidence = parent
        .put_text("parent evidence", "text", "read", "read", &[])
        .unwrap();
    for (child, leaf) in [("empty", None), ("branch", Some("first"))] {
        store
            .bootstrap_session(SessionBootstrap {
                command: child,
                session: child,
                origin: child,
                cursor_key: &[22; 32],
                seed: SessionSeed::RuntimeFork {
                    parent: "parent",
                    expected_revision: 1,
                    leaf,
                    timestamp: FORK_TIME,
                },
            })
            .unwrap();
        assert_eq!(
            store.snapshot(child).unwrap().entries.len(),
            usize::from(leaf.is_some())
        );
        assert_eq!(
            store.artifact_record(child, "parent", &evidence.id).is_ok(),
            leaf.is_some()
        );
        let mut context = ContextStore::new(
            &store,
            SessionId::new(child).unwrap(),
            child,
            "own",
            NOW,
            Features::default(),
        )
        .unwrap();
        let own = context
            .put_text("child evidence", "text", "read", "read", &[])
            .unwrap();
        assert!(store.artifact_record("parent", child, &own.id).is_err());
    }
}

struct ForkFault {
    boundary: Boundary,
    armed: AtomicBool,
}
impl FaultInjector for ForkFault {
    fn check(&self, boundary: Boundary) -> StorageResult<()> {
        if boundary == self.boundary && self.armed.swap(false, Ordering::AcqRel) {
            return Err(StorageError::Injected(boundary));
        }
        Ok(())
    }
}
#[test]
fn runtime_fork_commit_faults_preserve_atomic_origin_and_exact_retry_after_parent_changes() {
    for boundary in [Boundary::BeforeCommit, Boundary::BeforeAck] {
        let dir = tempfile::tempdir().unwrap();
        let fault = Arc::new(ForkFault {
            boundary,
            armed: AtomicBool::new(false),
        });
        let store = Store::open_with(dir.path(), Arc::new(TestClock), fault.clone()).unwrap();
        create(&store, "parent");
        let row=serde_json::to_vec(&json!({"type":"message","id":"first","parentId":null,"timestamp":NOW,"message":{"role":"user","content":"First"}})).unwrap();
        store.append_batch("parent", "populate", 0, &[row]).unwrap();
        let request = || SessionBootstrap {
            command: "fork",
            session: "child",
            origin: "child",
            cursor_key: &[33; 32],
            seed: SessionSeed::RuntimeFork {
                parent: "parent",
                expected_revision: 1,
                leaf: Some("first"),
                timestamp: FORK_TIME,
            },
        };
        fault.armed.store(true, Ordering::Release);
        assert!(store.bootstrap_session(request()).is_err());
        assert!(!fault.armed.load(Ordering::Acquire));
        assert_eq!(
            store.snapshot("child").is_ok(),
            boundary == Boundary::BeforeAck
        );
        assert_eq!(
            store.command_result("fork").unwrap().is_some(),
            boundary == Boundary::BeforeAck
        );
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        let ack = store.bootstrap_session(request()).unwrap();
        let row=serde_json::to_vec(&json!({"type":"message","id":"later","parentId":"first","timestamp":NOW,"message":{"role":"user","content":"Later"}})).unwrap();
        store.append_batch("parent", "advance", 1, &[row]).unwrap();
        assert_eq!(store.bootstrap_session(request()).unwrap(), ack);
        assert_eq!(store.snapshot("child").unwrap().entries.len(), 1);
        let mut child = ContextStore::new(
            &store,
            SessionId::new("child").unwrap(),
            "child",
            "write",
            NOW,
            Features::default(),
        )
        .unwrap();
        child
            .put_text("retained origin", "text", "read", "read", &[])
            .unwrap();
        let mut stale = request();
        stale.command = "stale";
        stale.session = "stale";
        stale.origin = "stale";
        assert!(store.bootstrap_session(stale).is_err());
    }
}

struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        1000
    }
}
