use ag1_core::Clock;
use ag1_storage::*;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
struct Fault {
    point: Boundary,
    armed: AtomicBool,
}
impl FaultInjector for Fault {
    fn check(&self, b: Boundary) -> StorageResult<()> {
        if b == self.point && self.armed.swap(false, Ordering::SeqCst) {
            Err(StorageError::Injected(b))
        } else {
            Ok(())
        }
    }
}
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/storage")
}
fn manifest() -> FrozenManifest {
    serde_json::from_slice(&fs::read(fixture().join("manifest.json")).unwrap()).unwrap()
}
fn oracle() -> Value {
    serde_json::from_slice(&fs::read(fixture().join("oracle.json")).unwrap()).unwrap()
}
fn identities() -> Value {
    serde_json::from_slice(&fs::read(fixture().join("identities.json")).unwrap()).unwrap()
}
fn header(id: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"type":"session","version":3,"id":id,"cwd":"/frozen","timestamp":"t"}),
    )
    .unwrap()
}
fn record(id: &str, parent: Option<&str>) -> Vec<u8> {
    serde_json::to_vec(&json!({"type":"custom","id":id,"parentId":parent,"timestamp":"t","customType":"fixture","data":{"opaque":true}})).unwrap()
}
fn seed(store: &Store) {
    store.create_session("parent", &header("parent")).unwrap();
    store.register_origin("origin", "parent", &[7; 32]).unwrap();
}
fn artifact(id: usize, encoding: &str) -> ArtifactInput {
    let mut bytes = "first\r\nneedle β 🦙 final\n".as_bytes().to_vec();
    if encoding == "binary" {
        bytes = vec![0; 24];
        bytes[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
        bytes[19] = 1;
        bytes[23] = 1;
    }
    let mut v = json!({"version":1,"id":format!("art_{id:032x}"),"sessionId":"parent","kind":if encoding=="binary"{"godot-visual"}else{"text"},"mimeType":if encoding=="binary"{"image/png"}else{"text/plain"},"encoding":encoding,"createdAt":"2026-09-05T00:00:00Z","sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"contract"},"unknown":{"preserved":true}});
    if encoding == "binary" {
        v["binaryMetadata"] = json!({"width":1,"height":1,"purpose":"fixture","source":"frozen","retentionState":"retained"});
    }
    ArtifactInput {
        origin: "origin".into(),
        session: "parent".into(),
        record: serde_json::to_vec(&v).unwrap(),
        bytes,
    }
}
fn canonical(mut values: Vec<Value>) -> Vec<Value> {
    let ids: BTreeMap<String, String> = values
        .iter()
        .filter(|v| v["type"] != "session")
        .enumerate()
        .map(|(i, v)| (v["id"].as_str().unwrap().into(), format!("entry-{}", i + 1)))
        .collect();
    for v in &mut values {
        for key in ["id", "parentId", "firstKeptEntryId", "fromId", "targetId"] {
            if let Some(new) = v[key].as_str().and_then(|s| ids.get(s)) {
                v[key] = new.clone().into();
            }
        }
    }
    values
}
#[test]
fn imported_journals_match_reference_and_retain_physical_and_branch_order() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("store")).unwrap();
    store
        .import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    let expected = oracle();
    for id in ["v1", "v2", "v3", "p01"] {
        let snapshot = store.snapshot(id).unwrap();
        let mut values = vec![serde_json::from_slice(&snapshot.header).unwrap()];
        values.extend(
            snapshot
                .entries
                .iter()
                .map(|e| serde_json::from_slice(&e.normalized).unwrap()),
        );
        assert_eq!(
            canonical(values),
            expected["journals"][id].as_array().unwrap().clone()
        );
    }
    let snapshot = store.snapshot("v3").unwrap();
    assert!(snapshot.entries.iter().any(|e| e.id == "side"));
    assert!(
        !store
            .branch("v3", None)
            .unwrap()
            .iter()
            .any(|e| e.id == "side")
    );
    for f in manifest().files {
        assert_eq!(
            store.original_bytes(&f.path).unwrap(),
            fs::read(fixture().join("source").join(f.path)).unwrap()
        );
    }
}
#[test]
fn memory_versions_preserve_unknowns_and_corruption_is_not_empty() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("store")).unwrap();
    store
        .import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    let expected = oracle();
    for version in 1..=3 {
        let id = format!("memory/v{version}.json");
        let memory = store.memory(&id).unwrap().unwrap();
        assert_eq!(memory.status, "valid");
        let v = memory.normalized.unwrap();
        assert_eq!(v["entries"], expected["memory"][&id]["entries"]);
        assert_eq!(v["unknownTop"]["preserved"], "yes");
    }
    assert!(store.memory("missing").unwrap().is_none());
    for id in [
        "memory/corrupt.json",
        "memory/empty.json",
        "memory/invalid.json",
    ] {
        let v = store.memory(id).unwrap().unwrap();
        assert_eq!(v.status, "corrupt");
        assert!(v.normalized.is_none());
        assert!(
            store
                .save_memory(id, Some(&v.digest), br#"{"version":3,"entries":[]}"#)
                .is_err()
        );
        assert_eq!(
            store.original_bytes(id).unwrap(),
            fs::read(fixture().join("source").join(id)).unwrap()
        );
    }
}
#[test]
fn unicode_crlf_windows_and_both_cursor_versions_match_reference() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("store")).unwrap();
    store
        .import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    let ids = identities();
    let expected = oracle();
    for (offset, p) in expected["pages"].as_array().unwrap().iter().enumerate() {
        let actual = store
            .page(
                "v3",
                "v3",
                ids["records"][0].as_str().unwrap(),
                offset as u64,
                5,
            )
            .unwrap();
        assert_eq!(actual.offset, p["offset"].as_u64().unwrap());
        assert_eq!(
            String::from_utf8(actual.bytes).unwrap(),
            p["text"].as_str().unwrap()
        );
        assert_eq!(actual.next_offset, p["nextOffset"].as_u64());
    }
    for name in ["cursor", "legacy"] {
        let p = store
            .read_cursor("v3", ids[name].as_str().unwrap())
            .unwrap();
        assert_eq!(
            String::from_utf8(p.bytes).unwrap(),
            expected["cursors"][name]["text"]
        );
        assert_eq!(
            p.next_offset,
            expected["cursors"][name]["nextOffset"].as_u64()
        );
    }
}
#[test]
fn fork_reopen_parent_archival_and_retention_keep_authorized_closure() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("store");
    let store = Store::open(&path).unwrap();
    store
        .import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    store.fork("v3", "child", None).unwrap();
    store.archive_session("v3").unwrap();
    store.collect_orphans().unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let ids = identities();
    for id in ids["records"].as_array().unwrap() {
        let id = id.as_str().unwrap();
        let raw = store.artifact_record("child", "v3", id).unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert!(
            !store
                .page("child", "v3", id, 0, 32768)
                .unwrap()
                .bytes
                .is_empty()
        );
        assert!(
            store
                .blob_path(v["sha256"].as_str().unwrap())
                .unwrap()
                .exists()
        );
    }
    for name in ["cursor", "legacy"] {
        assert!(
            store
                .read_cursor("child", ids[name].as_str().unwrap())
                .is_ok()
        );
        assert!(
            store
                .read_cursor("v2", ids[name].as_str().unwrap())
                .is_err()
        );
    }
    assert!(store.snapshot("v3").is_err());
    assert!(store.export(&d.path().join("unsafe"), true).is_err());
}
#[test]
fn cursor_authentication_rejects_mutation_and_cross_session_access() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("s")).unwrap();
    seed(&s);
    let id = s.put_artifact(&artifact(1, "utf8")).unwrap();
    s.create_session("other", &header("other")).unwrap();
    let cursor = s.cursor("parent", "origin", &id, 0, 5).unwrap();
    assert!(s.read_cursor("other", &cursor).is_err());
    for i in 4..cursor.len() {
        let mut bad = cursor.clone().into_bytes();
        bad[i] = if bad[i] == b'A' { b'B' } else { b'A' };
        assert!(
            s.read_cursor("parent", std::str::from_utf8(&bad).unwrap())
                .is_err()
        );
    }
    assert!(s.decode_cursor("parent", &"x".repeat(2049)).is_err());
}
#[test]
fn durable_command_dedup_revision_and_uncertain_external_intents() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let s = Store::open(&path).unwrap();
    seed(&s);
    let input = vec![record("a", None)];
    let ack = s.append_batch("parent", "cmd", 0, &input).unwrap();
    assert_eq!(s.append_batch("parent", "cmd", 0, &input).unwrap(), ack);
    assert!(
        s.append_batch("parent", "cmd", 1, &[record("b", Some("a"))])
            .is_err()
    );
    assert!(s.append_batch("parent", "new", 0, &input).is_err());
    s.record_intent("external", b"write intended").unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    let result = s.command_result("external").unwrap().unwrap();
    assert_eq!(result.state, "uncertain");
    assert!(result.result.is_none());
    assert_eq!(s.snapshot("parent").unwrap().entries.len(), 1);
    let result = s
        .settle_intent("external", b"write intended", b"verified observed bytes")
        .unwrap();
    assert_eq!(result.state, "completed");
    assert!(
        s.settle_intent("external", b"write intended", b"different outcome")
            .is_err()
    );
}
#[test]
fn one_writer_lock_releases_on_drop() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("s");
    let s = Store::open(&p).unwrap();
    assert!(Store::open(&p).is_err());
    drop(s);
    assert!(Store::open(&p).is_ok());
}
#[test]
fn interrupted_import_is_invisible_and_repeated_twice_is_idempotent() {
    for point in [
        Boundary::BeforeBlobSync,
        Boundary::AfterBlobSync,
        Boundary::BeforeRename,
        Boundary::AfterRename,
        Boundary::BeforeImportCheckpoint,
        Boundary::AfterImportCheckpoint,
        Boundary::BeforeTransaction,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s");
        let fault = Arc::new(Fault {
            point,
            armed: AtomicBool::new(true),
        });
        let s = Store::open_with(&p, Arc::new(TestClock(AtomicU64::new(0))), fault).unwrap();
        assert!(
            s.import_frozen(&fixture().join("source"), &manifest())
                .is_err(),
            "{point:?}"
        );
        let state = s.import_status().unwrap().unwrap();
        if state.state != "complete" {
            assert!(s.snapshot("v3").is_err());
        }
        drop(s);
        let s = Store::open(&p).unwrap();
        let first = s
            .import_frozen(&fixture().join("source"), &manifest())
            .unwrap();
        let seq = s.sequence().unwrap();
        let second = s
            .import_frozen(&fixture().join("source"), &manifest())
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(s.sequence().unwrap(), seq);
        assert_eq!(s.snapshot("v3").unwrap().entries.len(), 9);
    }
}
#[test]
fn blob_and_transaction_failures_never_ack_missing_evidence() {
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
        let p = d.path().join("s");
        let fault = Arc::new(Fault {
            point,
            armed: AtomicBool::new(false),
        });
        let s =
            Store::open_with(&p, Arc::new(TestClock(AtomicU64::new(0))), fault.clone()).unwrap();
        seed(&s);
        fault.armed.store(true, Ordering::SeqCst);
        assert!(s.put_artifact(&artifact(1, "binary")).is_err());
        drop(s);
        let s = Store::open(&p).unwrap();
        let found = s.page("parent", "origin", &format!("art_{:032x}", 1), 0, 24);
        if matches!(point, Boundary::AfterCommit | Boundary::BeforeAck) {
            assert_eq!(found.unwrap().bytes, artifact(1, "binary").bytes);
        } else {
            assert!(found.is_err());
        }
        s.collect_orphans().unwrap();
        s.reconcile().unwrap();
    }
}
#[test]
fn committed_journal_survives_lost_ack_and_failed_transaction_rolls_back() {
    for point in [
        Boundary::BeforeTransaction,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s");
        let fault = Arc::new(Fault {
            point,
            armed: AtomicBool::new(false),
        });
        let s =
            Store::open_with(&p, Arc::new(TestClock(AtomicU64::new(0))), fault.clone()).unwrap();
        seed(&s);
        fault.armed.store(true, Ordering::SeqCst);
        let input = vec![record("a", None)];
        assert!(s.append_batch("parent", "c", 0, &input).is_err());
        drop(s);
        let s = Store::open(&p).unwrap();
        let committed = matches!(point, Boundary::AfterCommit | Boundary::BeforeAck);
        assert_eq!(
            s.snapshot("parent").unwrap().entries.len(),
            usize::from(committed)
        );
        s.append_batch("parent", "c", 0, &input).unwrap();
        assert_eq!(s.snapshot("parent").unwrap().entries.len(), 1);
    }
}
#[test]
fn binary_tamper_is_detected_before_read_and_on_reopen() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let s = Store::open(&path).unwrap();
    seed(&s);
    let input = artifact(1, "binary");
    let id = s.put_artifact(&input).unwrap();
    let p = s.blob_path(&digest(&input.bytes)).unwrap();
    fs::write(p, vec![0; input.bytes.len()]).unwrap();
    assert!(s.page("parent", "origin", &id, 0, 24).is_err());
    drop(s);
    assert!(Store::open(&path).is_err());
}
#[test]
fn derived_catalog_is_reconstructed_without_changing_evidence() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let s = Store::open(&path).unwrap();
    seed(&s);
    s.put_artifact(&artifact(1, "utf8")).unwrap();
    let before = s.search("parent", "needle", 10).unwrap();
    drop(s);
    let db = rusqlite::Connection::open(path.join("storage.sqlite")).unwrap();
    db.execute("DELETE FROM catalog", []).unwrap();
    drop(db);
    let s = Store::open(&path).unwrap();
    let after = s.search("parent", "needle", 10).unwrap();
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    drop(s);
    let db = rusqlite::Connection::open(path.join("storage.sqlite")).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM catalog", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn lease_ttl_caps_release_and_binary_bounds_use_injected_clock() {
    let d = tempfile::tempdir().unwrap();
    let clock = Arc::new(TestClock(AtomicU64::new(1000)));
    let s = Store::open_with(
        &d.path().join("s"),
        clock.clone(),
        Arc::new(Fault {
            point: Boundary::BeforeAck,
            armed: AtomicBool::new(false),
        }),
    )
    .unwrap();
    seed(&s);
    for i in 0..17 {
        let id = s.put_artifact(&artifact(i, "utf8")).unwrap();
        let r = s.lease("parent", "origin", &id, "  review evidence  ", 1, true);
        if i < 16 {
            assert_eq!(r.unwrap().expires, 61000);
        } else {
            assert!(r.is_err());
        }
    }
    assert_eq!(s.leases("parent").unwrap().len(), 16);
    clock.0.store(61000, Ordering::SeqCst);
    assert!(s.leases("parent").unwrap().is_empty());
    let id = format!("art_{:032x}", 16);
    s.lease("parent", "origin", &id, "again", u64::MAX, true)
        .unwrap();
    assert_eq!(s.release_verified_leases("parent").unwrap(), 1);
    for i in 100..132 {
        s.put_artifact(&artifact(i, "binary")).unwrap();
    }
    assert!(s.put_artifact(&artifact(133, "binary")).is_err());
    let mut too_large = artifact(200, "binary");
    too_large.bytes = vec![0; 8 * 1024 * 1024 + 1];
    assert!(s.put_artifact(&too_large).is_err());
}
#[test]
fn lossless_export_restore_preserves_raw_records_origins_and_unknown_fields() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("s")).unwrap();
    s.import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    s.fork("v3", "child", None).unwrap();
    let path = d.path().join("export");
    s.export(&path, false).unwrap();
    assert!(s.export(&d.path().join("unsafe"), true).is_err());
    let restored = d.path().join("restored");
    Store::restore_export(&path, &restored).unwrap();
    let r = Store::open(&restored).unwrap();
    assert_eq!(
        s.snapshot("child").unwrap().entries,
        r.snapshot("child").unwrap().entries
    );
    assert!(
        r.read_cursor("child", identities()["legacy"].as_str().unwrap())
            .is_ok()
    );
    for f in manifest().files {
        assert_eq!(
            s.original_bytes(&f.path).unwrap(),
            r.original_bytes(&f.path).unwrap()
        );
    }
    let mut bytes = fs::read(path.join("storage.sqlite")).unwrap();
    bytes[200] ^= 1;
    fs::write(path.join("storage.sqlite"), bytes).unwrap();
    assert!(Store::restore_export(&path, &d.path().join("tampered")).is_err());
    assert!(!d.path().join("tampered").exists());
}
#[test]
fn compatible_export_is_v3_and_unsafe_unknown_entry_downgrade_fails() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("s")).unwrap();
    seed(&s);
    s.append_batch("parent", "c", 0, &[record("a", None)])
        .unwrap();
    s.put_artifact(&artifact(1, "utf8")).unwrap();
    assert_eq!(
        s.export(&d.path().join("compatible"), true).unwrap()["compatible_resume"],
        true
    );
    let v = json!({"type":"future-type","id":"b","parentId":"a","timestamp":"t","payload":{"keep":true}});
    s.append_batch("parent", "future", 1, &[serde_json::to_vec(&v).unwrap()])
        .unwrap();
    assert!(s.export(&d.path().join("unsafe"), true).is_err());
    assert!(s.export(&d.path().join("lossless"), false).is_ok());
}
#[test]
fn preflight_rejects_hash_drift_duplicate_paths_traversal_and_symlinks() {
    let root = fixture().join("source");
    let mut m = manifest();
    m.files[0].sha256 = "0".repeat(64);
    assert!(m.preflight(&root).is_err());
    let mut m = manifest();
    m.files.push(m.files[0].clone());
    assert!(m.preflight(&root).is_err());
    let mut m = manifest();
    m.files[0].path = "../escape".into();
    assert!(m.preflight(&root).is_err());
    #[cfg(unix)]
    {
        let d = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(root, d.path().join("link")).unwrap();
        assert!(manifest().preflight(&d.path().join("link")).is_err());
    }
}
#[test]
fn malformed_journal_fuzz_is_bounded_and_never_panics() {
    let seed = fs::read(fixture().join("source/journals/v3.jsonl")).unwrap();
    for n in 0..512 {
        let mut bytes = seed.clone();
        let pos = (n * 1543) % bytes.len();
        bytes[pos] = (n % 256) as u8;
        let outcome = std::panic::catch_unwind(|| normalize_journal(&bytes, &digest(&bytes)));
        assert!(outcome.is_ok());
    }
    for raw in [
        b"".as_slice(),
        b"null\n",
        b"[]\n",
        b"{\"type\":\"session\",\"id\":\"s\",\"version\":999}\n",
    ] {
        assert!(normalize_journal(raw, &digest(raw)).is_err());
    }
}
#[test]
fn randomized_branch_parent_properties_and_reopen_are_lossless() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("s");
    let s = Store::open(&path).unwrap();
    seed(&s);
    let mut inputs = Vec::new();
    let mut parents = BTreeMap::new();
    let mut rng = 0x5eedu64;
    for i in 0..1000 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = format!("e{i}");
        let parent = if i == 0 {
            None
        } else {
            Some(format!("e{}", rng % i))
        };
        parents.insert(id.clone(), parent.clone());
        inputs.push(record(&id, parent.as_deref()));
    }
    s.append_batch("parent", "batch", 0, &inputs).unwrap();
    drop(s);
    let s = Store::open(&path).unwrap();
    for i in (0..1000).step_by(17) {
        let leaf = format!("e{i}");
        let branch = s.branch("parent", Some(&leaf)).unwrap();
        assert_eq!(branch.last().unwrap().id, leaf);
        for pair in branch.windows(2) {
            assert_eq!(pair[1].parent.as_deref(), Some(pair[0].id.as_str()));
        }
        for e in branch {
            assert_eq!(e.parent, parents[&e.id]);
        }
    }
}
#[test]
fn imported_jobs_and_runtime_are_only_historical_metadata() {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("s")).unwrap();
    s.import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    let jobs = s
        .metadata("historical", "metadata/jobs.json")
        .unwrap()
        .unwrap();
    assert!(
        std::str::from_utf8(&jobs)
            .unwrap()
            .contains("never execute imported metadata")
    );
    assert!(s.command_result("job_frozen").unwrap().is_none());
    assert!(
        s.metadata("historical", "metadata/runtime.json")
            .unwrap()
            .is_some()
    );
}
#[test]
fn future_database_schema_and_wrong_database_identity_are_rejected() {
    for wrong in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("s");
        drop(Store::open(&path).unwrap());
        let db = rusqlite::Connection::open(path.join("storage.sqlite")).unwrap();
        db.execute_batch(if wrong {
            "PRAGMA application_id=1"
        } else {
            "PRAGMA user_version=99"
        })
        .unwrap();
        drop(db);
        assert!(Store::open(&path).is_err());
    }
}

