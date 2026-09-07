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

#[test]
fn native_output_origin_creation_preserves_existing_keys_and_rejects_foreign_ownership() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store");
    let store = Store::open(&path).unwrap();
    seed(&store);
    let id = store.put_artifact(&artifact(1, "utf8")).unwrap();
    let cursor = store.cursor("parent", "origin", &id, 0, 5).unwrap();
    let sequence = store.sequence().unwrap();
    assert!(!store.ensure_output_origin("origin", "parent").unwrap());
    assert_eq!(store.sequence().unwrap(), sequence);
    store.create_session("other", &header("other")).unwrap();
    assert!(store.ensure_output_origin("origin", "other").is_err());
    assert!(store.ensure_output_origin("missing", "missing").is_err());
    assert!(store.ensure_output_origin("other", "other").unwrap());
    let sequence = store.sequence().unwrap();
    assert!(!store.ensure_output_origin("other", "other").unwrap());
    assert_eq!(store.sequence().unwrap(), sequence);
    assert!(store.read_cursor("parent", &cursor).is_ok());
    drop(store);
    let store = Store::open(&path).unwrap();
    assert!(!store.ensure_output_origin("other", "other").unwrap());
    assert_eq!(store.cursor("parent", "origin", &id, 0, 5).unwrap(), cursor);
    assert!(store.read_cursor("other", &cursor).is_err());
}

