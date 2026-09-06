use super::*;
use ag1_storage::{RuntimeSelection, SelectionChange};

fn selection(
    command: &str,
    session: &str,
    epoch: u64,
    expected: Option<String>,
) -> SelectionChange {
    SelectionChange {
        expected,
        selection: RuntimeSelection {
            version: 1,
            workspace: "workspace".into(),
            session: session.into(),
            epoch,
            owner: format!("owner-{epoch}"),
            journal_revision: 0,
            command: command.into(),
        },
    }
}

#[test]
fn selected_owner_and_receipt_are_atomic_scoped_and_old_retries_never_revert_a_later_selection() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    create(&store, "first");
    create(&store, "second");
    let first = selection("select-first", "first", 1, None);
    let first_ack = store.commit_runtime_selection(&first).unwrap();
    let second = selection("select-second", "second", 2, Some(first_ack.sha256.clone()));
    let second_ack = store.commit_runtime_selection(&second).unwrap();
    let third = selection("select-third", "first", 3, Some(second_ack.sha256.clone()));
    let third_ack = store.commit_runtime_selection(&third).unwrap();
    assert_eq!(store.commit_runtime_selection(&second).unwrap(), second_ack);
    assert_eq!(
        store
            .runtime_selection("workspace")
            .unwrap()
            .unwrap()
            .sha256,
        third_ack.sha256
    );
    assert!(
        store
            .runtime_selection("another-workspace")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .save_metadata(
                "native-runtime-selection",
                "workspace",
                Some(&third_ack.sha256),
                b"{}"
            )
            .is_err()
    );
    assert!(
        store
            .commit_runtime_selection(&selection("stale", "second", 4, Some(first_ack.sha256)))
            .is_err()
    );
    for (name, mut invalid) in [
        (
            "skipped",
            selection("skipped", "second", 5, Some(third_ack.sha256.clone())),
        ),
        (
            "target-revision",
            selection(
                "target-revision",
                "second",
                4,
                Some(third_ack.sha256.clone()),
            ),
        ),
        (
            "same-owner",
            selection("same-owner", "second", 4, Some(third_ack.sha256.clone())),
        ),
        (
            "missing-session",
            selection(
                "missing-session",
                "absent",
                4,
                Some(third_ack.sha256.clone()),
            ),
        ),
    ] {
        if name == "target-revision" {
            invalid.selection.journal_revision = 1;
        }
        if name == "same-owner" {
            invalid.selection.owner = third.selection.owner.clone();
        }
        assert!(store.commit_runtime_selection(&invalid).is_err());
        assert!(store.command_result(name).unwrap().is_none());
    }
    let mut reused = third.clone();
    reused.selection.session = "second".into();
    assert!(store.commit_runtime_selection(&reused).is_err());
    assert_eq!(
        store
            .runtime_selection("workspace")
            .unwrap()
            .unwrap()
            .selection,
        third.selection
    );
    let receipt = store.command_result("select-third").unwrap().unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(
        serde_json::from_slice::<ag1_storage::SelectionAck>(receipt.result.as_ref().unwrap())
            .unwrap(),
        third_ack
    );
}

struct SelectionFault {
    point: Boundary,
    armed: AtomicBool,
}
impl FaultInjector for SelectionFault {
    fn check(&self, _: Boundary) -> StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(&self, command: &str, point: Boundary) -> StorageResult<()> {
        if command == "select-next"
            && point == self.point
            && self.armed.swap(false, Ordering::AcqRel)
        {
            return Err(StorageError::Injected(point));
        }
        Ok(())
    }
}

#[test]
fn selection_failure_and_lost_acknowledgement_reopen_as_one_complete_choice_with_an_immutable_receipt()
 {
    for point in [
        Boundary::BeforeTransaction,
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(SelectionFault {
            point,
            armed: AtomicBool::new(false),
        });
        let store = Store::open_with(directory.path(), Arc::new(TestClock), fault.clone()).unwrap();
        create(&store, "first");
        create(&store, "second");
        let initial = store
            .commit_runtime_selection(&selection("select-initial", "first", 1, None))
            .unwrap();
        let next = selection("select-next", "second", 2, Some(initial.sha256.clone()));
        fault.armed.store(true, Ordering::Release);
        assert!(store.commit_runtime_selection(&next).is_err());
        assert!(!fault.armed.load(Ordering::Acquire));
        let committed = matches!(point, Boundary::AfterCommit | Boundary::BeforeAck);
        let observed = store.runtime_selection("workspace").unwrap().unwrap();
        assert_eq!(
            observed.selection,
            if committed {
                next.selection.clone()
            } else {
                initial.selection.clone()
            }
        );
        let prior_receipt = store.command_result("select-next").unwrap();
        assert_eq!(prior_receipt.is_some(), committed);
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        assert_eq!(
            store.runtime_selection("workspace").unwrap().unwrap(),
            observed
        );
        let ack = store.commit_runtime_selection(&next).unwrap();
        assert_eq!(ack.selection, next.selection);
        if let Some(receipt) = prior_receipt {
            assert_eq!(
                serde_json::from_slice::<ag1_storage::SelectionAck>(
                    receipt.result.as_ref().unwrap()
                )
                .unwrap(),
                ack
            );
        }
        assert_eq!(
            store
                .runtime_selection("workspace")
                .unwrap()
                .unwrap()
                .sha256,
            ack.sha256
        );
    }
}

#[test]
fn two_selection_writers_cannot_publish_different_owners_from_the_same_revision() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(directory.path()).unwrap());
    create(&store, "first");
    create(&store, "second");
    let initial = store
        .commit_runtime_selection(&selection("initial", "first", 1, None))
        .unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let handles = ["candidate-a", "candidate-b"].map(|name| {
        let store = store.clone();
        let barrier = barrier.clone();
        let mut change = selection(name, "second", 2, Some(initial.sha256.clone()));
        change.selection.owner = name.into();
        std::thread::spawn(move || {
            barrier.wait();
            store.commit_runtime_selection(&change)
        })
    });
    let results = handles.map(|handle| handle.join().unwrap());
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let chosen = results.into_iter().find_map(Result::ok).unwrap();
    assert_eq!(
        store
            .runtime_selection("workspace")
            .unwrap()
            .unwrap()
            .selection,
        chosen.selection
    );
}

struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        1000
    }
}