#[test]
fn abrupt_child_exit_recovers_committed_state_without_fabricating_effects() {
    let probe = env!("CARGO_BIN_EXE_storage-probe");
    for operation in ["artifact", "journal", "intent"] {
        let points = if operation == "artifact" {
            vec![
                Boundary::BeforeBlobSync,
                Boundary::AfterBlobSync,
                Boundary::BeforeRename,
                Boundary::AfterRename,
                Boundary::BeforeTransaction,
                Boundary::BeforeCommit,
                Boundary::AfterCommit,
                Boundary::BeforeAck,
            ]
        } else {
            vec![
                Boundary::BeforeTransaction,
                Boundary::BeforeCommit,
                Boundary::AfterCommit,
                Boundary::BeforeAck,
            ]
        };
        for point in points.into_iter().chain([Boundary::AfterAck]) {
            let d = tempfile::tempdir().unwrap();
            let path = d.path().join("s");
            assert!(
                std::process::Command::new(probe)
                    .arg("seed")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            let status = std::process::Command::new(probe)
                .arg("crash")
                .arg(&path)
                .arg(format!("{point:?}"))
                .arg(operation)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(91), "{operation} {point:?}");
            let store = Store::open(&path).unwrap();
            let committed = matches!(
                point,
                Boundary::AfterCommit | Boundary::BeforeAck | Boundary::AfterAck
            );
            if point == Boundary::AfterAck {
                assert!(path.join("delivered-ack").exists());
            }
            match operation {
                "artifact" => assert_eq!(
                    store
                        .page("s", "s", &format!("art_{:032x}", 1), 0, 32)
                        .is_ok(),
                    committed
                ),
                "journal" => assert_eq!(
                    store.snapshot("s").unwrap().entries.len(),
                    usize::from(committed)
                ),
                _ => {
                    let result = store.command_result("external").unwrap();
                    assert_eq!(result.is_some(), committed);
                    if let Some(result) = result {
                        assert_eq!(result.state, "uncertain");
                        assert!(result.result.is_none());
                    }
                }
            }
            store.collect_orphans().unwrap();
            store.reconcile().unwrap();
        }
    }
}