#[test]
fn quiescent_inspection_is_read_only_and_does_not_recover_pending_state() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("store");
    assert!(Store::inspect_quiescent(&path, "parent").is_err());
    assert!(!path.exists());
    let store = Store::open(&path).unwrap();
    seed(&store);
    let expected = store.snapshot("parent").unwrap();
    assert!(Store::inspect_quiescent(&path, "parent").is_err());
    drop(store);
    fs::write(path.join("staging/unpublished"), b"retain staged bytes").unwrap();
    let before = fs::read(path.join("storage.sqlite")).unwrap();
    let (health, snapshot) = Store::inspect_quiescent(&path, "parent").unwrap();
    assert_eq!(snapshot.sequence, expected.sequence);
    assert_eq!(health.staged_files, 1);
    assert_eq!(fs::read(path.join("storage.sqlite")).unwrap(), before);
    assert_eq!(
        fs::read(path.join("staging/unpublished")).unwrap(),
        b"retain staged bytes"
    );
    assert!(!path.join("quarantine/unpublished").exists());
    let db = rusqlite::Connection::open(path.join("storage.sqlite")).unwrap();
    db.execute(
        "INSERT INTO commands(id,fingerprint,state,intent,sequence) VALUES('pending','fixture-fingerprint','intent',?1,1)",
        [b"pending bytes".as_slice()],
    )
    .unwrap();
    drop(db);
    let before = fs::read(path.join("storage.sqlite")).unwrap();
    assert!(Store::inspect_quiescent(&path, "parent").is_err());
    assert_eq!(fs::read(path.join("storage.sqlite")).unwrap(), before);
    let db = rusqlite::Connection::open_with_flags(
        path.join("storage.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(
        db.query_row("SELECT state FROM commands WHERE id='pending'", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "intent"
    );
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

#[test]
fn native_artifact_catalog_pages_are_complete_ordered_and_requester_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("store")).unwrap();
    seed(&store);
    for index in 0..150 {
        store.put_artifact(&artifact(index, "utf8")).unwrap();
    }
    store.put_artifact(&artifact(200, "binary")).unwrap();
    store
        .create_session("stranger", &header("stranger"))
        .unwrap();
    assert!(
        store
            .artifact_catalog("stranger", None, 128, None)
            .unwrap()
            .is_empty()
    );
    let mut before = None;
    let mut records = vec![];
    loop {
        let page = store
            .artifact_catalog("parent", before.as_ref(), 17, Some("text"))
            .unwrap();
        assert!(page.len() <= 17);
        if page.is_empty() {
            break;
        }
        before = Some(page.last().unwrap().position.clone());
        records.extend(page);
    }
    assert_eq!(records.len(), 150);
    for (record, index) in records.iter().zip((0..150).rev()) {
        assert_eq!(record.position.id, format!("art_{index:032x}"));
        assert_eq!(record.position.origin, "origin");
        assert_eq!(record.record["unknown"]["preserved"], true);
    }
    assert!(store.artifact_catalog("parent", None, 129, None).is_err());
    assert_eq!(
        store.artifact_catalog("parent", None, 1, None).unwrap()[0]
            .position
            .id,
        format!("art_{:032x}", 200)
    );
    store.fork("parent", "child", None).unwrap();
    store.put_artifact(&artifact(300, "utf8")).unwrap();
    assert!(
        store
            .artifact_catalog("child", None, 128, None)
            .unwrap()
            .iter()
            .all(|record| record.position.id != format!("art_{:032x}", 300))
    );
    store.archive_session("parent").unwrap();
    assert!(store.artifact_catalog("parent", None, 1, None).is_err());
    assert_eq!(
        store
            .artifact_catalog("child", None, 1, None)
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn native_context_cursors_match_sdk_bytes_and_keep_authorized_fork_access() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store");
    let store = Store::open(&path).unwrap();
    seed(&store);
    let id = store.put_artifact(&artifact(1, "utf8")).unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/artifact-cursor-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    for row in rows {
        let cursor = store
            .context_cursor(
                "parent",
                "origin",
                &id,
                row["start"].as_u64().unwrap()..row["end"].as_u64().unwrap(),
                row["anchor"].as_u64().unwrap() as u32,
            )
            .unwrap();
        assert_eq!(cursor, row["cursor"]);
        let decoded = store.decode_cursor("parent", &cursor).unwrap();
        assert!(decoded.context);
        assert_eq!(
            decoded.anchor_line,
            Some(row["anchor"].as_u64().unwrap() as u32)
        );
    }
    let cursor = store
        .context_cursor(
            "parent",
            "origin",
            &id,
            0..artifact(1, "utf8").bytes.len() as u64,
            2,
        )
        .unwrap();
    store
        .create_session("stranger", &header("stranger"))
        .unwrap();
    assert!(store.read_cursor("stranger", &cursor).is_err());
    assert!(
        store
            .context_cursor("stranger", "origin", &id, 0..20, 2)
            .is_err()
    );
    store.fork("parent", "child", None).unwrap();
    store.archive_session("parent").unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let page = store.read_cursor("child", &cursor).unwrap();
    assert_eq!(page.bytes, artifact(1, "utf8").bytes);
    assert_eq!(page.next_offset, None);
    let mut forged = cursor.into_bytes();
    forged[9] = if forged[9] == b'A' { b'B' } else { b'A' };
    assert!(
        store
            .read_cursor("child", &String::from_utf8(forged).unwrap())
            .is_err()
    );
}
#[test]
fn native_manual_lease_release_accepts_lease_or_artifact_id_and_preserves_foreign_leases() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open_with(
        &directory.path().join("store"),
        Arc::new(TestClock(AtomicU64::new(1000))),
        Arc::new(Fault {
            point: Boundary::BeforeAck,
            armed: AtomicBool::new(false),
        }),
    )
    .unwrap();
    seed(&store);
    let id = store.put_artifact(&artifact(1, "utf8")).unwrap();
    assert!(
        store
            .lease_record("parent", "origin", &id, &"🦙".repeat(121), 120000, true)
            .is_err()
    );
    let record = store
        .lease_record(
            "parent",
            "origin",
            &id,
            " retain decisive evidence ",
            120000,
            true,
        )
        .unwrap();
    assert_eq!(record["reason"], "retain decisive evidence");
    assert_eq!(record["expiresAt"], "1970-01-01T00:02:01.000Z");
    assert_eq!(
        store.evidence_lease_records("parent").unwrap(),
        vec![record.clone()]
    );
    store
        .create_session("stranger", &header("stranger"))
        .unwrap();
    assert_eq!(
        store
            .release_evidence_lease("stranger", record["id"].as_str().unwrap())
            .unwrap(),
        0
    );
    assert!(store.evidence_lease_records("stranger").unwrap().is_empty());
    assert_eq!(store.evidence_lease_records("parent").unwrap().len(), 1);
    assert_eq!(
        store
            .release_evidence_lease("parent", record["id"].as_str().unwrap())
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .release_evidence_lease("parent", record["id"].as_str().unwrap())
            .unwrap(),
        0
    );
    store
        .lease_record("parent", "origin", &id, "renewed", 120000, false)
        .unwrap();
    assert_eq!(store.release_evidence_lease("parent", &id).unwrap(), 1);
    assert!(store.evidence_lease_records("parent").unwrap().is_empty());
    assert_eq!(
        store.page("parent", "origin", &id, 0, 32768).unwrap().bytes,
        artifact(1, "utf8").bytes
    );
}
#[test]
fn native_lease_creation_and_release_preserve_committed_state_when_ack_is_lost() {
    for point in [Boundary::BeforeCommit, Boundary::BeforeAck] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(Fault {
            point,
            armed: AtomicBool::new(false),
        });
        let store = Store::open_with(
            &directory.path().join("store"),
            Arc::new(TestClock(AtomicU64::new(1000))),
            fault.clone(),
        )
        .unwrap();
        seed(&store);
        let id = store.put_artifact(&artifact(1, "utf8")).unwrap();
        fault.armed.store(true, Ordering::SeqCst);
        assert!(
            store
                .lease_record("parent", "origin", &id, "faulted creation", 60000, true)
                .is_err()
        );
        assert_eq!(
            store.evidence_lease_records("parent").unwrap().len(),
            usize::from(point == Boundary::BeforeAck)
        );
        store
            .lease_record("parent", "origin", &id, "ready for release", 60000, true)
            .unwrap();
        fault.armed.store(true, Ordering::SeqCst);
        assert!(store.release_evidence_lease("parent", &id).is_err());
        assert_eq!(
            store.evidence_lease_records("parent").unwrap().len(),
            usize::from(point == Boundary::BeforeCommit)
        );
    }
}

