use super::*;

pub(super) fn cancel(
    c: &SessionCommands,
    id: &str,
    revision: u64,
    operation: w::OperationId,
) -> w::CommandEnvelope {
    let mut envelope = prompt(c, id, revision, "");
    envelope.command = w::Command::Cancel { operation };
    envelope
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_cancel_reaches_approval_on_a_single_tool_worker_and_drains_each_result() {
    let (endpoint, server) = server(vec![calls(vec![
        (
            "first",
            "write",
            json!({"path":"output.txt","content":"one"}),
        ),
        (
            "second",
            "write",
            json!({"path":"output.txt","content":"two"}),
        ),
    ])]);
    let mut fixture = fixture_configured(&endpoint, "interactive", vec![], None, None, 1);
    let approval = Arc::new(WaitingApproval {
        entered: Notify::new(),
        approved: AtomicBool::new(false),
    });
    Arc::get_mut(&mut fixture.files).unwrap().approvals = approval.clone();
    let f = attach(fixture, Arc::new(Resolver::new(false)));
    let operation = accepted(
        f.commands
            .dispatch(prompt(&f.commands, "run", 0, "Write output"))
            .await
            .result,
    );
    let task = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    approval.entered.notified().await;
    let envelope = cancel(
        &f.commands,
        "cancel",
        f.host.journal_revision().unwrap(),
        operation.clone(),
    );
    let reply = tokio::time::timeout(
        Duration::from_secs(2),
        f.commands.dispatch(envelope.clone()),
    )
    .await
    .unwrap();
    assert_eq!(accepted(reply.result), operation);
    let done = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(done.status, w::OperationStatus::Cancelled);
    assert_eq!(
        lookup(&f.commands, "cancel").await,
        w::CommandResult::Settled { operation: done }
    );
    assert_eq!(
        accepted(f.commands.dispatch(envelope).await.result),
        operation
    );
    assert!(!f.root.join("output.txt").exists());
    let journal = f.store.snapshot("session").unwrap();
    let results: Vec<Value> = journal
        .entries
        .iter()
        .map(|e| serde_json::from_slice(&e.normalized).unwrap())
        .filter(|e: &Value| e["message"]["role"] == "toolResult")
        .collect();
    assert_eq!(results.len(), 2);
    assert!(f.host.settled().unwrap());
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cancel_never_interrupts_the_active_prompt_or_appends_cancelled_input() {
    let (endpoint, server) = server(vec![stop()]);
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(
        fixture_configured(&endpoint, "disabled", vec![], None, None, 1),
        resolver.clone(),
    );
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "active", 0, "Keep running"))
            .await
            .result,
    );
    let queued = accepted(
        f.commands
            .dispatch(prompt(&f.commands, "queued", 0, "Never deliver"))
            .await
            .result,
    );
    let task = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    let cancel = cancel(
        &f.commands,
        "cancel-queued",
        f.host.journal_revision().unwrap(),
        queued.clone(),
    );
    assert_eq!(accepted(f.commands.dispatch(cancel).await.result), queued);
    assert_eq!(accepted(lookup(&f.commands, "cancel-queued").await), queued);
    assert!(!task.is_finished());
    resolver.release.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    let done = f.commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(done.operation, queued);
    assert_eq!(done.status, w::OperationStatus::Cancelled);
    assert_eq!(
        lookup(&f.commands, "cancel-queued").await,
        w::CommandResult::Settled { operation: done }
    );
    assert_eq!(user_messages(&f.store).len(), 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_requires_current_revision_and_acknowledged_intent_before_signalling() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let resolver = Arc::new(Resolver::new(true));
        let f = attach(
            fixture_configured(
                "http://127.0.0.1:1",
                "disabled",
                vec![],
                Some(fault.clone()),
                None,
                1,
            ),
            resolver.clone(),
        );
        let operation = accepted(
            f.commands
                .dispatch(prompt(&f.commands, "run", 0, "Wait for cancellation"))
                .await
                .result,
        );
        let task = {
            let c = f.commands.clone();
            tokio::spawn(async move { c.run_next(&ignore).await })
        };
        resolver.entered.notified().await;
        assert_eq!(
            rejected(
                f.commands
                    .dispatch(cancel(&f.commands, "stale", 0, operation.clone()))
                    .await
                    .result
            ),
            w::ProtocolError::StaleContext
        );
        assert!(!task.is_finished());
        let envelope = cancel(
            &f.commands,
            "fault",
            f.host.journal_revision().unwrap(),
            operation,
        );
        fault.armed.store(true, Ordering::SeqCst);
        assert!(matches!(
            rejected(f.commands.dispatch(envelope.clone()).await.result),
            w::ProtocolError::Unavailable { .. }
        ));
        assert!(!task.is_finished());
        if point == ag1_storage::Boundary::BeforeAck {
            accepted(f.commands.dispatch(envelope).await.result);
            assert!(matches!(
                rejected(lookup(&f.commands, "fault").await),
                w::ProtocolError::Unavailable { .. }
            ));
            assert!(!task.is_finished());
            f.host.cancel().unwrap();
        } else {
            accepted(f.commands.dispatch(envelope).await.result);
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .unwrap()
                .status,
            w::OperationStatus::Cancelled
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        assert!(f.host.settled().unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_cancel_response_does_not_drop_the_accepted_signal_worker() {
    let gate = Arc::new(WorkerGate::default());
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(
        fixture_configured(
            "http://127.0.0.1:1",
            "disabled",
            vec![],
            Some(gate.clone()),
            None,
            1,
        ),
        resolver.clone(),
    );
    let operation = accepted(
        f.commands
            .dispatch(prompt(&f.commands, "run", 0, "Wait"))
            .await
            .result,
    );
    let run = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    let envelope = cancel(
        &f.commands,
        "cancel",
        f.host.journal_revision().unwrap(),
        operation,
    );
    gate.armed.store(true, Ordering::SeqCst);
    let dispatch = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.dispatch(envelope).await })
    };
    gate.entered.notified().await;
    dispatch.abort();
    assert!(dispatch.await.unwrap_err().is_cancelled());
    assert!(!run.is_finished());
    gate.release();
    let done = tokio::time::timeout(Duration::from_secs(2), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(done.status, w::OperationStatus::Cancelled);
    assert_eq!(
        lookup(&f.commands, "cancel").await,
        w::CommandResult::Settled { operation: done }
    );
    assert!(f.host.settled().unwrap());
}