#[test]
fn core_storage_ports_provide_cas_memory_and_settings_without_database_handles() {
    use ag1_core::storage::StoragePort;
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("s")).unwrap();
    seed(&store);
    let port: &dyn StoragePort = &store;
    let bytes = br#"{"version":3,"entries":[],"opaque":{"retained":true}}"#;
    let hash = port.commit_memory("memory", None, bytes).unwrap();
    assert_eq!(port.load_memory("memory").unwrap().unwrap().bytes, bytes);
    assert!(port.commit_memory("memory", None, bytes).is_err());
    port.commit_memory("memory", Some(&hash), bytes).unwrap();
    let settings = br#"{"theme":"dark","unknown":false}"#;
    let hash = port
        .commit_metadata("settings", "desktop", None, settings)
        .unwrap();
    assert_eq!(
        port.load_metadata("settings", "desktop")
            .unwrap()
            .unwrap()
            .sha256,
        hash
    );
    assert!(
        port.commit_metadata("historical", "runtime", None, settings)
            .is_err()
    );
    let id = ag1_core::SessionId::new("parent").unwrap();
    assert!(port.journal_snapshot(&id).unwrap().entries.is_empty());
}

#[test]
fn missing_historical_artifact_or_origin_key_cannot_publish_an_import() {
    for omit in [FileKind::Artifact, FileKind::CursorKey] {
        let d = tempfile::tempdir().unwrap();
        let mut m = manifest();
        let index = m.files.iter().position(|f| f.kind == omit).unwrap();
        m.files.remove(index);
        let source = d.path().join("source");
        fs::create_dir(&source).unwrap();
        for f in &m.files {
            let target = source.join(&f.path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture().join("source").join(&f.path), target).unwrap();
        }
        let store = Store::open(&d.path().join("s")).unwrap();
        assert!(store.import_frozen(&source, &m).is_err());
        assert_eq!(store.import_status().unwrap().unwrap().state, "staging");
        assert!(store.snapshot("v3").is_err());
    }
}