#[test]
fn native_artifact_scan_authorizes_and_hashes_the_same_bytes_before_returning_matches() {
    use ag1_core::{SessionId, evidence::EvidenceRequest};
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("store")).unwrap();
    seed(&store);
    let input = artifact(1, "utf8");
    let id = store.put_artifact(&input).unwrap();
    let request = EvidenceRequest {
        requester: SessionId::new("parent").unwrap(),
        origin: "origin".into(),
        artifact: id.clone(),
    };
    let result = store
        .scan_artifact(&request, "needle", true, 10, || Ok(()))
        .unwrap();
    assert_eq!(result.total_matches, 1);
    assert_eq!(result.hits[0].line, 2);
    assert_eq!(result.hits[0].text, "needle β 🦙 final");
    assert_eq!(result.sha256, digest(&input.bytes));
    let hit = &result.hits[0];
    let cursor = store
        .context_cursor(
            "parent",
            "origin",
            &id,
            hit.context_start..hit.context_end,
            hit.line as u32,
        )
        .unwrap();
    assert_eq!(
        store.read_cursor("parent", &cursor).unwrap().bytes,
        input.bytes
    );
    store
        .create_session("stranger", &header("stranger"))
        .unwrap();
    let foreign = EvidenceRequest {
        requester: SessionId::new("stranger").unwrap(),
        ..request.clone()
    };
    assert!(
        store
            .scan_artifact(&foreign, "needle", true, 10, || Ok(()))
            .is_err()
    );
    let binary = store.put_artifact(&artifact(2, "binary")).unwrap();
    assert!(
        store
            .scan_artifact(
                &EvidenceRequest {
                    artifact: binary,
                    ..request.clone()
                },
                "needle",
                true,
                10,
                || Ok(())
            )
            .is_err()
    );
    // Corrupt bytes still contain a match. No match may escape the final hash
    // check, even when the caller already knows an authorized artifact id.
    fs::write(
        store.blob_path(&digest(&input.bytes)).unwrap(),
        b"tampered needle\n",
    )
    .unwrap();
    assert!(
        store
            .scan_artifact(&request, "needle", true, 10, || Ok(()))
            .is_err()
    );
    assert!(store.artifact_record("parent", "origin", &id).is_ok());
}
#[test]
fn native_artifact_scan_cancels_between_reads_and_rechecks_access_before_publication() {
    use ag1_core::{DomainError, SessionId, evidence::EvidenceRequest};
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("store")).unwrap();
    seed(&store);
    let mut input = artifact(1, "utf8");
    input.bytes = format!("{} needle\n", "x".repeat(2100000)).into_bytes();
    let mut record: Value = serde_json::from_slice(&input.record).unwrap();
    record["bytes"] = json!(input.bytes.len());
    record["sha256"] = json!(digest(&input.bytes));
    input.record = serde_json::to_vec(&record).unwrap();
    let id = store.put_artifact(&input).unwrap();
    let request = EvidenceRequest {
        requester: SessionId::new("parent").unwrap(),
        origin: "origin".into(),
        artifact: id,
    };
    let mut checks = 0;
    let cancelled = store.scan_artifact(&request, "needle", true, 1, || {
        checks += 1;
        if checks == 3 {
            Err(DomainError::Cancelled)
        } else {
            Ok(())
        }
    });
    assert!(matches!(cancelled, Err(DomainError::Cancelled)));
    assert_eq!(checks, 3);
    let full = store
        .scan_artifact(&request, "needle", true, 1, || Ok(()))
        .unwrap();
    assert_eq!(full.total_matches, 1);
    assert_eq!(full.hits[0].text.len(), 4000);
    assert_eq!(full.bytes, input.bytes.len() as u64);
    let mut checks = 0;
    let revoked = store.scan_artifact(&request, "needle", true, 1, || {
        checks += 1;
        if checks == 2 {
            store.archive_session("parent").unwrap();
        }
        Ok(())
    });
    assert!(revoked.is_err());
}

