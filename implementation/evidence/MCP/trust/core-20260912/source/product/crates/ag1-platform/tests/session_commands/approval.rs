use super::*;
use ag1_platform::session::{approvals::PendingApprovals, commands::PendingApproval};

async fn pending(c: &SessionCommands) -> PendingApproval {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let mut requests = c.pending_approvals().unwrap();
            if !requests.is_empty() {
                return requests.remove(0);
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap()
}
fn answer(
    c: &SessionCommands,
    id: &str,
    binding: w::ApprovalBinding,
    allow: bool,
) -> w::CommandEnvelope {
    let mut envelope = prompt(c, id, binding.revision.get(), "");
    envelope.command = w::Command::Approve { binding, allow };
    envelope
}
fn with_approvals(mut f: Fixture) -> Fixture {
    Arc::get_mut(&mut f.files).unwrap().approvals =
        Arc::new(PendingApprovals::new(Arc::new(FixedClock)));
    f
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_approval_binds_actual_preflight_and_releases_one_worker_only_after_admission() {
    for allow in [true, false] {
        let (endpoint, server) = server(vec![
            calls(vec![(
                "write",
                "write",
                json!({"path":"output.txt","content":"approved"}),
            )]),
            stop(),
        ]);
        let f = attach(
            with_approvals(fixture_configured(
                &endpoint,
                "interactive",
                vec![],
                None,
                None,
                1,
            )),
            Arc::new(Resolver::new(false)),
        );
        let operation = accepted(
            f.commands
                .dispatch(prompt(&f.commands, "run", 0, "Write output"))
                .await
                .result,
        );
        let run = {
            let c = f.commands.clone();
            tokio::spawn(async move { c.run_next(&ignore).await })
        };
        let request = pending(&f.commands).await;
        assert_eq!(request.binding.operation, operation);
        assert!(!f.root.join("output.txt").exists());
        for (i, mut invalid) in (0..5).map(|i| (i, request.binding.clone())) {
            match i {
                0 => invalid.argument_sha256 = "0".repeat(64),
                1 => invalid.scope.epoch = w::Sequence::new(8),
                2 => invalid.operation = "foreign-operation".to_owned().try_into().unwrap(),
                3 => invalid.expires_ms = w::Sequence::new(1001),
                _ => invalid.revision = w::Sequence::new(0),
            }
            assert_eq!(
                rejected(
                    f.commands
                        .dispatch(answer(&f.commands, &format!("invalid-{i}"), invalid, true))
                        .await
                        .result
                ),
                w::ProtocolError::InvalidApproval
            );
        }
        let envelope = answer(&f.commands, "answer", request.binding.clone(), allow);
        let accepted_id = accepted(f.commands.dispatch(envelope.clone()).await.result);
        assert_eq!(
            accepted(f.commands.dispatch(envelope).await.result),
            accepted_id
        );
        assert!(
            matches!(lookup(&f.commands, "answer").await, w::CommandResult::Settled { operation } if operation.status == w::OperationStatus::Completed)
        );
        assert_eq!(
            rejected(
                f.commands
                    .dispatch(answer(&f.commands, "second-answer", request.binding, true))
                    .await
                    .result
            ),
            w::ProtocolError::InvalidApproval
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .unwrap()
                .status,
            w::OperationStatus::Completed
        );
        assert_eq!(f.root.join("output.txt").exists(), allow);
        if allow {
            assert_eq!(
                std::fs::read_to_string(f.root.join("output.txt")).unwrap(),
                "approved"
            );
        }
        assert!(f.commands.pending_approvals().unwrap().is_empty());
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_approval_intent_ack_never_grants_permission_on_retry() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let (endpoint, server) = server(vec![calls(vec![(
            "write",
            "write",
            json!({"path":"output.txt","content":"forbidden"}),
        )])]);
        let f = attach(
            with_approvals(fixture_configured(
                &endpoint,
                "interactive",
                vec![],
                Some(fault.clone()),
                None,
                1,
            )),
            Arc::new(Resolver::new(false)),
        );
        let operation = accepted(
            f.commands
                .dispatch(prompt(&f.commands, "run", 0, "Write output"))
                .await
                .result,
        );
        let run = {
            let c = f.commands.clone();
            tokio::spawn(async move { c.run_next(&ignore).await })
        };
        let request = pending(&f.commands).await;
        let envelope = answer(&f.commands, "answer", request.binding, true);
        fault.armed.store(true, Ordering::SeqCst);
        assert!(matches!(
            rejected(f.commands.dispatch(envelope.clone()).await.result),
            w::ProtocolError::Unavailable { .. }
        ));
        if point == ag1_storage::Boundary::BeforeAck {
            accepted(f.commands.dispatch(envelope).await.result);
            assert!(matches!(
                rejected(lookup(&f.commands, "answer").await),
                w::ProtocolError::Unavailable { .. }
            ));
        }
        assert!(!run.is_finished());
        assert!(!f.root.join("output.txt").exists());
        let cancel = super::cancellation::cancel(
            &f.commands,
            "cancel",
            f.host.journal_revision().unwrap(),
            operation,
        );
        accepted(f.commands.dispatch(cancel).await.result);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .unwrap()
                .status,
            w::OperationStatus::Cancelled
        );
        assert!(!f.root.join("output.txt").exists());
        assert!(f.commands.pending_approvals().unwrap().is_empty());
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
