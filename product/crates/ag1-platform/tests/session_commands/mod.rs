//! Command boundary tests compose the actual native loop, bounded workers and
//! SQL writer with the parent module's owned loopback provider fixture.
use super::*;
use ag1_core::context::{ContextArtifactPort, features::Features};
use ag1_platform::session::{
    commands::{RunSettings, SessionCommands},
    registry::{PreparedRuntime, Selection},
};
use ag1_protocol::wire as w;
use ag1_storage::context::ContextStore;
mod approval;
mod cancellation;
mod compaction;
mod configuration;
mod delivery;
mod messages;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_print_continuations_keep_the_command_alive_until_mode_settlement() {
    let mut promise = stop();
    promise["choices"][0]["delta"]["content"] = json!("Let me inspect it:");
    for (text, replies, expected) in [
        (
            "Explain the source.",
            vec![promise, stop()],
            w::OperationStatus::Completed,
        ),
        (
            "Fix the source.",
            vec![stop(); 4],
            w::OperationStatus::Failed,
        ),
    ] {
        let count = replies.len();
        let (endpoint, server) = server(replies);
        let f = attach(
            fixture(&endpoint, "disabled"),
            Arc::new(Resolver::new(false)),
        );
        let mut envelope = prompt(&f.commands, "print", 0, "");
        envelope.command = serde_json::from_value(json!({"kind":"session","action":"print","args":{"input":{"kind":"inline","text":text}}})).unwrap();
        accepted(f.commands.dispatch(envelope).await.result);
        let terminal = f.commands.run_next(&ignore).await.unwrap().unwrap();
        assert_eq!(terminal.status, expected);
        assert_eq!(
            lookup(&f.commands, "print").await,
            w::CommandResult::Settled {
                operation: terminal
            }
        );
        assert!(f.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(), count);
    }
}