#[test]
fn native_catalog_search_matches_sdk_deduplication_salience_counts_and_limits() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("store")).unwrap());
    seed(&store);
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/artifact-search-oracle.json"
    ))
    .unwrap();
    for input in fixture["inputs"].as_array().unwrap() {
        store
            .put_artifact(&ArtifactInput {
                origin: "origin".into(),
                session: "parent".into(),
                record: serde_json::to_vec(&input["record"]).unwrap(),
                bytes: input["text"].as_str().unwrap().as_bytes().to_vec(),
            })
            .unwrap();
    }
    let rows = fixture["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 5);
    for row in rows {
        let snapshot = store
            .capture_artifact_search("parent", row["kind"].as_str())
            .unwrap();
        let result = snapshot
            .search(
                row["query"].as_str().unwrap(),
                row["ignoreCase"].as_bool().unwrap_or(true),
                row["maxArtifacts"].as_u64().unwrap_or(12) as usize,
                row["maxMatchesPerArtifact"].as_u64().unwrap_or(3) as usize,
                || Ok(()),
            )
            .unwrap();
        let matches=result.matches.iter().map(|matched|json!({"id":matched.record.position.id,"occurrenceCount":matched.occurrence_count,"totalMatches":matched.total_matches,"hits":matched.hits.iter().map(|hit|json!({"line":hit.line,"text":hit.text,"byteOffset":hit.byte_offset})).collect::<Vec<_>>()})).collect::<Vec<_>>();
        assert_eq!(json!(matches), row["matches"], "{}", row["id"]);
        assert_eq!(json!(result.scanned_artifacts), row["scannedArtifacts"]);
        assert_eq!(json!(result.truncated), row["truncated"]);
        // The current native scanner reports actual full scans. An optional
        // lexical skip index is not being fabricated in these metrics.
        assert_eq!(result.candidate_artifacts, result.scanned_artifacts);
        assert!(result.scanned_blobs < result.scanned_artifacts);
    }
}
#[test]
fn native_catalog_search_snapshot_excludes_later_inserts_and_releases_its_scratch_table() {
    use ag1_core::DomainError;
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("store")).unwrap());
    seed(&store);
    store.put_artifact(&artifact(1, "utf8")).unwrap();
    let snapshots = (0..8)
        .map(|_| store.capture_artifact_search("parent", None).unwrap())
        .collect::<Vec<_>>();
    assert!(store.capture_artifact_search("parent", None).is_err());
    store.put_artifact(&artifact(2, "utf8")).unwrap();
    let result = snapshots[0]
        .search("needle", true, 12, 3, || Ok(()))
        .unwrap();
    assert_eq!(result.scanned_artifacts, 1);
    assert_eq!(result.matches.len(), 1);
    assert_eq!(
        result.matches[0].record.position.id,
        format!("art_{:032x}", 1)
    );
    assert_eq!(result.matches[0].occurrence_count, 1);
    assert!(matches!(
        snapshots[0].search("needle", true, 12, 3, || Err(DomainError::Cancelled)),
        Err(DomainError::Cancelled)
    ));
    drop(snapshots);
    for _ in 0..12 {
        let snapshot = store.capture_artifact_search("parent", None).unwrap();
        let result = snapshot.search("needle", true, 12, 3, || Ok(())).unwrap();
        assert_eq!(result.scanned_artifacts, 2);
        assert_eq!(result.scanned_blobs, 1);
        assert_eq!(
            result.matches[0].record.position.id,
            format!("art_{:032x}", 2)
        );
        assert_eq!(result.matches[0].occurrence_count, 2);
    }
    let snapshot = store.capture_artifact_search("parent", None).unwrap();
    store.archive_session("parent").unwrap();
    assert!(snapshot.search("needle", true, 12, 3, || Ok(())).is_err());
}

