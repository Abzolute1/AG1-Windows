#![cfg(target_os = "linux")]
//! Real native owners and durable journals, with no model process or desktop.
use ag1_core::{
    DomainError, PortFuture, SessionId,
    context::compiler::CompileOptions,
    inference::Scheduler,
    runtime::{SessionRuntime, session::Configuration},
    storage::StoragePort,
    tools::registry::Scope,
};
use ag1_platform::{
    executor::BlockingPool,
    provider::http::ProviderHttp,
    session::{
        NativeSession, ProviderInput, ProviderResolver, ResolvedRequest, ToolOutcome, ToolRunner,
        ToolSelection, TurnInput,
        registry::{PreparedRuntime, RuntimeFactory, RuntimeRegistry, Target},
    },
    workspace::{Workspace, WorkspaceService},
};
use ag1_storage::{SessionBootstrap, SessionSeed, Store};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;
fn ignore(_: &ag1_platform::session::SessionEvent) {}

#[path = "registry_actions/mod.rs"]
mod registry_actions;
#[path = "registry_selection/mod.rs"]
mod registry_selection;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_cannot_orphan_an_accepted_queue_and_succeeds_after_terminal_custody() {
    use ag1_platform::session::commands::{RunSettings, SessionCommands};
    use ag1_protocol::wire as w;
    let f = Fixture::new();
    let (first, resolver) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let commands = SessionCommands::new(
        registry.selected().unwrap(),
        "queue-owner".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    let envelope = |id: &str, command: w::Command| w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: commands.scope().clone(),
        expected_revision: w::Sequence::new(old.journal_revision().unwrap()),
        command,
        extension: None,
    };
    let reply = commands
        .dispatch(envelope(
            "pending",
            w::Command::Prompt {
                input: w::PromptInput::Inline {
                    text: "Do not lose this prompt".into(),
                },
            },
        ))
        .await;
    let w::CommandResult::Accepted { operation } = reply.result else {
        panic!("{reply:?}")
    };
    let (second, _) = f.prepared("second", 2, false);
    let unused = second.runtime().clone();
    let error = registry
        .replace(1, f.target("second"), &Factory::new(Some(second), false))
        .await;
    assert!(
        matches!(error, Err(DomainError::Unavailable(message)) if message.contains("command queue"))
    );
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(old.accepts_commands().unwrap());
    assert!(!unused.accepts_commands().unwrap());
    assert_eq!(commands.snapshot().unwrap().queued, vec![operation.clone()]);
    let reply = commands
        .dispatch(envelope(
            "cancel",
            w::Command::Cancel {
                operation: operation.clone(),
            },
        ))
        .await;
    assert!(matches!(reply.result, w::CommandResult::Accepted { .. }));
    let terminal = commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(terminal.status, w::OperationStatus::Cancelled);
    assert_eq!(terminal.operation, operation);
    assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
    let (second, _) = f.prepared("second", 2, false);
    registry
        .replace(1, f.target("second"), &Factory::new(Some(second), false))
        .await
        .unwrap();
    assert!(!old.accepts_commands().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_rejects_a_lost_command_owner_instead_of_assuming_an_empty_queue() {
    use ag1_platform::session::commands::{RunSettings, SessionCommands};
    let f = Fixture::new();
    let (first, _) = f.prepared("first", 1, false);
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let commands = SessionCommands::new(
        registry.selected().unwrap(),
        "lost-owner".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    drop(commands);
    let (second, _) = f.prepared("second", 2, false);
    assert!(
        matches!(registry.replace(1, f.target("second"), &Factory::new(Some(second), false)).await,
        Err(DomainError::Unavailable(message)) if message.contains("command queue"))
    );
    assert!(
        registry
            .selected()
            .unwrap()
            .services
            .runtime()
            .accepts_commands()
            .unwrap()
    );
}

struct QueueRetirementFault {
    point: ag1_storage::Boundary,
    armed: AtomicBool,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_queue_retirement_keeps_admission_frozen_until_accepted_receipts_drain() {
    use ag1_platform::session::commands::{RunSettings, SessionCommands};
    use ag1_protocol::wire as w;
    let gate = Arc::new(AckGate::default());
    let f = Fixture::with_fault(Some(gate.clone()));
    let (first, resolver) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = Arc::new(RuntimeRegistry::new(first, 1).unwrap());
    let commands = SessionCommands::new(
        registry.selected().unwrap(),
        "dropped-retirement".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    let reply = commands
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: "pending".to_owned().try_into().unwrap(),
            scope: commands.scope().clone(),
            expected_revision: w::Sequence::new(0),
            extension: None,
            command: w::Command::Prompt {
                input: w::PromptInput::Inline {
                    text: "Retain cancellation".into(),
                },
            },
        })
        .await;
    let w::CommandResult::Accepted { operation } = reply.result else {
        panic!("{reply:?}")
    };
    let (second, _) = f.prepared("second", 2, false);
    let unused = second.runtime().clone();
    gate.armed.store(true, Ordering::Release);
    let owner = registry.clone();
    let target = f.target("second");
    let replacement = tokio::spawn(async move {
        owner
            .replace_cancelling_pending(1, target, &Factory::new(Some(second), false))
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    replacement.abort();
    assert!(matches!(replacement.await, Err(error) if error.is_cancelled()));
    assert!(!old.accepts_commands().unwrap());
    assert!(commands.run_next(&ignore).await.is_err());
    assert!(!unused.accepts_commands().unwrap());
    gate.release();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !old.accepts_commands().unwrap() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(commands.snapshot().unwrap().queued.is_empty());
    let receipt = f.store.command_result(operation.as_str()).unwrap().unwrap();
    assert_eq!(receipt.state, "completed");
    assert_eq!(
        serde_json::from_slice::<Value>(receipt.result.as_ref().unwrap()).unwrap()["operation"]["status"],
        "cancelled"
    );
    assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
}
impl ag1_storage::FaultInjector for QueueRetirementFault {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point == self.point && self.armed.swap(false, Ordering::AcqRel) {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_queue_retirement_acknowledges_every_prompt_and_retries_lost_receipts_without_execution()
 {
    use ag1_platform::session::commands::{RunSettings, SessionCommands};
    use ag1_protocol::wire as w;
    for point in [
        None,
        Some(ag1_storage::Boundary::BeforeCommit),
        Some(ag1_storage::Boundary::BeforeAck),
    ] {
        let fault = Arc::new(QueueRetirementFault {
            point: point.unwrap_or(ag1_storage::Boundary::BeforeCommit),
            armed: AtomicBool::new(false),
        });
        let f = Fixture::with_fault(Some(fault.clone()));
        let (first, resolver) = f.prepared("first", 1, false);
        let old = first.runtime().clone();
        let registry = RuntimeRegistry::new(first, 1).unwrap();
        let commands = SessionCommands::new(
            registry.selected().unwrap(),
            "retiring".to_owned().try_into().unwrap(),
            Arc::new(FixedClock),
            RunSettings::default(),
        )
        .unwrap();
        let mut operations = vec![];
        for id in ["prompt", "steer", "follow_up"] {
            let input = w::PromptInput::Inline {
                text: "Never execute this retired request".into(),
            };
            let command = if id == "prompt" {
                w::Command::Prompt { input }
            } else {
                w::Command::Session {
                    action: id.into(),
                    args: json!({"input":input}),
                }
            };
            let reply = commands
                .dispatch(w::CommandEnvelope {
                    version: Default::default(),
                    id: id.to_owned().try_into().unwrap(),
                    scope: commands.scope().clone(),
                    expected_revision: w::Sequence::new(0),
                    command,
                    extension: None,
                })
                .await;
            let w::CommandResult::Accepted { operation } = reply.result else {
                panic!("{reply:?}")
            };
            operations.push(operation);
        }
        let (second, _) = f.prepared("second", 2, false);
        fault.armed.store(point.is_some(), Ordering::Release);
        let result = registry
            .replace_cancelling_pending(1, f.target("second"), &Factory::new(Some(second), false))
            .await;
        if point.is_some() {
            assert!(result.is_err());
            assert_eq!(registry.selected().unwrap().epoch, 1);
            assert!(old.accepts_commands().unwrap());
            assert_eq!(commands.snapshot().unwrap().queued, operations);
            assert!(matches!(
                commands.run_next(&ignore).await,
                Err(DomainError::Conflict)
            ));
            old.select_thinking(
                "after-retirement-failure",
                "after-retirement-failure",
                NOW,
                "high".into(),
                &ignore,
            )
            .await
            .unwrap();
            let (second, _) = f.prepared("second", 2, false);
            registry
                .replace_cancelling_pending(
                    1,
                    f.target("second"),
                    &Factory::new(Some(second), false),
                )
                .await
                .unwrap();
        } else {
            result.unwrap();
        }
        assert_eq!(registry.selected().unwrap().epoch, 2);
        assert!(!old.accepts_commands().unwrap());
        assert!(commands.snapshot().unwrap().queued.is_empty());
        for operation in &operations {
            let record = f.store.command_result(operation.as_str()).unwrap().unwrap();
            assert_eq!(record.state, "completed");
            let result: Value = serde_json::from_slice(record.result.as_ref().unwrap()).unwrap();
            assert_eq!(result["operation"]["status"], "cancelled");
            assert_eq!(result["generations"], 0);
            assert_eq!(result["execution_settled"], true);
        }
        let journal = f
            .store
            .journal_snapshot(&SessionId::new("first").unwrap())
            .unwrap();
        assert!(
            !ag1_core::task::branch_values(&journal)
                .unwrap()
                .iter()
                .any(|e| e["message"]["role"] == "user")
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
    }
}
const NOW: &str = "2026-09-06T00:00:00.000Z";
struct FixedClock;
impl ag1_core::Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        1000
    }
}
#[derive(Default)]
struct AckGate {
    armed: AtomicBool,
    entered: Notify,
    released: Mutex<bool>,
    changed: std::sync::Condvar,
}
impl AckGate {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
}
impl ag1_storage::FaultInjector for AckGate {
    fn check(&self, boundary: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if boundary == ag1_storage::Boundary::BeforeAck && self.armed.swap(false, Ordering::SeqCst)
        {
            self.entered.notify_one();
            let (released, _) = self
                .changed
                .wait_timeout_while(
                    self.released.lock().unwrap(),
                    Duration::from_secs(5),
                    |released| !*released,
                )
                .unwrap();
            if !*released {
                return Err(ag1_storage::StorageError::Injected(boundary));
            }
        }
        Ok(())
    }
}
struct NoTools;
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
        if command == "failed-control" && point == self.0 {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drained_but_unacknowledged_control_blocks_runtime_replacement() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let f = Fixture::with_fault(Some(Arc::new(JournalFault(point))));
        let (first, _) = f.prepared("first", 1, false);
        let old = first.runtime().clone();
        let registry = RuntimeRegistry::new(first, 1).unwrap();
        let (second, _) = f.prepared("second", 2, false);
        let unused = second.runtime().clone();
        let factory = Factory::new(Some(second), false);
        assert!(
            old.select_thinking(
                "failed-control",
                "failed-control",
                NOW,
                "high".into(),
                &ignore
            )
            .await
            .is_err()
        );
        assert!(old.settled().unwrap());
        assert!(!old.persistence_healthy().unwrap());
        assert!(matches!(
            registry.replace(1, f.target("second"), &factory).await,
            Err(DomainError::Unavailable(_))
        ));
        assert_eq!(registry.selected().unwrap().epoch, 1);
        assert!(old.accepts_commands().unwrap());
        assert!(!unused.accepts_commands().unwrap());
    }
}
struct QuiescingTools(Arc<AckGate>);
struct FailedQuiescence;
impl ToolRunner for FailedQuiescence {
    fn quiesce(&self, _: &Scope, _: &str) -> Result<(), DomainError> {
        Err(DomainError::DurabilityUnknown)
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
        panic!("no tool batch expected")
    }
}
impl ToolRunner for QuiescingTools {
    fn quiesce(&self, _: &Scope, _: &str) -> Result<(), DomainError> {
        ag1_storage::FaultInjector::check(self.0.as_ref(), ag1_storage::Boundary::BeforeAck)
            .map_err(DomainError::from)
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
        panic!("no tool batch expected")
    }
}
impl ToolRunner for NoTools {
    fn execute(
        &self,
        _: &Scope,
        _: &[ag1_core::runtime::session::PendingCall],
        _: &ToolSelection,
        _: &str,
        _: &AtomicBool,
        _: &AtomicU64,
    ) -> Result<Vec<ToolOutcome>, DomainError> {
        panic!("fixture must not execute tools")
    }
}
struct Resolver {
    calls: AtomicUsize,
    entered: Notify,
    release: Notify,
    pause: bool,
    dropped: Arc<AtomicUsize>,
}
impl Drop for Resolver {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl ProviderResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        _: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::AcqRel);
            self.entered.notify_one();
            if self.pause {
                self.release.notified().await;
            }
            assert!(cancel.load(Ordering::Acquire));
            Err(DomainError::Cancelled)
        })
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<Store>,
    root: Arc<Workspace>,
    pool: Arc<BlockingPool>,
    scheduler: Scheduler,
    drops: Arc<AtomicUsize>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_fault(None)
    }
    fn with_fault(fault: Option<Arc<dyn ag1_storage::FaultInjector>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("root");
        std::fs::create_dir(&path).unwrap();
        let root = WorkspaceService::new().unwrap().register(&path).unwrap();
        let store = Arc::new(
            match fault {
                Some(fault) => {
                    Store::open_with(&dir.path().join("store"), Arc::new(FixedClock), fault)
                }
                None => Store::open(&dir.path().join("store")),
            }
            .unwrap(),
        );
        for name in ["first", "second"] {
            let header = serde_json::to_vec(
                &json!({"type":"session","version":3,"id":name,"cwd":path,"timestamp":NOW}),
            )
            .unwrap();
            store
                .bootstrap_session(SessionBootstrap {
                    command: &format!("create-{name}"),
                    session: name,
                    origin: name,
                    cursor_key: &[7; 32],
                    seed: SessionSeed::Fresh {
                        header: &header,
                        records: &[],
                    },
                })
                .unwrap();
        }
        Self {
            _dir: dir,
            store,
            root,
            pool: Arc::new(BlockingPool::new(1, 16).unwrap()),
            scheduler: Scheduler::default(),
            drops: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn target(&self, name: &str) -> Target {
        Target {
            workspace: self.root.id().as_str().into(),
            session: SessionId::new(name).unwrap(),
        }
    }
    fn prepared(&self, name: &str, epoch: u64, pause: bool) -> (PreparedRuntime, Arc<Resolver>) {
        self.prepared_with_tools(name, epoch, pause, Arc::new(NoTools))
    }
    fn prepared_with_tools(
        &self,
        name: &str,
        epoch: u64,
        pause: bool,
        tools: Arc<dyn ToolRunner>,
    ) -> (PreparedRuntime, Arc<Resolver>) {
        let state = SessionRuntime::resume(
            SessionId::new(name).unwrap(),
            epoch,
            &self
                .store
                .journal_snapshot(&SessionId::new(name).unwrap())
                .unwrap(),
            Configuration {
                model: model("initial"),
                thinking: "off".into(),
                tools: vec![],
                system_prompt: "Native registry fixture".into(),
                prompt: None,
                compiler: CompileOptions::default(),
            },
        )
        .unwrap();
        let runtime = NativeSession::new(
            state,
            self.store.clone(),
            name.into(),
            self.root.clone(),
            self.pool.clone(),
            self.scheduler.clone(),
            ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap(),
        )
        .unwrap();
        let resolver = Arc::new(Resolver {
            calls: AtomicUsize::new(0),
            entered: Notify::new(),
            release: Notify::new(),
            pause,
            dropped: self.drops.clone(),
        });
        (
            PreparedRuntime::new(runtime, resolver.clone(), tools),
            resolver,
        )
    }
}
fn model(id: &str) -> Value {
    json!({"provider":"fixture","id":id,"api":"openai-completions","baseUrl":"http://127.0.0.1:1","contextWindow":32768,"maxTokens":4096,"input":["text"],"reasoning":false})
}
fn input() -> TurnInput {
    TurnInput {
        command: "prompt".into(),
        message: json!({"role":"user","content":[{"type":"text","text":"Investigate"}],"timestamp":1000}),
        timestamp: NOW.into(),
        max_generations: Some(1),
        compaction: ag1_core::context::compaction::Settings {
            enabled: false,
            ..Default::default()
        },
        compaction_timeout_ms: 900000,
        retry: Default::default(),
    }
}
struct Factory {
    result: Mutex<Option<PreparedRuntime>>,
    entered: Notify,
    release: Notify,
    pause: bool,
    calls: AtomicUsize,
}
impl Factory {
    fn new(result: Option<PreparedRuntime>, pause: bool) -> Self {
        Self {
            result: Mutex::new(result),
            entered: Notify::new(),
            release: Notify::new(),
            pause,
            calls: AtomicUsize::new(0),
        }
    }
}
impl RuntimeFactory for Factory {
    fn prepare<'a>(&'a self, _: &'a Target, _: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            if self.pause {
                self.release.notified().await;
            }
            self.result
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| DomainError::Unavailable("fixture preparation failure".into()))
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preparation_failure_and_stale_epoch_leave_selected_owner_usable() {
    let f = Fixture::new();
    let (first, _) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let factory = Factory::new(None, false);
    assert!(matches!(
        registry.replace(0, f.target("second"), &factory).await,
        Err(DomainError::Conflict)
    ));
    assert_eq!(factory.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        registry.replace(1, f.target("second"), &factory).await,
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert_eq!(old.journal_revision().unwrap(), 0);
    old.select_model("change", "change", NOW, model("new"), &ignore)
        .await
        .unwrap();
    assert_eq!(old.journal_revision().unwrap(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn successful_replacement_retires_leaked_old_handles_and_releases_services() {
    let f = Fixture::new();
    let cwd = std::env::current_dir().unwrap();
    let (first, provider) = f.prepared("first", 1, false);
    drop(provider);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let (second, provider) = f.prepared("second", 2, false);
    drop(provider);
    let factory = Factory::new(Some(second), false);
    let next = registry
        .replace(1, f.target("second"), &factory)
        .await
        .unwrap();
    assert_eq!(next.epoch, 2);
    assert_eq!(f.drops.load(Ordering::SeqCst), 1);
    assert!(matches!(
        old.select_model("late", "late", NOW, model("late"), &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        old.select_tools("late-tools", "late-tools", NOW, vec![], &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        old.run(
            input(),
            next.services.provider().as_ref(),
            next.services.tools().clone(),
            &ignore
        )
        .await,
        Err(DomainError::Conflict)
    ));
    old.cancel().unwrap(); // Cancellation cannot reactivate a retired owner.
    assert!(matches!(
        old.select_model("again", "again", NOW, model("late"), &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
    next.services
        .runtime()
        .select_model("new", "new", NOW, model("selected"), &ignore)
        .await
        .unwrap();
    assert_eq!(old.journal_revision().unwrap(), 0);
    assert_eq!(next.services.runtime().journal_revision().unwrap(), 1);
    assert!(matches!(
        old.select_thinking(
            "late-thinking",
            "late-thinking",
            NOW,
            "high".into(),
            &ignore
        )
        .await,
        Err(DomainError::Conflict)
    ));
    next.services
        .runtime()
        .select_thinking("thinking", "thinking", NOW, "high".into(), &ignore)
        .await
        .unwrap();
    assert_eq!(
        next.services.runtime().configuration().unwrap().thinking,
        "high"
    );
    assert_eq!(next.services.runtime().journal_revision().unwrap(), 2);
    assert_eq!(std::env::current_dir().unwrap(), cwd);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_prepares_first_then_fences_and_drains_an_actual_native_turn() {
    let f = Fixture::new();
    let (first, provider) = f.prepared("first", 1, true);
    let old = first.runtime().clone();
    let registry = Arc::new(RuntimeRegistry::new(first, 1).unwrap());
    let running = registry.selected().unwrap().services;
    let turn = tokio::spawn(async move {
        running
            .runtime()
            .run(
                input(),
                running.provider().as_ref(),
                running.tools().clone(),
                &ignore,
            )
            .await
    });
    provider.entered.notified().await;
    let (second, _) = f.prepared("second", 2, false);
    let factory = Arc::new(Factory::new(Some(second), true));
    let replacement = {
        let registry = registry.clone();
        let factory = factory.clone();
        let target = f.target("second");
        tokio::spawn(async move { registry.replace(1, target, factory.as_ref()).await })
    };
    factory.entered.notified().await;
    // Old owner is still admitted while its replacement is being prepared.
    old.select_model(
        "during-prepare",
        "during-prepare",
        NOW,
        model("updated"),
        &ignore,
    )
    .await
    .unwrap();
    assert!(matches!(
        registry
            .replace(1, f.target("second"), &Factory::new(None, false))
            .await,
        Err(DomainError::Conflict)
    ));
    factory.release.notify_one();
    // Observe the admission fence without submitting any new journal write.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if !old.accepts_commands().unwrap() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Resolver preparation grants no effects. Cancellation must not require
    // this indefinitely paused resolver to produce a response before cutover.
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), turn)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    let next = tokio::time::timeout(Duration::from_secs(2), replacement)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(next.epoch, 2);
    assert!(old.settled().unwrap());
    assert!(matches!(
        old.select_model("retired", "retired", NOW, model("bad"), &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_prepared_journal_is_disposed_and_does_not_replace_current_owner() {
    let f = Fixture::new();
    let (first, _) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let (second, provider) = f.prepared("second", 2, false);
    drop(provider);
    let unused = second.runtime().clone();
    let factory = Factory::new(Some(second), false);
    let row=serde_json::to_vec(&json!({"type":"thinking_level_change","id":"advance","parentId":null,"timestamp":NOW,"thinkingLevel":"low"})).unwrap();
    f.store
        .append_batch("second", "advance", 0, &[row])
        .unwrap();
    assert!(matches!(
        registry.replace(1, f.target("second"), &factory).await,
        Err(DomainError::Conflict)
    ));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert_eq!(f.drops.load(Ordering::SeqCst), 1);
    assert!(matches!(
        unused
            .select_model("unused", "unused", NOW, model("bad"), &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
    old.select_model("old", "old", NOW, model("okay"), &ignore)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_waits_for_real_sql_acknowledgement_on_one_worker() {
    let gate = Arc::new(AckGate::default());
    let f = Fixture::with_fault(Some(gate.clone()));
    let (first, _) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = Arc::new(RuntimeRegistry::new(first, 1).unwrap());
    let (second, _) = f.prepared("second", 2, false);
    let factory = Arc::new(Factory::new(Some(second), false));
    gate.armed.store(true, Ordering::SeqCst);
    let control = {
        let old = old.clone();
        tokio::spawn(async move {
            old.select_model("saved", "saved", NOW, model("saved"), &ignore)
                .await
        })
    };
    gate.entered.notified().await;
    let replacement = {
        let registry = registry.clone();
        let factory = factory.clone();
        let target = f.target("second");
        tokio::spawn(async move { registry.replace(1, target, factory.as_ref()).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while old.accepts_commands().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(old.journal_revision().unwrap(), 0);
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(!replacement.is_finished());
    assert!(matches!(
        old.select_model("late", "late", NOW, model("late"), &ignore)
            .await,
        Err(DomainError::Conflict)
    ));
    gate.release();
    control.await.unwrap().unwrap();
    assert_eq!(replacement.await.unwrap().unwrap().epoch, 2);
    assert_eq!(old.journal_revision().unwrap(), 1);
    let saved = f
        .store
        .journal_snapshot(&SessionId::new("first").unwrap())
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        serde_json::from_slice::<Value>(&saved.entries[0].raw).unwrap()["modelId"],
        "saved"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_replacement_during_journal_drain_thaws_old_and_retires_unused_preparation() {
    let gate = Arc::new(AckGate::default());
    let f = Fixture::with_fault(Some(gate.clone()));
    let (first, _) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = Arc::new(RuntimeRegistry::new(first, 1).unwrap());
    let (second, next_provider) = f.prepared("second", 2, false);
    drop(next_provider);
    let unused = second.runtime().clone();
    let factory = Arc::new(Factory::new(Some(second), false));
    gate.armed.store(true, Ordering::SeqCst);
    let writer = old.clone();
    let write = tokio::spawn(async move {
        writer
            .select_thinking(
                "accepted-write",
                "accepted-write",
                NOW,
                "high".into(),
                &ignore,
            )
            .await
    });
    gate.entered.notified().await;
    let replacement = {
        let registry = registry.clone();
        let factory = factory.clone();
        let target = f.target("second");
        tokio::spawn(async move { registry.replace(1, target, factory.as_ref()).await })
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while old.accepts_commands().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    replacement.abort();
    assert!(matches!(replacement.await, Err(error) if error.is_cancelled()));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(old.accepts_commands().unwrap());
    assert!(!unused.accepts_commands().unwrap());
    assert_eq!(f.drops.load(Ordering::SeqCst), 1);
    gate.release();
    write.await.unwrap().unwrap();
    old.select_model("after-abort", "after-abort", NOW, model("usable"), &ignore)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_control_receipt_requires_recovery_instead_of_retiring_the_owner() {
    let gate = Arc::new(AckGate::default());
    let f = Fixture::with_fault(Some(gate.clone()));
    let (first, _) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let (second, _) = f.prepared("second", 2, false);
    let unused = second.runtime().clone();
    let factory = Factory::new(Some(second), false);
    gate.armed.store(true, Ordering::SeqCst);
    let control = {
        let old = old.clone();
        tokio::spawn(async move {
            old.select_model("lost", "lost", NOW, model("saved"), &ignore)
                .await
        })
    };
    gate.entered.notified().await;
    control.abort();
    assert!(control.await.unwrap_err().is_cancelled());
    assert!(matches!(
        registry.replace(1, f.target("second"), &factory).await,
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(!unused.accepts_commands().unwrap());
    assert!(!old.settled().unwrap());
    gate.release();
    let store = f.store.clone();
    let saved = f
        .pool
        .submit(move || store.journal_snapshot(&SessionId::new("first").unwrap()))
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(old.journal_revision().unwrap(), 0);
    assert!(!old.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_replacement_keeps_worker_owned_background_settlement_fenced() {
    let f = Fixture::new();
    let gate = Arc::new(AckGate::default());
    gate.armed.store(true, Ordering::SeqCst);
    let (first, _) =
        f.prepared_with_tools("first", 1, false, Arc::new(QuiescingTools(gate.clone())));
    let old = first.runtime().clone();
    let registry = Arc::new(RuntimeRegistry::new(first, 1).unwrap());
    let (second, _) = f.prepared("second", 2, false);
    let unused = second.runtime().clone();
    let factory = Arc::new(Factory::new(Some(second), false));
    let replacement = {
        let registry = registry.clone();
        let factory = factory.clone();
        let target = f.target("second");
        tokio::spawn(async move { registry.replace(1, target, factory.as_ref()).await })
    };
    gate.entered.notified().await;
    replacement.abort();
    assert!(matches!(replacement.await,Err(error) if error.is_cancelled()));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(
        !old.accepts_commands().unwrap(),
        "worker retains admission fence after async cancellation"
    );
    assert!(!unused.accepts_commands().unwrap());
    gate.release();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !old.accepts_commands().unwrap() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    old.select_model(
        "after-worker",
        "after-worker",
        NOW,
        model("usable"),
        &ignore,
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_background_settlement_keeps_old_selected_and_discards_new_services() {
    let f = Fixture::new();
    let (first, _) = f.prepared_with_tools("first", 1, false, Arc::new(FailedQuiescence));
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let (second, _) = f.prepared("second", 2, false);
    let unused = second.runtime().clone();
    assert!(matches!(
        registry
            .replace(1, f.target("second"), &Factory::new(Some(second), false))
            .await,
        Err(DomainError::DurabilityUnknown)
    ));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(old.accepts_commands().unwrap());
    assert!(!unused.accepts_commands().unwrap());
    old.select_model(
        "still-selected",
        "still-selected",
        NOW,
        model("usable"),
        &ignore,
    )
    .await
    .unwrap();
}

struct Services {
    workspace: Arc<Workspace>,
    provider: Arc<Resolver>,
    wrong_model: bool,
    seen: Mutex<Vec<(String, u64)>>,
}
impl ag1_platform::session::registry::ServiceFactory for Services {
    fn prepare<'a>(
        &'a self,
        input: ag1_platform::session::registry::ServiceInput<'a>,
    ) -> PortFuture<'a, ag1_platform::session::registry::ScopedServices> {
        Box::pin(async move {
            self.seen
                .lock()
                .unwrap()
                .push((input.target.session.as_str().into(), input.journal.revision));
            let branch = ag1_core::task::branch_values(input.journal)?;
            assert!(branch.iter().all(|entry| entry["type"] != "message"));
            let selected = branch
                .iter()
                .rev()
                .find(|entry| entry["type"] == "model_change")
                .and_then(|entry| entry["modelId"].as_str())
                .unwrap_or("initial");
            Ok(ag1_platform::session::registry::ScopedServices {
                workspace: self.workspace.clone(),
                origin: input.target.session.as_str().into(),
                configuration: Configuration {
                    model: model(if self.wrong_model { "wrong" } else { selected }),
                    thinking: "off".into(),
                    tools: vec![],
                    system_prompt: "Scoped native services".into(),
                    prompt: None,
                    compiler: CompileOptions::default(),
                },
                provider: self.provider.clone(),
                tools: Arc::new(NoTools),
            })
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_factory_resolves_zero_message_configuration_and_rejects_wrong_model_before_switch()
{
    use ag1_platform::session::registry::StoredRuntimeFactory;
    let f = Fixture::new();
    let (first, provider) = f.prepared("first", 1, false);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::new(first, 1).unwrap();
    let records=vec![
        json!({"type":"model_change","id":"selected","parentId":null,"timestamp":NOW,"provider":"fixture","modelId":"persisted"}),
        json!({"type":"thinking_level_change","id":"thinking","parentId":"selected","timestamp":NOW,"thinkingLevel":"high"}),
        json!({"type":"custom","id":"tools","parentId":"thinking","timestamp":NOW,"customType":"ag1_active_tools","data":[]}),
    ].into_iter().map(|r|serde_json::to_vec(&r).unwrap()).collect::<Vec<_>>();
    f.store
        .append_batch("second", "configure-second", 0, &records)
        .unwrap();
    let services = Arc::new(Services {
        workspace: f.root.clone(),
        provider: provider.clone(),
        wrong_model: false,
        seen: Mutex::new(vec![]),
    });
    let mut factory = StoredRuntimeFactory {
        store: f.store.clone(),
        pool: f.pool.clone(),
        scheduler: f.scheduler.clone(),
        http: ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap(),
        services: Arc::new(Services {
            workspace: f.root.clone(),
            provider,
            wrong_model: true,
            seen: Mutex::new(vec![]),
        }),
    };
    assert!(matches!(
        registry.replace(1, f.target("second"), &factory).await,
        Err(DomainError::InvalidInput(_))
    ));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert_eq!(old.journal_revision().unwrap(), 0);
    assert!(old.accepts_commands().unwrap());
    factory.services = services.clone();
    let next = registry
        .replace(1, f.target("second"), &factory)
        .await
        .unwrap();
    let selected = next.services.runtime().configuration().unwrap();
    assert_eq!(selected.model["id"], "persisted");
    assert_eq!(selected.thinking, "high");
    assert!(selected.tools.is_empty());
    assert_eq!(next.services.runtime().journal_revision().unwrap(), 3);
    assert_eq!(*services.seen.lock().unwrap(), vec![("second".into(), 3)]);
    assert_eq!(next.epoch, 2);
}