#[test]
fn normal_put_cannot_grant_an_unrelated_session_access_to_foreign_evidence() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("s")).unwrap();
    seed(&store);
    let mut input = artifact(1, "utf8");
    store.put_artifact(&input).unwrap();
    store.create_session("other", &header("other")).unwrap();
    input.session = "other".into();
    assert!(store.put_artifact(&input).is_err());
    assert!(
        store
            .page("other", "origin", &format!("art_{:032x}", 1), 0, 10)
            .is_err()
    );
}

#[test]
fn all_valid_frozen_bytes_survive_repeated_lossless_export_restore() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("s")).unwrap();
    store
        .import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    let first = d.path().join("first");
    store.export(&first, false).unwrap();
    let restored = d.path().join("restored");
    Store::restore_export(&first, &restored).unwrap();
    let store = Store::open(&restored).unwrap();
    let second = d.path().join("second");
    store.export(&second, false).unwrap();
    for f in manifest().files {
        assert_eq!(
            fs::read(first.join("original").join(&f.path)).unwrap(),
            fs::read(second.join("original").join(&f.path)).unwrap()
        );
    }
}

#[test]
fn previously_evicted_binary_metadata_is_retained_without_inventing_bytes() {
    let d = tempfile::tempdir().unwrap();
    let source = d.path().join("source");
    fs::create_dir(&source).unwrap();
    let mut m = manifest();
    for f in &m.files {
        let target = source.join(&f.path);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture().join("source").join(&f.path), target).unwrap();
    }
    let binary_id = identities()["records"][4].as_str().unwrap().to_owned();
    let record: Value = serde_json::from_slice(
        &fs::read(source.join(format!("artifacts/v3/records/{binary_id}.json"))).unwrap(),
    )
    .unwrap();
    let hash = record["sha256"].as_str().unwrap();
    for f in m
        .files
        .iter()
        .filter(|f| f.kind == FileKind::Blob && f.sha256 == hash)
    {
        fs::remove_file(source.join(&f.path)).unwrap();
    }
    m.files
        .retain(|f| !(f.kind == FileKind::Blob && f.sha256 == hash));
    let state = m
        .files
        .iter_mut()
        .find(|f| f.path.ends_with("godot-visual-retention.json"))
        .unwrap();
    let raw = serde_json::to_vec(
        &json!({"version":1,"updatedAt":"2026-09-05T00:00:00Z","retainedArtifactIds":[]}),
    )
    .unwrap();
    state.sha256 = digest(&raw);
    state.bytes = raw.len() as u64;
    fs::write(source.join(&state.path), raw).unwrap();
    let path = d.path().join("s");
    let s = Store::open(&path).unwrap();
    s.import_frozen(&source, &m).unwrap();
    assert!(s.artifact_record("v3", "v3", &binary_id).is_ok());
    assert!(matches!(
        s.page("v3", "v3", &binary_id, 0, 24),
        Err(StorageError::Missing(_))
    ));
    let export = d.path().join("export");
    assert!(s.export(&d.path().join("unsafe"), true).is_err());
    s.export(&export, false).unwrap();
    drop(s);
    let restored = d.path().join("restored");
    Store::restore_export(&export, &restored).unwrap();
    let s = Store::open(&restored).unwrap();
    assert!(s.page("v3", "v3", &binary_id, 0, 24).is_err());
    assert!(s.artifact_record("v3", "v3", &binary_id).is_ok());
}