#[test]
fn native_catalog_search_ranks_across_all_pages_without_exposing_foreign_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("store")).unwrap());
    seed(&store);
    store
        .create_session("stranger", &header("stranger"))
        .unwrap();
    store
        .register_origin("foreign", "stranger", &[9; 32])
        .unwrap();
    let mut expected_bytes = 0;
    for id in 1..=65 {
        let mut input = artifact(id, "utf8");
        // The best match has the oldest catalog position and is reached only
        // after four complete candidate pages. Each blob has distinct content.
        input.bytes = format!(
            "record {id}\n{}",
            "needle\n".repeat(if id == 1 { 7 } else { 1 })
        )
        .into_bytes();
        let mut record: Value = serde_json::from_slice(&input.record).unwrap();
        record["sha256"] = json!(digest(&input.bytes));
        record["bytes"] = json!(input.bytes.len());
        if id == 65 {
            input.origin = "foreign".into();
            input.session = "stranger".into();
            record["sessionId"] = json!("stranger");
            record["salientLines"] = json!(["needle"]);
        } else {
            expected_bytes += input.bytes.len() as u64;
        }
        input.record = serde_json::to_vec(&record).unwrap();
        store.put_artifact(&input).unwrap();
    }
    let snapshot = store.capture_artifact_search("parent", None).unwrap();
    let result = snapshot.search("needle", true, 2, 3, || Ok(())).unwrap();
    assert_eq!(result.scanned_artifacts, 64);
    assert_eq!(result.scanned_blobs, 64);
    assert_eq!(result.bytes_scanned, expected_bytes);
    assert!(result.truncated);
    assert_eq!(result.matches.len(), 2);
    assert_eq!(
        result.matches[0].record.position.id,
        format!("art_{:032x}", 1)
    );
    assert_eq!(result.matches[0].total_matches, 7);
    assert_eq!(result.matches[0].hits.len(), 3);
    assert_eq!(
        result.matches[1].record.position.id,
        format!("art_{:032x}", 64)
    );
}

#[test]
fn evidence_catalog_filters_only_native_custody_and_keeps_direct_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(&directory.path().join("store")).unwrap());
    seed(&store);
    let kinds = [
        "tool-arguments",
        "tool-rejected-arguments",
        "tool-original-result",
        "tool-post-hook-result",
        "tool-native-result",
        "tool-request-result",
        "runtime-provider-output",
        "runtime-request-input",
        "context-original-journal",
        "context-original-request",
        "text",
        "compaction-trajectory",
        "future-evidence-kind",
    ];
    for (index, kind) in kinds.iter().enumerate() {
        let mut input = artifact(index + 1, "utf8");
        let mut record: Value = serde_json::from_slice(&input.record).unwrap();
        record["kind"] = json!(kind);
        input.record = serde_json::to_vec(&record).unwrap();
        let id = store.put_artifact(&input).unwrap();
        assert_eq!(store.artifact_origin("parent", &id).unwrap(), "origin");
        assert!(store.page("parent", "origin", &id, 0, 32768).is_ok());
    }
    assert_eq!(
        store
            .artifact_catalog("parent", None, 100, None)
            .unwrap()
            .len(),
        kinds.len()
    );
    let visible = store.evidence_catalog("parent", 100).unwrap();
    assert_eq!(visible.len(), 3);
    assert_eq!(visible[0].record["kind"], "future-evidence-kind");
    let result = store
        .capture_evidence_search("parent", None)
        .unwrap()
        .search("needle", true, 12, 3, || Ok(()))
        .unwrap();
    assert_eq!(result.scanned_artifacts, 3);
    assert_eq!(result.matches[0].occurrence_count, 3);
    // Raw custody remains searchable explicitly by the host storage API.
    let all = store
        .capture_artifact_search("parent", None)
        .unwrap()
        .search("needle", true, 12, 3, || Ok(()))
        .unwrap();
    assert_eq!(all.scanned_artifacts, kinds.len() as u64);
}

#[cfg(target_os = "linux")]
#[test]
fn writer_release_is_explicit_even_if_a_spawn_inherits_the_open_description() {
    use std::os::fd::BorrowedFd;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store");
    let store = Store::open(&path).unwrap();
    let lock = path.join("writer.lock");
    let descriptor = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| std::fs::read_link(entry.path()).is_ok_and(|target| target == lock))
        .unwrap();
    let raw: i32 = descriptor.file_name().to_str().unwrap().parse().unwrap();
    // The store remains alive, so this FD cannot be closed/reused while borrowed.
    // dup retains the same open description, as an unrelated fork does until exec.
    let inherited = rustix::io::dup(unsafe { BorrowedFd::borrow_raw(raw) }).unwrap();
    assert!(Store::open(&path).is_err());
    drop(store);
    let reopened =
        Store::open(&path).expect("owner teardown must release its lock before unrelated exec");
    assert!(Store::open(&path).is_err());
    drop(inherited);
    assert!(
        Store::open(&path).is_err(),
        "closing an old description must not unlock the new owner"
    );
    drop(reopened);
    assert!(Store::open(&path).is_ok());
}