struct CommandsFixture {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    store: Arc<Store>,
    host: Arc<NativeSession>,
    commands: Arc<SessionCommands>,
}
fn attach(fixture: Fixture, resolver: Arc<dyn ProviderResolver>) -> CommandsFixture {
    attach_with_catalog(fixture, resolver, vec![])
}
fn attach_with_catalog(
    fixture: Fixture,
    resolver: Arc<dyn ProviderResolver>,
    models: Vec<Value>,
) -> CommandsFixture {
    attach_with_modes(
        fixture,
        resolver,
        models,
        ag1_platform::session::commands::QueueMode::OneAtATime,
    )
}
fn attach_with_modes(
    fixture: Fixture,
    resolver: Arc<dyn ProviderResolver>,
    models: Vec<Value>,
    mode: ag1_platform::session::commands::QueueMode,
) -> CommandsFixture {
    let Fixture {
        _directory,
        root,
        store,
        host,
        files,
        ..
    } = fixture;
    let services = Arc::new(PreparedRuntime::new(
        Arc::try_unwrap(host).ok().unwrap(),
        resolver,
        files,
    ));
    let host = services.runtime().clone();
    let commands = SessionCommands::new(
        Selection { epoch: 7, services },
        "instance-one".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings {
            models,
            steering_mode: mode,
            follow_up_mode: mode,
            max_generations: Some(8),
            compaction: ag1_core::context::compaction::Settings {
                enabled: false,
                ..Default::default()
            },
            retry: ag1_core::runtime::retry::Settings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    CommandsFixture {
        _directory,
        root,
        store,
        host,
        commands,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_replay_requires_a_snapshot_after_native_event_history_rolls_over() {
    struct UnavailableResolver;
    impl ProviderResolver for UnavailableResolver {
        fn resolve<'a>(
            &'a self,
            _: ProviderInput<'a>,
            _: &'a AtomicBool,
        ) -> PortFuture<'a, ResolvedRequest> {
            Box::pin(async {
                Err(DomainError::Unavailable(
                    "fixture credential unavailable".into(),
                ))
            })
        }
    }
    let f = attach(
        fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1),
        Arc::new(UnavailableResolver),
    );
    let first = prompt(&f.commands, "oldest", 0, "Inspect");
    let subscription = f.commands.subscribe().unwrap();
    for i in 0..513 {
        let command = if i == 0 {
            first.clone()
        } else {
            prompt(
                &f.commands,
                &format!("history-{i}"),
                f.host.journal_revision().unwrap(),
                "Inspect",
            )
        };
        accepted(f.commands.dispatch(command).await.result);
        assert_eq!(
            f.commands.run_next(&ignore).await.unwrap().unwrap().status,
            w::OperationStatus::Failed
        );
    }
    let snapshot = f.commands.snapshot().unwrap();
    assert_eq!(snapshot.watermark.get(), 1026);
    assert!(snapshot.queued.is_empty());
    assert_eq!(
        f.commands.poll(&subscription.id).unwrap(),
        w::Delivery::Gap {
            snapshot: snapshot.clone()
        }
    );
    let mut cursor = w::Cursor {
        scope: f.commands.scope().clone(),
        after: w::Sequence::new(0),
    };
    assert_eq!(
        f.commands.replay(&cursor).unwrap(),
        w::Delivery::Gap {
            snapshot: snapshot.clone()
        }
    );
    cursor.after = w::Sequence::new(1);
    assert!(matches!(
        f.commands.replay(&cursor).unwrap(),
        w::Delivery::Gap { .. }
    ));
    cursor.after = w::Sequence::new(2);
    let w::Delivery::Events { events, more, .. } = f.commands.replay(&cursor).unwrap() else {
        panic!()
    };
    assert!(more);
    assert_eq!(events.len(), 32);
    assert_eq!(events[0].sequence.get(), 3);
    assert!(matches!(
        lookup(&f.commands, "oldest").await,
        w::CommandResult::Settled { .. }
    ));
    accepted(f.commands.dispatch(first).await.result);
    assert_eq!(f.commands.snapshot().unwrap().watermark, snapshot.watermark);
    assert_eq!(user_messages(&f.store).len(), 513);
    assert!(f.host.settled().unwrap());
}
fn prompt(c: &SessionCommands, id: &str, revision: u64, text: &str) -> w::CommandEnvelope {
    w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: c.scope().clone(),
        expected_revision: w::Sequence::new(revision),
        command: w::Command::Prompt {
            input: w::PromptInput::Inline { text: text.into() },
        },
        extension: None,
    }
}
async fn lookup(c: &Arc<SessionCommands>, id: &str) -> w::CommandResult {
    let mut query = prompt(c, "query", 0, "");
    query.command = w::Command::CommandResult {
        command: id.to_owned().try_into().unwrap(),
    };
    c.dispatch(query).await.result
}
fn accepted(result: w::CommandResult) -> w::OperationId {
    let w::CommandResult::Accepted { operation } = result else {
        panic!("expected acceptance: {result:?}")
    };
    operation
}
fn rejected(result: w::CommandResult) -> w::ProtocolError {
    let w::CommandResult::Rejected { error } = result else {
        panic!("expected rejection: {result:?}")
    };
    error
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drained_journal_failure_is_not_a_terminal_receipt_or_permission_to_consume_more_prompts() {
    struct JournalFault(ag1_storage::Boundary);
    impl ag1_storage::FaultInjector for JournalFault {
        fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            Ok(())
        }
        fn check_journal_command(
            &self,
            command: &str,
            point: ag1_storage::Boundary,
        ) -> ag1_storage::StorageResult<()> {
            if command.ends_with("-assistant-1") && point == self.0 {
                return Err(ag1_storage::StorageError::Injected(point));
            }
            Ok(())
        }
    }
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![stop()]);
        let f = attach(
            fixture_configured(
                &origin,
                "disabled",
                vec![],
                Some(Arc::new(JournalFault(point))),
                None,
                1,
            ),
            Arc::new(Resolver::new(false)),
        );
        let command = prompt(
            &f.commands,
            "failed-journal",
            0,
            "Preserve journal uncertainty",
        );
        accepted(f.commands.dispatch(command.clone()).await.result);
        accepted(
            f.commands
                .dispatch(prompt(
                    &f.commands,
                    "pending-journal",
                    0,
                    "Wait for recovery",
                ))
                .await
                .result,
        );
        assert!(matches!(
            f.commands.run_next(&ignore).await,
            Err(DomainError::DurabilityUnknown)
        ));
        assert!(f.host.settled().unwrap());
        assert!(!f.host.persistence_healthy().unwrap());
        assert!(matches!(
            lookup(&f.commands, "failed-journal").await,
            w::CommandResult::Rejected {
                error: w::ProtocolError::Unavailable { .. }
            }
        ));
        assert!(matches!(
            f.commands.run_next(&ignore).await,
            Err(DomainError::Conflict)
        ));
        let revision = f.store.snapshot("session").unwrap().revision;
        assert!(matches!(
            f.commands
                .dispatch(prompt(&f.commands, "new-journal", revision, "Still wait"))
                .await
                .result,
            w::CommandResult::Rejected {
                error: w::ProtocolError::Unavailable { .. }
            }
        ));
        accepted(f.commands.dispatch(command).await.result);
        assert_eq!(user_messages(&f.store).len(), 1);
        assert_eq!(f.commands.snapshot().unwrap().queued.len(), 2);
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
fn user_messages(store: &Store) -> Vec<Value> {
    store
        .snapshot("session")
        .unwrap()
        .entries
        .iter()
        .filter_map(|entry| {
            let value: Value = serde_json::from_slice(&entry.normalized).unwrap();
            (value["message"]["role"] == "user").then(|| value["message"].clone())
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn durable_prompts_execute_once_with_current_history_and_real_file_effects() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "write-once",
            "append",
            json!({"path":"once.txt","content":"one"}),
        )]),
        stop(),
        stop(),
    ]);
    let f = attach(
        fixture_configured(&endpoint, "disabled", vec![], None, None, 1),
        Arc::new(Resolver::new(false)),
    );
    // Append requires an existing file; native file mutation remains P09-owned.
    std::fs::write(f.root.join("once.txt"), "").unwrap();
    let a = prompt(&f.commands, "first", 0, "Append one once");
    let b = prompt(&f.commands, "second", 0, "Review the preceding result");
    let mut projection = ag1_protocol::projection::Projection::new(f.commands.snapshot().unwrap());
    let first = accepted(f.commands.dispatch(a.clone()).await.result);
    assert_eq!(accepted(f.commands.dispatch(a.clone()).await.result), first);
    let second = accepted(f.commands.dispatch(b).await.result);
    assert!(user_messages(&f.store).is_empty());
    assert!(!f.commands.handshake().fake);
    assert_eq!(
        f.commands.snapshot().unwrap().queued,
        vec![first.clone(), second.clone()]
    );
    let one = f.commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(
        one,
        w::OperationView {
            operation: first.clone(),
            status: w::OperationStatus::Completed
        }
    );
    assert_eq!(accepted(f.commands.dispatch(a.clone()).await.result), first);
    assert_eq!(
        lookup(&f.commands, "first").await,
        w::CommandResult::Settled { operation: one }
    );
    assert_eq!(
        f.commands
            .run_next(&ignore)
            .await
            .unwrap()
            .unwrap()
            .operation,
        second
    );
    assert!(f.commands.run_next(&ignore).await.unwrap().is_none());
    assert_eq!(user_messages(&f.store).len(), 2);
    assert_eq!(
        std::fs::read_to_string(f.root.join("once.txt")).unwrap(),
        "one"
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[2]["messages"]
            .to_string()
            .contains("Review the preceding result")
    );
    assert!(
        requests[2]["messages"]
            .to_string()
            .contains("Append one once")
    );
    let snapshot = f.commands.snapshot().unwrap();
    assert_eq!(snapshot.watermark.get(), 4);
    assert_eq!(snapshot.revision.get(), f.host.journal_revision().unwrap());
    assert!(snapshot.revision.get() > snapshot.watermark.get());
    let w::Delivery::Events { events, more, .. } = f
        .commands
        .replay(&w::Cursor {
            scope: f.commands.scope().clone(),
            after: w::Sequence::new(0),
        })
        .unwrap()
    else {
        panic!()
    };
    assert!(!more);
    assert_eq!(
        events.iter().map(|e| e.sequence.get()).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(events[0].revision.get(), 0);
    assert_eq!(events[1].revision.get(), 0);
    for event in &events {
        assert!(projection.apply(event).unwrap());
        assert!(!projection.apply(event).unwrap());
    }
    assert_eq!(projection.snapshot, snapshot);
    let mut gap = events[3].clone();
    gap.sequence = w::Sequence::new(6);
    assert_eq!(
        projection.apply(&gap),
        Err(w::ProtocolError::ResyncRequired)
    );
    let mut changed = a.clone();
    changed.extension = Some(Value::Null);
    assert_eq!(
        rejected(f.commands.dispatch(changed).await.result),
        w::ProtocolError::CommandConflict
    );
    let mut stale = a;
    stale.scope.epoch = w::Sequence::new(8);
    assert_eq!(
        rejected(f.commands.dispatch(stale).await.result),
        w::ProtocolError::StaleContext
    );
    assert!(f.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_is_bounded_revision_fenced_and_duplicates_bypass_full_queue() {
    let f = attach(
        fixture("http://127.0.0.1:1", "disabled"),
        Arc::new(Resolver::new(false)),
    );
    let command = prompt(&f.commands, "first", 0, "First");
    let operation = accepted(f.commands.dispatch(command.clone()).await.result);
    f.host
        .select_thinking("thinking", "thinking-entry", NOW, "high".into(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        accepted(f.commands.dispatch(command.clone()).await.result),
        operation
    );
    assert_eq!(
        rejected(
            f.commands
                .dispatch(prompt(&f.commands, "stale", 0, "Stale"))
                .await
                .result
        ),
        w::ProtocolError::StaleContext
    );
    let revision = f.host.journal_revision().unwrap();
    for i in 1..w::MAX_SESSION_QUEUE {
        accepted(
            f.commands
                .dispatch(prompt(
                    &f.commands,
                    &format!("fill-{i}"),
                    revision,
                    "Queued",
                ))
                .await
                .result,
        );
    }
    assert_eq!(
        f.commands.snapshot().unwrap().queued.len(),
        w::MAX_SESSION_QUEUE
    );
    assert!(matches!(
        rejected(
            f.commands
                .dispatch(prompt(&f.commands, "overflow", revision, "Full"))
                .await
                .result
        ),
        w::ProtocolError::QueueFull { capacity: 256, .. }
    ));
    assert_eq!(
        accepted(f.commands.dispatch(command.clone()).await.result),
        operation
    );
    let mut different = command.clone();
    different.extension = Some(Value::Null);
    assert_eq!(
        rejected(f.commands.dispatch(different).await.result),
        w::ProtocolError::CommandConflict
    );
    let mut wrong = command.clone();
    wrong.scope.instance = "stale-instance".to_owned().try_into().unwrap();
    assert_eq!(
        rejected(f.commands.dispatch(wrong).await.result),
        w::ProtocolError::StaleContext
    );
    let mut wrong = command;
    wrong.version.major = 99;
    assert_eq!(
        rejected(f.commands.dispatch(wrong).await.result),
        w::ProtocolError::IncompatibleVersion
    );
    assert!(matches!(
        rejected(
            f.commands
                .dispatch(prompt(
                    &f.commands,
                    "oversize",
                    revision,
                    &"x".repeat(w::MAX_ENVELOPE)
                ))
                .await
                .result
        ),
        w::ProtocolError::Oversize { .. }
    ));
    let mut cursor = w::Cursor {
        scope: f.commands.scope().clone(),
        after: w::Sequence::new(0),
    };
    let mut total = 0;
    loop {
        let delivery = f.commands.replay(&cursor).unwrap();
        assert!(w::encode(&w::WireMessage::Delivery(delivery.clone())).is_ok());
        let w::Delivery::Events {
            events,
            cursor: next,
            more,
        } = delivery
        else {
            panic!()
        };
        assert!(events.len() <= 32);
        total += events.len();
        cursor = next;
        if !more {
            break;
        }
    }
    assert_eq!(total, 256);
    cursor.after = w::Sequence::new(257);
    assert!(matches!(
        f.commands.replay(&cursor).unwrap(),
        w::Delivery::Gap { .. }
    ));
    assert!(user_messages(&f.store).is_empty());
    assert_eq!(
        lookup(&f.commands, "overflow").await,
        w::CommandResult::Rejected {
            error: w::ProtocolError::NotFound
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_duplicate_delivery_has_one_execution_claim() {
    let (endpoint, server) = server(vec![stop()]);
    let f = attach(
        fixture(&endpoint, "disabled"),
        Arc::new(Resolver::new(false)),
    );
    let command = prompt(&f.commands, "same", 0, "One request");
    let mut tasks = vec![];
    for _ in 0..16 {
        let commands = f.commands.clone();
        let envelope = command.clone();
        tasks.push(tokio::spawn(
            async move { commands.dispatch(envelope).await },
        ));
    }
    let mut operations = vec![];
    for task in tasks {
        operations.push(accepted(task.await.unwrap().result));
    }
    assert!(operations.iter().all(|op| *op == operations[0]));
    assert_eq!(f.commands.snapshot().unwrap().queued.len(), 1);
    f.commands.run_next(&ignore).await.unwrap();
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(user_messages(&f.store).len(), 1);
    assert!(f.commands.run_next(&ignore).await.unwrap().is_none());
}

#[derive(Default)]
struct WorkerGate {
    armed: AtomicBool,
    entered: Notify,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}
impl WorkerGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
impl ag1_storage::FaultInjector for WorkerGate {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point == ag1_storage::Boundary::BeforeAck && self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            let (released, _) = self
                .changed
                .wait_timeout_while(
                    self.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            assert!(*released, "release accepted worker");
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_admission_response_still_publishes_acknowledged_worker_queue() {
    let gate = Arc::new(WorkerGate::default());
    let (endpoint, server) = server(vec![stop()]);
    let f = attach(
        fixture_configured(&endpoint, "disabled", vec![], Some(gate.clone()), None, 1),
        Arc::new(Resolver::new(false)),
    );
    let command = prompt(
        &f.commands,
        "lost-response",
        0,
        "Accepted despite UI disconnect",
    );
    gate.armed.store(true, Ordering::SeqCst);
    let task = {
        let c = f.commands.clone();
        let e = command.clone();
        tokio::spawn(async move { c.dispatch(e).await })
    };
    gate.entered.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.commands.snapshot().unwrap().queued.is_empty());
    gate.release();
    let operation = accepted(f.commands.dispatch(command).await.result);
    assert_eq!(f.commands.snapshot().unwrap().queued, vec![operation]);
    f.commands.run_next(&ignore).await.unwrap();
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(user_messages(&f.store).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_admission_ack_never_grants_execution_on_retry() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let f = attach(
            fixture_seeded_with_fault(
                "http://127.0.0.1:1",
                "disabled",
                vec![],
                Some(fault.clone()),
            ),
            Arc::new(Resolver::new(false)),
        );
        let command = prompt(
            &f.commands,
            "lost-ack",
            0,
            "Do not infer execution from an intent",
        );
        fault.armed.store(true, Ordering::SeqCst);
        assert!(matches!(
            rejected(f.commands.dispatch(command.clone()).await.result),
            w::ProtocolError::Unavailable { .. }
        ));
        assert!(f.commands.snapshot().unwrap().queued.is_empty());
        accepted(f.commands.dispatch(command).await.result);
        if point == ag1_storage::Boundary::BeforeAck {
            assert!(
                matches!(rejected(lookup(&f.commands, "lost-ack").await), w::ProtocolError::Unavailable { family } if family == "native_command_recovery")
            );
            assert!(f.commands.run_next(&ignore).await.unwrap().is_none());
        } else {
            assert_eq!(f.commands.snapshot().unwrap().queued.len(), 1);
        }
        assert!(user_messages(&f.store).is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_consumer_is_never_requeued_after_unsent_preparation_drains() {
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(fixture("http://127.0.0.1:1", "disabled"), resolver.clone());
    let subscription = f.commands.subscribe().unwrap();
    let first = prompt(
        &f.commands,
        "abandoned",
        0,
        "Interrupted after durable user append",
    );
    let operation = accepted(f.commands.dispatch(first.clone()).await.result);
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "next", 0, "Wait for recovery"))
            .await
            .result,
    );
    let task = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    f.commands
        .progress(&operation, "Transient preparation".into())
        .unwrap();
    assert!(matches!(
        f.commands.run_next(&ignore).await,
        Err(DomainError::Conflict)
    ));
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        matches!(f.commands.poll(&subscription.id).unwrap(), w::Delivery::Events { events, .. } if events.len() == 2)
    );
    assert!(
        matches!(f.commands.poll(&subscription.id).unwrap(), w::Delivery::Events { events, .. } if events.is_empty())
    );
    assert_eq!(
        f.commands.queued_progress_bytes(&subscription.id).unwrap(),
        0
    );
    // Dropped unsent preparation releases its exact native reservation. The
    // admitted command still has no terminal receipt: queue ownership below
    // must block another consumer instead of relying on a leaked reservation.
    assert!(f.host.settled().unwrap());
    accepted(f.commands.dispatch(first).await.result);
    assert!(matches!(
        rejected(lookup(&f.commands, "abandoned").await),
        w::ProtocolError::Unavailable { .. }
    ));
    assert!(matches!(
        f.commands.run_next(&ignore).await,
        Err(DomainError::Conflict)
    ));
    assert_eq!(user_messages(&f.store).len(), 1);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_live_consumer_drains_an_indefinitely_pending_resolver() {
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(
        fixture_configured("http://127.0.0.1:1", "disabled", vec![], None, None, 1),
        resolver.clone(),
    );
    let command = prompt(&f.commands, "cancelled", 0, "Cancel preparation");
    accepted(f.commands.dispatch(command).await.result);
    let task = {
        let commands = f.commands.clone();
        tokio::spawn(async move { commands.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    f.host.cancel().unwrap();
    let operation = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(operation.status, w::OperationStatus::Cancelled);
    assert_eq!(
        lookup(&f.commands, "cancelled").await,
        w::CommandResult::Settled { operation }
    );
    assert!(f.commands.snapshot().unwrap().queued.is_empty());
    assert!(f.host.settled().unwrap());
    assert!(f.host.pending_transport().unwrap().is_none());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(user_messages(&f.store).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_receipt_failure_blocks_following_prompts_until_committed_evidence_is_reconciled()
{
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let (endpoint, server) = server(vec![stop()]);
        let f = attach(
            fixture_configured(&endpoint, "disabled", vec![], Some(fault.clone()), None, 1),
            Arc::new(Resolver::new(false)),
        );
        let command = prompt(&f.commands, "terminal-fault", 0, "One completed generation");
        let operation = accepted(f.commands.dispatch(command.clone()).await.result);
        accepted(
            f.commands
                .dispatch(prompt(&f.commands, "pending", 0, "Wait for receipt"))
                .await
                .result,
        );
        let listener = |event: &SessionEvent| {
            if matches!(event, SessionEvent::Committed { .. })
                && f.store
                    .snapshot("session")
                    .unwrap()
                    .entries
                    .iter()
                    .any(|e| {
                        serde_json::from_slice::<Value>(&e.normalized).unwrap()["message"]["role"]
                            == "assistant"
                    })
            {
                fault.armed.store(true, Ordering::SeqCst);
            }
        };
        assert!(f.commands.run_next(&listener).await.is_err());
        assert!(f.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(), 1);
        assert_eq!(
            accepted(f.commands.dispatch(command).await.result),
            operation
        );
        assert!(matches!(
            f.commands.run_next(&ignore).await,
            Err(DomainError::Conflict)
        ));
        let id = "terminal-fault".to_owned().try_into().unwrap();
        if point == ag1_storage::Boundary::BeforeAck {
            let result = lookup(&f.commands, "terminal-fault").await;
            assert!(matches!(result, w::CommandResult::Settled { .. }));
            let done = f.commands.reconcile_terminal(id).await.unwrap();
            assert_eq!(done.status, w::OperationStatus::Completed);
            assert_eq!(f.commands.snapshot().unwrap().queued.len(), 1);
            let watermark = f.commands.snapshot().unwrap().watermark;
            f.commands
                .reconcile_terminal("terminal-fault".to_owned().try_into().unwrap())
                .await
                .unwrap();
            assert_eq!(f.commands.snapshot().unwrap().watermark, watermark);
        } else {
            assert!(matches!(
                rejected(lookup(&f.commands, "terminal-fault").await),
                w::ProtocolError::Unavailable { .. }
            ));
            assert!(matches!(
                f.commands.reconcile_terminal(id).await,
                Err(DomainError::DurabilityUnknown)
            ));
            assert_eq!(f.commands.snapshot().unwrap().queued.len(), 2);
        }
        assert_eq!(user_messages(&f.store).len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_terminal_response_keeps_accepted_worker_settlement_and_publication() {
    let gate = Arc::new(WorkerGate::default());
    let (endpoint, server) = server(vec![stop()]);
    let f = attach(
        fixture_configured(&endpoint, "disabled", vec![], Some(gate.clone()), None, 1),
        Arc::new(Resolver::new(false)),
    );
    accepted(
        f.commands
            .dispatch(prompt(
                &f.commands,
                "done",
                0,
                "Retain terminal acknowledgement",
            ))
            .await
            .result,
    );
    let task = {
        let c = f.commands.clone();
        let store = f.store.clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            let listener = |event: &SessionEvent| {
                if matches!(event, SessionEvent::Committed { .. })
                    && store.snapshot("session").unwrap().entries.iter().any(|e| {
                        serde_json::from_slice::<Value>(&e.normalized).unwrap()["message"]["role"]
                            == "assistant"
                    })
                {
                    gate.armed.store(true, Ordering::SeqCst);
                }
            };
            c.run_next(&listener).await
        })
    };
    gate.entered.notified().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(f.host.settled().unwrap());
    assert!(f.commands.snapshot().unwrap().last_terminal.is_none());
    assert_eq!(f.commands.snapshot().unwrap().queued.len(), 1);
    gate.release();
    assert!(matches!(
        lookup(&f.commands, "done").await,
        w::CommandResult::Settled { .. }
    ));
    assert!(f.commands.snapshot().unwrap().queued.is_empty());
    assert_eq!(
        f.commands.snapshot().unwrap().last_terminal.unwrap().status,
        w::OperationStatus::Completed
    );
    assert!(f.commands.run_next(&ignore).await.unwrap().is_none());
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn artifact_prompts_use_authorized_bounded_bytes_and_retain_idempotency() {
    let (endpoint, server) = server(vec![stop()]);
    let f = attach(
        fixture_configured(&endpoint, "disabled", vec![], None, None, 1),
        Arc::new(Resolver::new(false)),
    );
    let mut artifacts = ContextStore::new(
        &f.store,
        SessionId::new("session").unwrap(),
        "session",
        "input",
        NOW,
        Features::default(),
    )
    .unwrap();
    let text = format!(
        "{}\nRead the complete authorized prompt",
        "αβγ".repeat(12000)
    );
    let artifact = artifacts
        .put_text(&text, "user-input", "command", "input", &[])
        .unwrap();
    let mut command = prompt(&f.commands, "artifact-input", 0, "");
    command.command = w::Command::Prompt {
        input: w::PromptInput::Artifact {
            artifact: artifact.id.clone().try_into().unwrap(),
        },
    };
    let operation = accepted(f.commands.dispatch(command.clone()).await.result);
    let mut page = prompt(&f.commands, "read", 0, "");
    page.command = w::Command::ArtifactRead {
        artifact: artifact.id.try_into().unwrap(),
        offset: w::Sequence::new(0),
        limit: 32,
    };
    let reply = f.commands.dispatch(page).await;
    let w::CommandResult::ArtifactPage { base64, next, .. } = reply.result else {
        panic!()
    };
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .unwrap();
    assert!(text.as_bytes().starts_with(&bytes));
    assert!(next.is_some());
    f.commands.run_next(&ignore).await.unwrap();
    assert_eq!(user_messages(&f.store)[0]["content"][0]["text"], text);
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(
        accepted(f.commands.dispatch(command).await.result),
        operation
    );
    let mut missing = prompt(
        &f.commands,
        "missing-artifact",
        f.host.journal_revision().unwrap(),
        "",
    );
    missing.command = w::Command::Prompt {
        input: w::PromptInput::Artifact {
            artifact: "art_missing".to_owned().try_into().unwrap(),
        },
    };
    assert!(matches!(
        f.commands.dispatch(missing).await.result,
        w::CommandResult::Rejected { .. }
    ));
    assert!(f.commands.snapshot().unwrap().queued.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_recovers_terminal_queries_and_never_reexecutes_unfinished_admissions() {
    let (endpoint, server) = server(vec![stop()]);
    let f = attach(
        fixture(&endpoint, "disabled"),
        Arc::new(Resolver::new(false)),
    );
    let done = prompt(&f.commands, "done", 0, "Remember completion");
    accepted(f.commands.dispatch(done.clone()).await.result);
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "unfinished", 0, "No automatic restart"))
            .await
            .result,
    );
    let terminal = f.commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(server.join().unwrap().len(), 1);
    let configuration = f.host.configuration().unwrap();
    let CommandsFixture {
        _directory,
        root,
        store,
        host,
        commands,
    } = f;
    drop(commands);
    drop(host);
    drop(store);
    let store = Arc::new(Store::open(&_directory.path().join("store")).unwrap());
    let state = SessionRuntime::resume(
        SessionId::new("session").unwrap(),
        8,
        &store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
        configuration,
    )
    .unwrap();
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let native = NativeSession::new(
        state,
        store.clone(),
        "session".into(),
        workspace,
        Arc::new(BlockingPool::new(1, 8).unwrap()),
        Scheduler::default(),
        ProviderHttp::new(&[endpoint], Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    struct NoTools;
    impl ag1_platform::session::ToolRunner for NoTools {
        fn execute(
            &self,
            _: &ag1_core::tools::registry::Scope,
            _: &[ag1_core::runtime::session::PendingCall],
            _: &ag1_platform::session::ToolSelection,
            _: &str,
            _: &AtomicBool,
            _: &std::sync::atomic::AtomicU64,
        ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
            panic!("restart may not execute")
        }
    }
    let resolver = Arc::new(Resolver::new(false));
    let selection = Selection {
        epoch: 8,
        services: Arc::new(PreparedRuntime::new(
            native,
            resolver.clone(),
            Arc::new(NoTools),
        )),
    };
    let commands = SessionCommands::new(
        selection.clone(),
        "instance-two".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    assert!(
        SessionCommands::new(
            selection,
            "instance-three".to_owned().try_into().unwrap(),
            Arc::new(FixedClock),
            RunSettings::default()
        )
        .is_err()
    );
    assert_eq!(
        rejected(commands.dispatch(done.clone()).await.result),
        w::ProtocolError::StaleContext
    );
    assert_eq!(
        lookup(&commands, "done").await,
        w::CommandResult::Settled {
            operation: terminal
        }
    );
    assert!(
        matches!(rejected(lookup(&commands, "unfinished").await), w::ProtocolError::Unavailable { family } if family == "native_command_recovery")
    );
    assert!(commands.run_next(&ignore).await.unwrap().is_none());
    let mut reused = done;
    reused.scope = commands.scope().clone();
    assert_eq!(
        rejected(commands.dispatch(reused).await.result),
        w::ProtocolError::Unavailable {
            family: "native_restart_recovery".into()
        }
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(user_messages(&store).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn artifact_input_rejects_foreign_or_oversized_evidence_before_admission() {
    let f = attach(
        fixture("http://127.0.0.1:1", "disabled"),
        Arc::new(Resolver::new(false)),
    );
    f.store
        .create_session(
            "foreign",
            &serde_json::to_vec(
                &json!({"type":"session","version":3,"id":"foreign","cwd":f.root,"timestamp":NOW}),
            )
            .unwrap(),
        )
        .unwrap();
    f.store
        .register_origin("foreign", "foreign", &[3; 32])
        .unwrap();
    let mut foreign = ContextStore::new(
        &f.store,
        SessionId::new("foreign").unwrap(),
        "foreign",
        "foreign-input",
        NOW,
        Features::default(),
    )
    .unwrap();
    let foreign = foreign
        .put_text("Not authorized", "user-input", "command", "foreign", &[])
        .unwrap();
    let mut local = ContextStore::new(
        &f.store,
        SessionId::new("session").unwrap(),
        "session",
        "oversize-input",
        NOW,
        Features::default(),
    )
    .unwrap();
    let large = local
        .put_text(
            &"x".repeat(ag1_platform::session::commands::MAX_PROMPT_BYTES as usize + 1),
            "user-input",
            "command",
            "large",
            &[],
        )
        .unwrap();
    for (id, artifact) in [("foreign", foreign.id), ("large", large.id)] {
        let mut command = prompt(&f.commands, id, 0, "");
        command.command = w::Command::Prompt {
            input: w::PromptInput::Artifact {
                artifact: artifact.try_into().unwrap(),
            },
        };
        assert!(matches!(
            f.commands.dispatch(command).await.result,
            w::CommandResult::Rejected { .. }
        ));
        assert_eq!(
            lookup(&f.commands, id).await,
            w::CommandResult::Rejected {
                error: w::ProtocolError::NotFound
            }
        );
    }
    assert!(f.commands.snapshot().unwrap().queued.is_empty());
    assert!(user_messages(&f.store).is_empty());
}
