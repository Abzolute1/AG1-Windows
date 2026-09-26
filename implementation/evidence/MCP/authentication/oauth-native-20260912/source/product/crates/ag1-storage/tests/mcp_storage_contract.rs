use ag1_core::Clock;
use ag1_storage::{Boundary, FaultInjector, StorageError, StorageResult, Store};
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
            return Err(StorageError::Injected(boundary));
        }
        Ok(())
    }
}
#[test]
fn state_cas_restart_reserved_namespace_and_bounded_retention() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        assert!(store.mcp_state().unwrap().is_none());
        assert!(matches!(
            store.save_metadata("mcp-manager", "state", None, b"{}"),
            Err(StorageError::Denied)
        ));
        assert_eq!(store.commit_mcp_state(0, br#"{"servers":{}}"#).unwrap(), 1);
        assert!(matches!(
            store.commit_mcp_state(0, b"{}"),
            Err(StorageError::Conflict(_))
        ));
        for rev in 1..20 {
            store
                .commit_mcp_state(rev, br#"{"servers":{},"test":true}"#)
                .unwrap();
        }
        assert_eq!(store.mcp_state().unwrap().unwrap().revision, 20);
        assert_eq!(store.inspect_integrity().unwrap().orphan_blobs, 0);
        assert_eq!(
            std::fs::read_dir(dir.path().join("blobs")).unwrap().count(),
            1
        );
        assert!(
            store
                .commit_mcp_state(20, &vec![b' '; 1024 * 1024 + 1])
                .is_err()
        );
        assert_eq!(store.mcp_state().unwrap().unwrap().revision, 20);
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.mcp_state().unwrap().unwrap().revision, 20);
}
#[test]
fn failure_before_commit_rolls_back_and_after_commit_requires_readback() {
    for (boundary, expected) in [
        (Boundary::BeforeCommit, 1),
        (Boundary::AfterCommit, 2),
        (Boundary::BeforeAck, 2),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fault = Arc::new(Fault {
            boundary,
            armed: AtomicBool::new(false),
        });
        let store = Store::open_with(dir.path(), Arc::new(TestClock), fault.clone()).unwrap();
        store.commit_mcp_state(0, br#"{"value":"before"}"#).unwrap();
        fault.armed.store(true, Ordering::Release);
        assert!(store.commit_mcp_state(1, br#"{"value":"after"}"#).is_err());
        assert_eq!(store.mcp_state().unwrap().unwrap().revision, expected);
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.mcp_state().unwrap().unwrap().revision, expected);
        reopened.inspect_integrity().unwrap();
    }
}
