use super::*;
use ag1_core::runtime::subagent::{
    Profile,
    checkpoint::{Checkpoint, Evidence, Manager, Owner, State},
};
fn owner() -> Owner {
    Owner {
        workspace: "workspace".into(),
        session: "session".into(),
    }
}
fn initial() -> Checkpoint {
    Checkpoint::new(
        "ckpt_fixture".into(),
        Profile::Explore,
        "Inspect".into(),
        NOW.into(),
    )
    .unwrap()
}

#[test]
fn checkpoint_updates_wait_for_real_sql_ack_and_reconcile_without_repeating_evidence() {
    for point in [Boundary::BeforeCommit, Boundary::BeforeAck] {
        let directory = tempfile::tempdir().unwrap();
        let fault = Arc::new(FailOnce {
            point,
            armed: AtomicBool::new(false),
        });
        let store =
            Store::open_with(directory.path(), Arc::new(FixedClock), fault.clone()).unwrap();
        let mut manager = Manager::create(&store, owner(), initial()).unwrap();
        let mut next = manager.acknowledged().clone();
        next.add_evidence(
            Evidence::Inspect {
                tool: "read".into(),
                detail: "main.rs".into(),
            },
            0,
            NOW,
        )
        .unwrap();
        next.add_summary("Retained finding", 0, NOW).unwrap();
        next.last_turn = 1;
        fault.armed.store(true, Ordering::SeqCst);
        assert!(manager.commit(&store, next.clone()).is_err());
        assert_eq!(manager.acknowledged().tool_calls, 0);
        assert!(manager.needs_reconciliation());
        assert!(matches!(
            manager.commit(&store, next.clone()),
            Err(DomainError::DurabilityUnknown)
        ));
        manager.reconcile(&store).unwrap();
        assert!(!manager.needs_reconciliation());
        if point == Boundary::BeforeCommit {
            assert_eq!(manager.acknowledged().tool_calls, 0);
            manager.commit(&store, next).unwrap();
        }
        assert_eq!(manager.acknowledged().tool_calls, 1);
        assert_eq!(manager.acknowledged().evidence.len(), 1);
        let mut done = manager.acknowledged().clone();
        done.final_state = Some(State::Completed);
        manager.commit(&store, done.clone()).unwrap();
        assert!(manager.commit(&store, done).is_err());
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        let recovered = Manager::load(&store, owner(), "ckpt_fixture").unwrap();
        assert_eq!(recovered.acknowledged().final_state, Some(State::Completed));
        assert_eq!(
            recovered.acknowledged().summaries[0].text,
            "Retained finding"
        );
        assert!(matches!(
            Manager::load(
                &store,
                Owner {
                    session: "foreign".into(),
                    ..owner()
                },
                "ckpt_fixture"
            ),
            Err(DomainError::NotFound)
        ));
        assert!(matches!(
            Manager::load(&store, owner(), "../outside"),
            Err(DomainError::InvalidInput(_))
        ));
    }
}

#[test]
fn checkpoint_compare_and_swap_rejects_a_second_writer_without_losing_committed_findings() {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(directory.path()).unwrap();
    let mut one = Manager::create(&store, owner(), initial()).unwrap();
    let mut two = Manager::load(&store, owner(), "ckpt_fixture").unwrap();
    let mut first = one.acknowledged().clone();
    first.add_summary("First writer", 0, NOW).unwrap();
    one.commit(&store, first).unwrap();
    let mut stale = two.acknowledged().clone();
    stale.add_summary("Stale replacement", 0, NOW).unwrap();
    assert!(two.commit(&store, stale).is_err());
    assert!(two.needs_reconciliation());
    two.reconcile(&store).unwrap();
    assert_eq!(two.acknowledged().summaries[0].text, "First writer");
    assert_eq!(two.acknowledged().summaries.len(), 1);
}
