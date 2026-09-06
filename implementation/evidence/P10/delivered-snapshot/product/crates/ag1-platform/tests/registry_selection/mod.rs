use super::*;
use ag1_storage::{Boundary, FaultInjector, SelectionChange, StorageError, StorageResult};

struct SelectionFault {
    boundary: Boundary,
    armed: AtomicBool,
}
impl FaultInjector for SelectionFault {
    fn check(&self, _: Boundary) -> StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(&self, command: &str, boundary: Boundary) -> StorageResult<()> {
        if command.starts_with("select-")
            && boundary == self.boundary
            && self.armed.swap(false, Ordering::AcqRel)
        {
            return Err(StorageError::Injected(boundary));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_initialization_and_switch_activate_only_after_selection_custody() {
    let f = Fixture::new();
    let (first, resolver) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
    assert!(registry.selected().is_err());
    assert!(!old.accepts_commands().unwrap());
    assert!(
        f.store
            .runtime_selection(f.root.id().as_str())
            .unwrap()
            .is_none()
    );
    let pending = registry.pending_selection().unwrap().unwrap();
    assert_eq!(pending.selection.session, "first");
    let first = registry.reconcile_selection().await.unwrap();
    assert_eq!(first.epoch, 1);
    assert!(old.accepts_commands().unwrap());
    assert!(registry.pending_selection().unwrap().is_none());
    let initial = f
        .store
        .runtime_selection(f.root.id().as_str())
        .unwrap()
        .unwrap();
    assert_eq!(initial.selection, pending.selection);
    let (second, next_resolver) = f.prepared("second", 2, false);
    let next = second.runtime().clone();
    let selected = registry
        .replace(1, f.target("second"), &Factory::new(Some(second), false))
        .await
        .unwrap();
    assert_eq!(selected.epoch, 2);
    assert!(!old.accepts_commands().unwrap());
    assert!(next.accepts_commands().unwrap());
    let saved = f
        .store
        .runtime_selection(f.root.id().as_str())
        .unwrap()
        .unwrap();
    assert_eq!(saved.selection.session, "second");
    assert_eq!(saved.selection.epoch, 2);
    assert_ne!(saved.selection.owner, initial.selection.owner);
    assert_eq!(registry.reconcile_selection().await.unwrap().epoch, 2);
    assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
    assert_eq!(next_resolver.calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_and_lost_selection_ack_keep_both_owners_frozen_until_exact_reconciliation() {
    for boundary in [Boundary::BeforeCommit, Boundary::BeforeAck] {
        let fault = Arc::new(SelectionFault {
            boundary,
            armed: AtomicBool::new(false),
        });
        let f = Fixture::with_fault(Some(fault.clone()));
        let (first, resolver) = f.prepared("first", 1, false);
        let old = first.runtime().clone();
        let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
        registry.reconcile_selection().await.unwrap();
        let (second, next_resolver) = f.prepared("second", 2, false);
        let next = second.runtime().clone();
        fault.armed.store(true, Ordering::Release);
        assert!(
            registry
                .replace(1, f.target("second"), &Factory::new(Some(second), false))
                .await
                .is_err()
        );
        assert!(!fault.armed.load(Ordering::Acquire));
        assert!(registry.selected().is_err());
        assert!(!old.accepts_commands().unwrap());
        assert!(!next.accepts_commands().unwrap());
        let pending = registry.pending_selection().unwrap().unwrap();
        let before = f.store.command_result(&pending.selection.command).unwrap();
        assert_eq!(before.is_some(), boundary == Boundary::BeforeAck);
        let selected = f
            .store
            .runtime_selection(f.root.id().as_str())
            .unwrap()
            .unwrap();
        assert_eq!(
            selected.selection.session,
            if boundary == Boundary::BeforeAck {
                "second"
            } else {
                "first"
            }
        );
        assert!(
            registry
                .replace(1, f.target("first"), &Factory::new(None, false))
                .await
                .is_err()
        );
        assert_eq!(registry.reconcile_selection().await.unwrap().epoch, 2);
        assert!(registry.pending_selection().unwrap().is_none());
        assert!(!old.accepts_commands().unwrap());
        assert!(next.accepts_commands().unwrap());
        let after = f
            .store
            .command_result(&pending.selection.command)
            .unwrap()
            .unwrap();
        if let Some(before) = before {
            assert_eq!(before.result, after.result);
            assert_eq!(before.sequence, after.sequence);
        }
        assert_eq!(
            f.store
                .runtime_selection(f.root.id().as_str())
                .unwrap()
                .unwrap()
                .selection,
            pending.selection
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
        assert_eq!(next_resolver.calls.load(Ordering::Acquire), 0);
    }
}

struct SelectionGate(Arc<AckGate>);
impl FaultInjector for SelectionGate {
    fn check(&self, _: Boundary) -> StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(&self, command: &str, boundary: Boundary) -> StorageResult<()> {
        if command.starts_with("select-") {
            self.0.check(boundary)?;
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_selection_awaiter_or_registry_cannot_thaw_an_owner_during_an_accepted_commit() {
    for drop_registry in [false, true] {
        let gate = Arc::new(AckGate::default());
        let f = Fixture::with_fault(Some(Arc::new(SelectionGate(gate.clone()))));
        let (first, resolver) = f.prepared("first", 1, false);
        let old = first.runtime().clone();
        let registry =
            Arc::new(RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap());
        registry.reconcile_selection().await.unwrap();
        let (second, next_resolver) = f.prepared("second", 2, false);
        let next = second.runtime().clone();
        gate.armed.store(true, Ordering::Release);
        let owner = registry.clone();
        let target = f.target("second");
        let task = tokio::spawn(async move {
            owner
                .replace(1, target, &Factory::new(Some(second), false))
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        assert!(registry.selected().is_err());
        assert!(!old.accepts_commands().unwrap());
        assert!(!next.accepts_commands().unwrap());
        assert!(matches!(
            registry.reconcile_selection().await,
            Err(DomainError::Conflict)
        ));
        let pending = registry.pending_selection().unwrap().unwrap();
        if drop_registry {
            drop(registry);
            gate.release();
            // A barrier on the same single-worker pool proves accepted SQL and
            // the last retained fence owner have finished dropping.
            f.pool.submit(|| Ok(())).await.unwrap();
            assert!(!old.accepts_commands().unwrap());
            assert!(!next.accepts_commands().unwrap());
        } else {
            gate.release();
            f.pool.submit(|| Ok(())).await.unwrap();
            assert!(!old.accepts_commands().unwrap());
            assert!(!next.accepts_commands().unwrap());
            registry.reconcile_selection().await.unwrap();
            assert!(next.accepts_commands().unwrap());
        }
        assert_eq!(
            f.store
                .runtime_selection(f.root.id().as_str())
                .unwrap()
                .unwrap()
                .selection,
            pending.selection
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
        assert_eq!(next_resolver.calls.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discarded_uncertain_selection_resumes_current_durable_session_with_a_fresh_owner() {
    for boundary in [Boundary::BeforeCommit, Boundary::BeforeAck] {
        let fault = Arc::new(SelectionFault {
            boundary,
            armed: AtomicBool::new(false),
        });
        let f = Fixture::with_fault(Some(fault.clone()));
        let (first, _) = f.prepared("first", 1, false);
        let old = first.runtime().clone();
        let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
        registry.reconcile_selection().await.unwrap();
        let (second, _) = f.prepared("second", 2, false);
        let next = second.runtime().clone();
        fault.armed.store(true, Ordering::Release);
        assert!(
            registry
                .replace(1, f.target("second"), &Factory::new(Some(second), false))
                .await
                .is_err()
        );
        drop(registry);
        assert!(!old.accepts_commands().unwrap());
        assert!(!next.accepts_commands().unwrap());
        let previous = f
            .store
            .runtime_selection(f.root.id().as_str())
            .unwrap()
            .unwrap();
        let (fresh, resolver) = f.prepared(
            &previous.selection.session,
            previous.selection.epoch + 1,
            false,
        );
        let runtime = fresh.runtime().clone();
        let registry =
            RuntimeRegistry::durable(fresh, Some(previous.clone()), "select-resumed".into())
                .unwrap();
        assert!(!runtime.accepts_commands().unwrap());
        registry.reconcile_selection().await.unwrap();
        let current = f
            .store
            .runtime_selection(f.root.id().as_str())
            .unwrap()
            .unwrap();
        assert_eq!(current.selection.session, previous.selection.session);
        assert_eq!(current.selection.epoch, previous.selection.epoch + 1);
        assert_ne!(current.selection.owner, previous.selection.owner);
        assert_eq!(
            current.selection.journal_revision,
            previous.selection.journal_revision
        );
        assert!(runtime.accepts_commands().unwrap());
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn historical_committed_selection_receipt_cannot_reactivate_a_superseded_owner() {
    let fault = Arc::new(SelectionFault {
        boundary: Boundary::BeforeAck,
        armed: AtomicBool::new(false),
    });
    let f = Fixture::with_fault(Some(fault.clone()));
    let (first, _) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
    registry.reconcile_selection().await.unwrap();
    let (second, _) = f.prepared("second", 2, false);
    let next = second.runtime().clone();
    fault.armed.store(true, Ordering::Release);
    assert!(
        registry
            .replace(1, f.target("second"), &Factory::new(Some(second), false))
            .await
            .is_err()
    );
    let pending = registry.pending_selection().unwrap().unwrap();
    let saved = f
        .store
        .runtime_selection(f.root.id().as_str())
        .unwrap()
        .unwrap();
    let mut later = SelectionChange {
        expected: Some(saved.sha256),
        selection: saved.selection,
    };
    later.selection.epoch += 1;
    later.selection.owner = "another-owner".into();
    later.selection.command = "select-later".into();
    let later = f.store.commit_runtime_selection(&later).unwrap();
    assert_eq!(
        f.store
            .commit_runtime_selection(&pending)
            .unwrap()
            .selection,
        pending.selection
    );
    assert!(registry.reconcile_selection().await.is_err());
    assert!(registry.selected().is_err());
    assert!(!old.accepts_commands().unwrap());
    assert!(!next.accepts_commands().unwrap());
    assert_eq!(
        f.store
            .runtime_selection(f.root.id().as_str())
            .unwrap()
            .unwrap()
            .selection,
        later.selection
    );
}
