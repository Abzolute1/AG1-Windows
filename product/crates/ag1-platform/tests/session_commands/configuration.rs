use super::*;

fn control(c: &SessionCommands, id: &str, command: w::Command) -> w::CommandEnvelope {
    let mut envelope = prompt(c, id, c.snapshot().unwrap().revision.get(), "");
    envelope.command = command;
    envelope
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_configuration_and_task_changes_rebuild_a_paused_request_from_host_catalogs() {
    let (endpoint, server) = server(vec![stop()]);
    let fixture = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let mut model = fixture.host.configuration().unwrap().model;
    model["id"] = json!("changed-model");
    model["contextWindow"] = json!(18000);
    let resolver = Arc::new(Resolver::new(true));
    let f = attach_with_catalog(fixture, resolver.clone(), vec![model.clone()]);
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "run", 0, "Inspect changes"))
            .await
            .result,
    );
    let run = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    resolver.entered.notified().await;
    let cases = [
        (
            "model",
            w::Command::Model {
                action: "select".into(),
                args: json!({"provider":model["provider"],"model_id":model["id"]}),
            },
        ),
        (
            "thinking",
            w::Command::Model {
                action: "thinking".into(),
                args: json!({"level":"high"}),
            },
        ),
        (
            "tools",
            w::Command::Runtime {
                action: "tools".into(),
                args: json!({"names":["read"]}),
            },
        ),
        (
            "task",
            w::Command::Task {
                action: "set".into(),
                args: json!({"currentPhase":"wire-controlled-phase","nextAction":"Inspect only"}),
            },
        ),
    ];
    for (id, command) in cases {
        let envelope = control(&f.commands, id, command);
        let operation = accepted(f.commands.dispatch(envelope.clone()).await.result);
        let revision = f.host.journal_revision().unwrap();
        assert_eq!(
            accepted(f.commands.dispatch(envelope).await.result),
            operation
        );
        assert_eq!(f.host.journal_revision().unwrap(), revision);
        assert!(
            matches!(lookup(&f.commands, id).await, w::CommandResult::Settled { operation } if operation.status == w::OperationStatus::Completed)
        );
    }
    resolver.release.notify_one();
    assert_eq!(
        run.await.unwrap().unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "changed-model");
    assert!(requests[0].to_string().contains("wire-controlled-phase"));
    let config = f.host.configuration().unwrap();
    assert_eq!(config.thinking, "high");
    assert_eq!(config.tools.len(), 1);
    assert_eq!(config.tools[0]["name"], "read");
    assert!(resolver.calls.load(Ordering::SeqCst) >= 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn configuration_rejects_unregistered_models_schemas_and_invalid_arguments_before_intent() {
    let f = attach(
        fixture("http://127.0.0.1:1", "disabled"),
        Arc::new(Resolver::new(false)),
    );
    for (i, command) in [
        w::Command::Model {
            action: "select".into(),
            args: json!({"provider":"injected","model_id":"injected"}),
        },
        w::Command::Model {
            action: "thinking".into(),
            args: json!({"level":"unbounded"}),
        },
        w::Command::Model {
            action: "select".into(),
            args: json!({"provider":"x","model_id":"x","endpoint":"http://attacker"}),
        },
        w::Command::Runtime {
            action: "tools".into(),
            args: json!({"names":["unknown"]}),
        },
        w::Command::Runtime {
            action: "tools".into(),
            args: json!({"names":["read","read"]}),
        },
        w::Command::Task {
            action: "unknown".into(),
            args: json!({}),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let id = format!("invalid-{i}");
        assert!(matches!(
            f.commands
                .dispatch(control(&f.commands, &id, command))
                .await
                .result,
            w::CommandResult::Rejected { .. }
        ));
        assert_eq!(
            rejected(lookup(&f.commands, &id).await),
            w::ProtocolError::NotFound
        );
        assert_eq!(f.host.journal_revision().unwrap(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_configuration_interrupts_approval_through_the_existing_tool_journal_owner() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "write",
            "write",
            json!({"path":"output.txt","content":"forbidden"}),
        )]),
        stop(),
    ]);
    let mut fixture = fixture_configured(&endpoint, "interactive", vec![], None, None, 1);
    let approval = Arc::new(WaitingApproval {
        entered: Notify::new(),
        approved: AtomicBool::new(false),
    });
    Arc::get_mut(&mut fixture.files).unwrap().approvals = approval.clone();
    let f = attach(fixture, Arc::new(Resolver::new(false)));
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "run", 0, "Inspect"))
            .await
            .result,
    );
    let run = {
        let c = f.commands.clone();
        tokio::spawn(async move { c.run_next(&ignore).await })
    };
    approval.entered.notified().await;
    let envelope = control(
        &f.commands,
        "tools",
        w::Command::Runtime {
            action: "tools".into(),
            args: json!({"names":["read"]}),
        },
    );
    accepted(
        tokio::time::timeout(Duration::from_secs(2), f.commands.dispatch(envelope))
            .await
            .unwrap()
            .result,
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
    assert!(!f.root.join("output.txt").exists());
    assert_eq!(f.host.configuration().unwrap().tools.len(), 1);
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn discovered_effort_is_durable_and_reaches_the_template_without_downgrading() {
    let f = attach(
        fixture("http://127.0.0.1:1", "disabled"),
        Arc::new(Resolver::new(false)),
    );
    let mut original = f.host.configuration().unwrap();
    original.model["reasoning"] = json!(true);
    original.model["compat"] = json!({"thinkingFormat":"qwen-chat-template"});
    let command = control(
        &f.commands,
        "native-effort",
        w::Command::Model {
            action: "thinking".into(),
            args: json!({"level":"xhigh","native_profile":{"apply":"reasoning_effort","modes":["low","medium","high","xhigh"]}}),
        },
    );
    accepted(f.commands.dispatch(command).await.result);
    let scope = f.commands.snapshot().unwrap().scope;
    let snapshot = f
        .store
        .journal_snapshot(&SessionId::new(scope.session.as_str()).unwrap())
        .unwrap();
    let restored = SessionRuntime::resume(
        SessionId::new(scope.session.as_str()).unwrap(),
        1,
        &snapshot,
        original,
    )
    .unwrap();
    assert_eq!(restored.configuration().thinking, "xhigh");
    let model = &restored.configuration().model;
    assert_eq!(model["thinking"]["apply"], "reasoning_effort");
    let options = ag1_platform::provider::requests::session_options(model, "xhigh", None).unwrap();
    assert_eq!(options["reasoningEffort"], "xhigh");
    let payload = ag1_platform::provider::requests::build_payload(
        &ag1_platform::provider::requests::RequestContext {
            model,
            context: &json!({"systemPrompt":"Fixture","messages":[]}),
            options: &options,
            anthropic_oauth: false,
            google_project: None,
            request_id: "fixture",
        },
    )
    .unwrap();
    assert_eq!(payload["chat_template_kwargs"]["reasoning_effort"], "xhigh");
}
