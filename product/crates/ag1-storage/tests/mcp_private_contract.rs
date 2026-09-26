use ag1_core::{Clock, SecretBytes};
use ag1_storage::{
    Boundary, FaultInjector, StorageError, StorageResult, Store,
    mcp_private::{MAX_BYTES, PrivateStore},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        1
    }
}
struct Fault {
    boundary: Boundary,
    armed: AtomicBool,
}
impl FaultInjector for Fault {
    fn check(&self, boundary: Boundary) -> StorageResult<()> {
        if boundary == self.boundary && self.armed.swap(false, Ordering::AcqRel) {
            Err(StorageError::Injected(boundary))
        } else {
            Ok(())
        }
    }
}
#[test]
fn private_replacement_has_no_public_blob_metadata_or_history_and_preserves_cas() {
    let dir = tempfile::tempdir().unwrap();
    let private = PrivateStore::open(dir.path()).unwrap();
    assert!(PrivateStore::open(dir.path()).is_err());
    assert!(private.load().unwrap().is_none());
    let token = SecretBytes::new(b"fixture-private-token".to_vec());
    assert_eq!(private.commit(0, &token).unwrap(), 1);
    assert!(matches!(
        private.commit(0, &token),
        Err(StorageError::Conflict(_))
    ));
    let saved = private.load().unwrap().unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(saved.bytes.expose(), token.expose());
    drop(saved);
    assert_eq!(
        private
            .commit(1, &SecretBytes::new(b"revoked-tombstone".to_vec()))
            .unwrap(),
        2
    );
    assert!(
        private
            .commit(2, &SecretBytes::new(vec![0; MAX_BYTES + 1]))
            .is_err()
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0
    );
    drop(private);
    let db = rusqlite::Connection::open(dir.path().join("storage.sqlite")).unwrap();
    for table in ["metadata", "artifacts", "commands", "entries"] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    let raw: Vec<u8> = db
        .query_row("SELECT raw FROM mcp_private_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(raw, b"revoked-tombstone");
    drop(db);
    let reopened = PrivateStore::open(dir.path()).unwrap();
    assert_eq!(reopened.load().unwrap().unwrap().revision, 2);
}
#[test]
fn uncertain_private_ack_requires_reopen_and_never_replays_old_revision() {
    for boundary in [
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fault = Arc::new(Fault {
            boundary,
            armed: AtomicBool::new(false),
        });
        let private = PrivateStore::from_store(
            Store::open_with(dir.path(), Arc::new(TestClock), fault.clone()).unwrap(),
        )
        .unwrap();
        private
            .commit(0, &SecretBytes::new(b"old".to_vec()))
            .unwrap();
        fault.armed.store(true, Ordering::Release);
        assert!(
            private
                .commit(1, &SecretBytes::new(b"new".to_vec()))
                .is_err()
        );
        drop(private);
        let private = PrivateStore::open(dir.path()).unwrap();
        let saved = private.load().unwrap().unwrap();
        let committed = boundary != Boundary::BeforeCommit;
        assert_eq!(saved.revision, if committed { 2 } else { 1 });
        assert_eq!(
            saved.bytes.expose(),
            if committed { b"new" } else { b"old" }
        );
        if committed {
            assert!(matches!(
                private.commit(1, &SecretBytes::new(b"stale".to_vec())),
                Err(StorageError::Conflict(_))
            ));
        }
    }
}
