use super::*;
#[path = "commands.rs"]
mod commands;
use ag1_platform::session::registry::{
    ScopedServices, ServiceFactory, ServiceInput, SessionAction, SessionRequest,
    StoredRuntimeFactory,
};

fn oracle() -> Value {
    serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/session-lifecycle-oracle.json"
    ))
    .unwrap()
}
fn row(fixture: &Value, scenario: &str) -> Value {
    fixture["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["scenario"] == scenario)
        .unwrap()
        .clone()
}
fn populate(f: &Fixture, session: &str, messages: &Value) {
    let mut parent = Value::Null;
    let records=messages.as_array().unwrap().iter().enumerate().map(|(i,message)| {
        let id=format!("entry-{i}");
        let record=json!({"type":"message","id":id,"parentId":parent,"timestamp":NOW,"message":message["message"]});
        parent=json!(id);
        serde_json::to_vec(&record).unwrap()
    }).collect::<Vec<_>>();
    f.store
        .append_batch(session, &format!("populate-{session}"), 0, &records)
        .unwrap();
}
fn parent(f: &Fixture) -> (PreparedRuntime, Arc<Resolver>) {
    populate(f, "first", &row(&oracle(), "invalid-fork")["branch"]);
    let (_, provider) = f.prepared("first", 1, false);
    let state = SessionRuntime::resume(
        SessionId::new("first").unwrap(),
        1,
        &f.store
            .journal_snapshot(&SessionId::new("first").unwrap())
            .unwrap(),
        Configuration {
            model: model("selected"),
            thinking: "high".into(),
            tools: vec![],
            system_prompt: "Lifecycle reference fixture".into(),
            prompt: None,
            compiler: CompileOptions::default(),
        },
    )
    .unwrap();
    let native = NativeSession::new(
        state,
        f.store.clone(),
        "first".into(),
        f.root.clone(),
        f.pool.clone(),
        f.scheduler.clone(),
        ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap(),
    )
    .unwrap();
    (
        PreparedRuntime::new(native, provider.clone(), Arc::new(NoTools)),
        provider,
    )
}
struct ActionServices {
    workspace: Arc<Workspace>,
    provider: Arc<Resolver>,
}
impl ServiceFactory for ActionServices {
    fn prepare<'a>(&'a self, input: ServiceInput<'a>) -> PortFuture<'a, ScopedServices> {
        Box::pin(async move {
            let branch = ag1_core::task::branch_values(input.journal)?;
            let selected = branch
                .iter()
                .rev()
                .find(|v| v["type"] == "model_change")
                .and_then(|v| v["modelId"].as_str())
                .unwrap_or("default");
            let thinking = branch
                .iter()
                .rev()
                .find(|v| v["type"] == "thinking_level_change")
                .and_then(|v| v["thinkingLevel"].as_str())
                .unwrap_or("off");
            Ok(ScopedServices {
                workspace: self.workspace.clone(),
                origin: input.target.session.as_str().into(),
                configuration: Configuration {
                    model: model(selected),
                    thinking: thinking.into(),
                    tools: vec![],
                    system_prompt: "Prepared lifecycle fixture".into(),
                    prompt: None,
                    compiler: CompileOptions::default(),
                },
                provider: self.provider.clone(),
                tools: Arc::new(NoTools),
            })
        })
    }
}
fn factory(f: &Fixture, provider: Arc<Resolver>) -> StoredRuntimeFactory {
    StoredRuntimeFactory {
        store: f.store.clone(),
        pool: f.pool.clone(),
        scheduler: f.scheduler.clone(),
        http: ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap(),
        services: Arc::new(ActionServices {
            workspace: f.root.clone(),
            provider,
        }),
    }
}
fn request(action: SessionAction, target: &str) -> SessionRequest {
    SessionRequest {
        command: "bootstrap-next".into(),
        target: SessionId::new(target).unwrap(),
        expected_revision: 4,
        timestamp: NOW.into(),
        action,
    }
}
fn normalized(f: &Fixture, session: &str) -> Vec<Value> {
    f.store
        .branch(session, None)
        .unwrap()
        .into_iter()
        .map(|r| {
            let v: Value = serde_json::from_slice(&r.normalized).unwrap();
            match v["type"].as_str().unwrap() {
                "message" => json!({"type":"message","message":v["message"]}),
                "model_change" => {
                    json!({"type":"model_change","provider":v["provider"],"modelId":v["modelId"]})
                }
                "thinking_level_change" => {
                    json!({"type":"thinking_level_change","thinkingLevel":v["thinkingLevel"]})
                }
                other => panic!("unexpected {other}"),
            }
        })
        .collect()
}

struct PausedPreparation {
    delegate: StoredRuntimeFactory,
    prepared: Mutex<Option<Arc<NativeSession>>>,
    entered: Notify,
    release: Notify,
}
impl RuntimeFactory for PausedPreparation {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            let prepared = self.delegate.prepare(target, epoch).await?;
            *self.prepared.lock().unwrap() = Some(prepared.runtime().clone());
            self.entered.notify_one();
            self.release.notified().await;
            Ok(prepared)
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_configuration_changed_during_preparation_prevents_stale_new_session_selection() {
    let f = Fixture::new();
    let (first, provider) = parent(&f);
    let old = first.runtime().clone();
    let registry =
        Arc::new(RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap());
    registry.reconcile_selection().await.unwrap();
    let factory = Arc::new(PausedPreparation {
        delegate: factory(&f, provider.clone()),
        prepared: Mutex::new(None),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let owner = registry.clone();
    let next_factory = factory.clone();
    let task = tokio::spawn(async move {
        owner
            .change_session(
                1,
                request(SessionAction::New, "created"),
                next_factory.as_ref(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), factory.entered.notified())
        .await
        .unwrap();
    old.select_model("advance", "advance", NOW, model("changed"), &ignore)
        .await
        .unwrap();
    factory.release.notify_one();
    assert!(matches!(task.await.unwrap(), Err(DomainError::Conflict)));
    assert_eq!(registry.selected().unwrap().epoch, 1);
    assert!(old.accepts_commands().unwrap());
    assert!(
        !factory
            .prepared
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .accepts_commands()
            .unwrap()
    );
    assert_eq!(old.configuration().unwrap().model["id"], "changed");
    assert_eq!(
        f.store
            .runtime_selection(f.root.id().as_str())
            .unwrap()
            .unwrap()
            .selection
            .session,
        "first"
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_resume_and_user_forks_match_source_branch_configuration_and_selected_text() {
    let fixture = oracle();
    for scenario in ["new", "resume", "fork-first", "fork-later"] {
        let f = Fixture::new();
        let (first, provider) = parent(&f);
        let old = first.runtime().clone();
        let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
        registry.reconcile_selection().await.unwrap();
        let factory = factory(&f, provider.clone());
        let action = match scenario {
            "new" => SessionAction::New,
            "resume" => {
                populate(&f, "second", &row(&fixture, "resume")["branch"]);
                SessionAction::Resume
            }
            "fork-first" => SessionAction::Fork {
                entry: "entry-0".into(),
            },
            _ => SessionAction::Fork {
                entry: "entry-2".into(),
            },
        };
        let target = if scenario == "resume" {
            "second"
        } else {
            "created"
        };
        let outcome = registry
            .change_session(1, request(action, target), &factory)
            .await
            .unwrap();
        let expected = row(&fixture, scenario);
        assert_eq!(
            json!(normalized(&f, target)),
            expected["branch"],
            "{scenario}"
        );
        let config = outcome.selected.services.runtime().configuration().unwrap();
        assert_eq!(config.model["id"], expected["model"]);
        assert_eq!(config.thinking, expected["thinking"]);
        assert_eq!(
            json!(outcome.selected_text),
            expected["result"]
                .get("selectedText")
                .cloned()
                .unwrap_or(Value::Null)
        );
        assert_eq!(outcome.selected.epoch, 2);
        assert!(!old.accepts_commands().unwrap());
        assert_eq!(
            f.store
                .runtime_selection(f.root.id().as_str())
                .unwrap()
                .unwrap()
                .selection
                .session,
            target
        );
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
        assert_eq!(
            std::env::current_dir().unwrap(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_fork_missing_or_foreign_resume_and_stale_source_cannot_replace_the_owner() {
    let f = Fixture::new();
    let (first, provider) = parent(&f);
    let old = first.runtime().clone();
    let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
    registry.reconcile_selection().await.unwrap();
    let factory = factory(&f, provider.clone());
    for request in [
        request(
            SessionAction::Fork {
                entry: "entry-1".into(),
            },
            "created",
        ),
        request(
            SessionAction::Fork {
                entry: "absent".into(),
            },
            "created",
        ),
        request(SessionAction::Resume, "absent"),
        SessionRequest {
            expected_revision: 3,
            ..request(SessionAction::New, "created")
        },
    ] {
        assert!(registry.change_session(1, request, &factory).await.is_err());
        assert!(old.accepts_commands().unwrap());
        assert_eq!(registry.selected().unwrap().epoch, 1);
        assert!(f.store.snapshot("created").is_err());
    }
    f.store.create_session("foreign",&serde_json::to_vec(&json!({"type":"session","version":3,"id":"foreign","cwd":"/unrelated-workspace","timestamp":NOW})).unwrap()).unwrap();
    f.store
        .register_origin("foreign", "foreign", &[8; 32])
        .unwrap();
    assert!(matches!(
        registry
            .change_session(1, request(SessionAction::Resume, "foreign"), &factory)
            .await,
        Err(DomainError::PermissionDenied)
    ));
    assert!(old.accepts_commands().unwrap());
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_bootstrap_ack_retries_the_same_target_without_replaying_or_selecting_a_partial_session()
 {
    for boundary in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(QueueRetirementFault {
            point: boundary,
            armed: AtomicBool::new(false),
        });
        let f = Fixture::with_fault(Some(fault.clone()));
        let (first, provider) = parent(&f);
        let old = first.runtime().clone();
        let registry = RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap();
        registry.reconcile_selection().await.unwrap();
        let factory = factory(&f, provider.clone());
        let request = request(SessionAction::New, "created");
        fault.armed.store(true, Ordering::Release);
        assert!(
            registry
                .change_session(1, request.clone(), &factory)
                .await
                .is_err()
        );
        assert!(!fault.armed.load(Ordering::Acquire));
        assert!(old.accepts_commands().unwrap());
        assert_eq!(registry.selected().unwrap().epoch, 1);
        assert_eq!(
            f.store.snapshot("created").is_ok(),
            boundary == ag1_storage::Boundary::BeforeAck
        );
        let receipt = f.store.command_result(&request.command).unwrap();
        registry
            .change_session(1, request.clone(), &factory)
            .await
            .unwrap();
        assert_eq!(f.store.snapshot("created").unwrap().entries.len(), 2);
        if let Some(receipt) = receipt {
            assert_eq!(
                f.store.command_result(&request.command).unwrap().unwrap(),
                receipt
            );
        }
        assert_eq!(registry.selected().unwrap().epoch, 2);
        assert!(!old.accepts_commands().unwrap());
        assert_eq!(provider.calls.load(Ordering::Acquire), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_bootstrap_caller_preserves_the_created_session_and_leaves_previous_selection_usable()
 {
    let gate = Arc::new(AckGate::default());
    let f = Fixture::with_fault(Some(gate.clone()));
    let (first, provider) = parent(&f);
    let old = first.runtime().clone();
    let registry =
        Arc::new(RuntimeRegistry::durable(first, None, "select-initial".into()).unwrap());
    registry.reconcile_selection().await.unwrap();
    let factory = Arc::new(factory(&f, provider.clone()));
    let request = request(
        SessionAction::Fork {
            entry: "entry-2".into(),
        },
        "created",
    );
    let owner = registry.clone();
    let next_factory = factory.clone();
    let next_request = request.clone();
    gate.armed.store(true, Ordering::Release);
    let task = tokio::spawn(async move {
        owner
            .change_session(1, next_request, next_factory.as_ref())
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), gate.entered.notified())
        .await
        .unwrap();
    task.abort();
    assert!(matches!(task.await,Err(error) if error.is_cancelled()));
    assert!(old.accepts_commands().unwrap());
    assert_eq!(registry.selected().unwrap().epoch, 1);
    gate.release();
    f.pool.submit(|| Ok(())).await.unwrap();
    let receipt = f.store.command_result(&request.command).unwrap().unwrap();
    let outcome = registry
        .change_session(1, request.clone(), factory.as_ref())
        .await
        .unwrap();
    assert_eq!(outcome.selected_text, Some("Secondinstruction".into()));
    assert_eq!(
        f.store.command_result(&request.command).unwrap().unwrap(),
        receipt
    );
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}
