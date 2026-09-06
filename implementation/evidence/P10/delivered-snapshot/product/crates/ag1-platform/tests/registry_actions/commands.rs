use super::*;
use ag1_platform::session::{
    commands::RunSettings,
    runtime_commands::{RuntimeCommands, SessionReceipt},
};
use ag1_protocol::wire as w;

struct LifecycleEvents {
    registry: Mutex<std::sync::Weak<RuntimeRegistry>>,
    events: Mutex<Vec<Value>>,
    fail_start: bool,
}
impl ToolRunner for LifecycleEvents {
    fn handles_session_event(&self, _: &str) -> bool {
        true
    }
    fn session_event(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
    ) -> Result<Option<Value>, DomainError> {
        if event["type"] == "session_start" {
            let selected = self
                .registry
                .lock()
                .unwrap()
                .upgrade()
                .unwrap()
                .selected()?;
            assert_eq!(selected.epoch, scope.epoch);
            assert_eq!(
                selected.services.runtime().configuration()?.model,
                snapshot["model"]
            );
        }
        self.events
            .lock()
            .unwrap()
            .push(json!({"event":event,"snapshot":snapshot}));
        if self.fail_start && event["type"] == "session_start" && event["reason"] != "startup" {
            Err(DomainError::Unavailable("fixture startup failed".into()))
        } else {
            Ok(None)
        }
    }
    fn execute(
        &self,
        _: &Scope,
        _: &[ag1_core::runtime::session::PendingCall],
        _: &ToolSelection,
        _: &str,
        _: &AtomicBool,
        _: &AtomicU64,
    ) -> Result<Vec<ToolOutcome>, DomainError> {
        panic!("lifecycle fixture must not execute tools")
    }
}
struct LifecycleFactory {
    delegate: StoredRuntimeFactory,
    events: Arc<LifecycleEvents>,
}
impl RuntimeFactory for LifecycleFactory {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            let prepared = self.delegate.prepare(target, epoch).await?;
            Ok(PreparedRuntime::from_shared(
                prepared.runtime().clone(),
                prepared.provider().clone(),
                self.events.clone(),
            ))
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_observes_selected_configuration_and_terminal_recovery_never_replays_callbacks() {
    for fail_start in [false, true] {
        let fault = Arc::new(StageFault {
            boundary: ag1_storage::Boundary::BeforeAck,
            remaining: AtomicUsize::new(0),
            selection: false,
        });
        let f = Fixture::with_fault(Some(fault.clone()));
        let (first, provider) = parent(&f);
        let events = Arc::new(LifecycleEvents {
            registry: Mutex::new(std::sync::Weak::new()),
            events: Mutex::new(Vec::new()),
            fail_start,
        });
        let first =
            PreparedRuntime::from_shared(first.runtime().clone(), provider.clone(), events.clone());
        let registry =
            Arc::new(RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap());
        registry.reconcile_selection().await.unwrap();
        *events.registry.lock().unwrap() = Arc::downgrade(&registry);
        let host = RuntimeCommands::new(
            registry.clone(),
            Arc::new(LifecycleFactory {
                delegate: factory(&f, provider.clone()),
                events: events.clone(),
            }),
            "client".to_owned().try_into().unwrap(),
            Arc::new(FixedClock),
            RunSettings::default(),
            tokio::runtime::Handle::current(),
        )
        .unwrap();
        host.initialize().await.unwrap();
        host.initialize().await.unwrap();
        let original = host.handshake().unwrap().scope;
        let request = session(&original, "new", 4, "new", json!({}));
        fault.remaining.store(3, Ordering::Release);
        assert!(matches!(
            host.dispatch(request.clone()).await.result,
            w::CommandResult::Accepted { .. }
        ));
        recovery(&host).await;
        assert_eq!(host.handshake().unwrap().scope, original);
        assert_eq!(registry.selected().unwrap().epoch, 2);
        host.reconcile().await.unwrap();
        let status = settled(&host, &request).await.status;
        assert_eq!(
            status,
            if fail_start {
                w::OperationStatus::Failed
            } else {
                w::OperationStatus::Completed
            }
        );
        assert_eq!(host.handshake().unwrap().scope.epoch.get(), 2);
        assert!(matches!(
            host.dispatch(request.clone()).await.result,
            w::CommandResult::Settled { .. }
        ));
        let records = events.events.lock().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|r| r["event"]["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "session_start",
                "session_before_switch",
                "session_shutdown",
                "session_start"
            ]
        );
        assert_eq!(records[0]["event"]["reason"], "startup");
        assert_eq!(records[3]["snapshot"]["model"]["id"], "selected");
        assert_eq!(
            records[3]["snapshot"]["session"]["branch"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(records[3]["snapshot"]["isIdle"], true);
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    }
}

async fn host(f: &Fixture) -> (Arc<RuntimeCommands>, Arc<RuntimeRegistry>, Arc<Resolver>) {
    let (first, provider) = parent(f);
    let registry =
        Arc::new(RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap());
    registry.reconcile_selection().await.unwrap();
    let host = RuntimeCommands::new(
        registry.clone(),
        Arc::new(factory(f, provider.clone())),
        "client".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    (host, registry, provider)
}
fn envelope(scope: &w::Scope, id: &str, revision: u64, command: w::Command) -> w::CommandEnvelope {
    w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: scope.clone(),
        expected_revision: w::Sequence::new(revision),
        command,
        extension: None,
    }
}
fn session(
    scope: &w::Scope,
    id: &str,
    revision: u64,
    action: &str,
    args: Value,
) -> w::CommandEnvelope {
    envelope(
        scope,
        id,
        revision,
        w::Command::Session {
            action: action.into(),
            args,
        },
    )
}
async fn settled(host: &Arc<RuntimeCommands>, request: &w::CommandEnvelope) -> w::OperationView {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let reply = host
                .dispatch(envelope(
                    &request.scope,
                    "query",
                    0,
                    w::Command::CommandResult {
                        command: request.id.clone(),
                    },
                ))
                .await;
            if let w::CommandResult::Settled { operation } = &reply.result
                && !host
                    .snapshot()
                    .unwrap()
                    .queued
                    .contains(&operation.operation)
            {
                return operation.clone();
            }
            assert!(
                !host.recovery_required().unwrap(),
                "unexpected recovery: {reply:?}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn recovery(host: &Arc<RuntimeCommands>) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !host.recovery_required().unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_session_commands_change_scopes_retire_queued_input_and_replay_receipts_without_switching_again()
 {
    let f = Fixture::new();
    let (host, registry, provider) = host(&f).await;
    let original = host.handshake().unwrap().scope;
    let subscription = host.subscribe().unwrap();
    let prompt = envelope(
        &original,
        "pending",
        4,
        w::Command::Prompt {
            input: w::PromptInput::Inline {
                text: "Never run after switching".into(),
            },
        },
    );
    let w::CommandResult::Accepted {
        operation: prompt_operation,
    } = host.dispatch(prompt.clone()).await.result
    else {
        panic!("prompt admission failed");
    };
    let cancel = envelope(
        &original,
        "cancel-pending",
        4,
        w::Command::Cancel {
            operation: prompt_operation,
        },
    );
    assert!(matches!(
        host.dispatch(cancel.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    let new = session(&original, "new", 4, "new", json!({}));
    assert!(matches!(
        host.dispatch(new.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    let done = settled(&host, &new).await;
    assert_eq!(done.status, w::OperationStatus::Completed);
    let current = host.handshake().unwrap().scope;
    assert_eq!(current.epoch.get(), 2);
    assert_ne!(current.session, original.session);
    assert_eq!(normalized(&f, current.session.as_str()).len(), 2);
    assert!(
        matches!(host.poll(&subscription.id).unwrap(),w::Delivery::Gap{snapshot} if snapshot.scope==current)
    );
    assert_eq!(
        settled(&host, &prompt).await.status,
        w::OperationStatus::Cancelled
    );
    assert_eq!(
        settled(&host, &cancel).await.status,
        w::OperationStatus::Cancelled
    );
    assert!(
        matches!(host.dispatch(new.clone()).await.result,w::CommandResult::Settled{operation} if operation==done)
    );
    assert_eq!(host.handshake().unwrap().scope, current);
    let mut conflict = new.clone();
    conflict.command = w::Command::Session {
        action: "resume".into(),
        args: json!({"session":"first"}),
    };
    assert!(matches!(
        host.dispatch(conflict).await.result,
        w::CommandResult::Rejected {
            error: w::ProtocolError::CommandConflict
        }
    ));
    let resume = session(&current, "resume", 2, "resume", json!({"session":"first"}));
    assert!(matches!(
        host.dispatch(resume.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    assert_eq!(
        settled(&host, &resume).await.status,
        w::OperationStatus::Completed
    );
    let current = host.handshake().unwrap().scope;
    assert_eq!(current.session, original.session);
    assert_eq!(current.epoch.get(), 3);
    let fork = session(&current, "fork", 4, "fork", json!({"entry":"entry-2"}));
    assert!(matches!(
        host.dispatch(fork.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    let done = settled(&host, &fork).await;
    assert_eq!(done.status, w::OperationStatus::Completed);
    let stored = f
        .store
        .command_result(done.operation.as_str())
        .unwrap()
        .unwrap();
    let receipt: SessionReceipt = serde_json::from_slice(stored.result.as_ref().unwrap()).unwrap();
    assert_eq!(receipt.source, current);
    assert_eq!(receipt.selected, host.handshake().unwrap().scope);
    assert_eq!(receipt.selected_entry, Some("entry-2".into()));
    assert_eq!(normalized(&f, receipt.selected.session.as_str()).len(), 2);
    // The runtime is already bound to its one private command queue.
    let selection = registry.selected().unwrap();
    assert!(
        selection
            .services
            .runtime()
            .run(
                input(),
                selection.services.provider().as_ref(),
                selection.services.tools().clone(),
                &ignore
            )
            .await
            .is_err()
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}

struct StageFault {
    boundary: ag1_storage::Boundary,
    remaining: AtomicUsize,
    selection: bool,
}
impl ag1_storage::FaultInjector for StageFault {
    fn check(&self, boundary: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if !self.selection
            && boundary == self.boundary
            && self
                .remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
                == Ok(1)
        {
            return Err(ag1_storage::StorageError::Injected(boundary));
        }
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        boundary: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if self.selection
            && command.starts_with("select-")
            && boundary == self.boundary
            && self.remaining.swap(0, Ordering::AcqRel) > 0
        {
            return Err(ag1_storage::StorageError::Injected(boundary));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admission_selection_and_terminal_faults_keep_commands_blocked_until_explicit_recovery() {
    use ag1_storage::Boundary;
    // Generic writes are admission, bootstrap and terminal; selection has its
    // own targeted boundary. No effects or other commands run in this fixture.
    for (stage, boundary) in [
        (1, Boundary::BeforeCommit),
        (1, Boundary::BeforeAck),
        (3, Boundary::BeforeCommit),
        (3, Boundary::BeforeAck),
        (0, Boundary::BeforeAck),
    ] {
        let fault = Arc::new(StageFault {
            boundary,
            remaining: AtomicUsize::new(0),
            selection: stage == 0,
        });
        let f = Fixture::with_fault(Some(fault.clone()));
        let (host, registry, provider) = host(&f).await;
        let source = host.handshake().unwrap().scope;
        let change = session(&source, "new", 4, "new", json!({}));
        fault.remaining.store(stage.max(1), Ordering::Release);
        let reply = host.dispatch(change.clone()).await;
        if stage == 1 {
            assert!(matches!(reply.result, w::CommandResult::Rejected { .. }));
        } else {
            assert!(matches!(reply.result, w::CommandResult::Accepted { .. }));
        }
        recovery(&host).await;
        assert_eq!(fault.remaining.load(Ordering::Acquire), 0);
        let blocked = host
            .dispatch(envelope(
                &source,
                "blocked",
                4,
                w::Command::Prompt {
                    input: w::PromptInput::Inline {
                        text: "Must wait for custody".into(),
                    },
                },
            ))
            .await;
        assert!(matches!(blocked.result, w::CommandResult::Rejected { .. }));
        assert!(host.run_next(&ignore).await.is_err());
        host.reconcile().await.unwrap();
        assert!(!host.recovery_required().unwrap());
        if stage == 1 {
            assert_eq!(host.handshake().unwrap().scope, source);
            assert_eq!(registry.selected().unwrap().epoch, 1);
            if boundary == Boundary::BeforeAck {
                assert_eq!(
                    settled(&host, &change).await.status,
                    w::OperationStatus::Cancelled
                );
            } else {
                assert!(matches!(
                    host.dispatch(change.clone()).await.result,
                    w::CommandResult::Accepted { .. }
                ));
                assert_eq!(
                    settled(&host, &change).await.status,
                    w::OperationStatus::Completed
                );
            }
        } else {
            assert_eq!(host.handshake().unwrap().scope.epoch.get(), 2);
            assert_eq!(
                settled(&host, &change).await.status,
                w::OperationStatus::Completed
            );
        }
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_wire_caller_leaves_one_owned_driver_to_acknowledge_and_finish_the_switch() {
    let gate = Arc::new(AckGate::default());
    let f = Fixture::with_fault(Some(gate.clone()));
    let (host, _, provider) = host(&f).await;
    let scope = host.handshake().unwrap().scope;
    let change = session(&scope, "fork", 4, "fork", json!({"entry":"entry-2"}));
    gate.armed.store(true, Ordering::Release);
    let owner = host.clone();
    let request = change.clone();
    let task = tokio::spawn(async move { owner.dispatch(request).await });
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    assert!(
        host.snapshot().unwrap().queued.is_empty(),
        "admission has not acknowledged"
    );
    task.abort();
    assert!(matches!(task.await,Err(error) if error.is_cancelled()));
    gate.release();
    assert_eq!(
        settled(&host, &change).await.status,
        w::OperationStatus::Completed
    );
    assert_eq!(host.handshake().unwrap().scope.epoch.get(), 2);
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}