#[test]
fn reference_literal_search_returns_the_same_evidence_set() {
    let d = tempfile::tempdir().unwrap();
    let store = Store::open(&d.path().join("s")).unwrap();
    store
        .import_frozen(&fixture().join("source"), &manifest())
        .unwrap();
    let mut ids: Vec<_> = store
        .search("v3", "needle", 100)
        .unwrap()
        .into_iter()
        .map(|h| h.artifact)
        .collect();
    ids.sort();
    ids.dedup();
    let mut expected: Vec<_> = oracle()["search"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    expected.sort();
    assert_eq!(ids, expected);
}

#[test]
fn export_never_replaces_a_destination_created_during_publication() {
    struct Race(PathBuf);
    impl FaultInjector for Race {
        fn check(&self, b: Boundary) -> StorageResult<()> {
            if b == Boundary::BeforeRename {
                fs::create_dir(&self.0)?;
            }
            Ok(())
        }
    }
    let d = tempfile::tempdir().unwrap();
    let destination = d.path().join("export");
    let store = Store::open_with(
        &d.path().join("store"),
        Arc::new(TestClock(AtomicU64::new(0))),
        Arc::new(Race(destination.clone())),
    )
    .unwrap();
    store.create_session("s", &header("s")).unwrap();
    assert!(store.export(&destination, true).is_err());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}
