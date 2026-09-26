use super::*;
use ag1_core::runtime::subagent::{
    Profile,
    checkpoint::{Manager, Owner, State},
};
use ag1_platform::session::subagent::{Request, Services};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_all_failed_exploration_pauses_without_losing_worker_outcomes() {
    struct FailedWorkers;
    impl ProviderResolver for FailedWorkers {
        fn resolve<'a>(&'a self,_:ProviderInput<'a>,_:&'a AtomicBool)->PortFuture<'a,ResolvedRequest> {
            Box::pin(async {Err(DomainError::Unavailable("controlled same worker failure".into()))})
        }
    }
    let responses = (0..3).map(|i|calls(vec![(&format!("explore-{i}"),"explore",json!({"questions":["Locate parser","Locate tests"]}))])).collect();
    let (endpoint,server) = server(responses);
    let mut f = fixture(&endpoint,"disabled");
    enable_task(&mut f,Arc::new(FailedWorkers)).await;
    let tool = Registry::builtins().unwrap().public_tools().into_iter().find(|t| t["name"]=="explore").unwrap();
    f.host.select_tools("explore-retry-tools","explore-retry-tools",NOW,vec![tool],&ignore).await.unwrap();
    let result = f.host.run(input(),&Resolver::new(false),f.files.clone(),&ignore).await.unwrap();
    assert_eq!(result.generations,3);
    assert_eq!(result.assistant["nativePolicy"],"repetition_guard");
    let results = session_tool_results(&f);
    assert_eq!(results.len(),3);
    for result in results {
        assert_eq!(result["isError"],true);
        assert_eq!(result["details"]["failed"],2);
        assert_eq!(result["details"]["recoveryFailures"].as_array().unwrap().len(),2);
        for failure in result["details"]["recoveryFailures"].as_array().unwrap() {
            assert_eq!(failure["errorDigest"].as_str().unwrap().len(), 64);
            assert!(failure.get("error").is_none());
            assert!(failure.get("summary").is_none());
        }
        assert!(result["details"]["resultArtifact"]["id"].is_string());
    }
    assert!(f.host.settled().unwrap());
    assert_eq!(server.join().unwrap().len(),3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_failed_task_with_changed_labels_pauses_and_retains_children() {
    struct FailedChild;
    impl ProviderResolver for FailedChild {
        fn resolve<'a>(&'a self, _: ProviderInput<'a>, _: &'a AtomicBool) -> PortFuture<'a,ResolvedRequest> {
            Box::pin(async { Err(DomainError::Unavailable("controlled child fixture failure".into())) })
        }
    }
    let responses = (0..3).map(|i| calls(vec![(&format!("delegate-{i}"),"task",json!({"description":format!("New label {i}"),"type":"explore","prompt":"Inspect the same source","timeout":5}))])).collect();
    let (endpoint, server) = server(responses);
    let mut f = fixture(&endpoint,"disabled");
    enable_task(&mut f,Arc::new(FailedChild)).await;
    let result = f.host.run(input(),&Resolver::new(false),f.files.clone(),&ignore).await.unwrap();
    assert_eq!(result.generations,3);
    assert_eq!(result.assistant["nativePolicy"],"repetition_guard");
    assert!(result.assistant.to_string().contains("retained child error/checkpoint"));
    let results = session_tool_results(&f);
    assert_eq!(results.len(),3);
    assert!(results.iter().all(|result| result["isError"]==true));
    assert!(f.host.settled().unwrap());
    assert_eq!(server.join().unwrap().len(),3);
}

fn request(profile: Profile, operation: &str) -> Request {
    Request {
        operation: operation.into(),
        description: "Inspect the owned fixture".into(),
        profile,
        prompt: "Perform the delegated fixture task".into(),
        model: None,
        max_turns: None,
        timeout: Some(5.),
        token_budget: None,
        resume_from: None,
    }
}
fn services(f: &Fixture) -> Services {
    Services {
        tools: f.files.clone(),
        provider: Arc::new(Resolver::new(false)),
        models: vec![f.model.clone()],
    }
}
fn owner(f: &Fixture) -> Owner {
    Owner {
        workspace: f.workspace.id().as_str().into(),
        session: "session".into(),
    }
}
async fn enable_task(f: &mut Fixture, provider: Arc<dyn ProviderResolver>) {
    use ag1_platform::session::subagent_tool::DelegationService;
    let template = Arc::new((*f.files).clone());
    let service = DelegationService::new(
        &f.host,
        template,
        provider,
        vec![f.model.clone()],
        tokio::runtime::Handle::current(),
    )
    .unwrap();
    let definition = ag1_core::runtime::subagent::tool::definition().unwrap();
    let runner = Arc::get_mut(&mut f.files).unwrap();
    runner.adapters.delegation = Some(Arc::new(service));
    f.host
        .select_tools("task-tools", "task-tools", NOW, vec![definition], &ignore)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explore_runs_native_readonly_children_and_imports_board_without_original_writes() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "explore-pair",
            "explore",
            json!({"questions":["First question","Second question"]}),
        )]),
        stop(),
        stop(),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    std::fs::write(f.root.join("board-source.txt"), "changed source").unwrap();
    let original=json!({"version":1,"updatedAt":1,"findings":[{"question":"Prior question","summary":"Preserved finding","state":"completed","evidence":[],"at":1,"sourceReferences":[format!("file:board-source.txt@sha256:{}",ag1_storage::digest(b"old source"))]}]}).to_string();
    std::fs::create_dir_all(f.root.join(".ag1")).unwrap();
    std::fs::write(f.root.join(".ag1/explore-board.json"), &original).unwrap();
    let child_resolver = Arc::new(Resolver::new(false));
    enable_task(&mut f, child_resolver.clone()).await;
    let definition = Registry::builtins()
        .unwrap()
        .public_tools()
        .into_iter()
        .find(|v| v["name"] == "explore")
        .unwrap();
    f.host
        .select_tools(
            "explore-tools",
            "explore-tools",
            NOW,
            vec![definition],
            &ignore,
        )
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(12),
        f.host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.generations, 2);
    assert!(f.host.settled().unwrap());
    assert_eq!(child_resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        std::fs::read_to_string(f.root.join(".ag1/explore-board.json")).unwrap(),
        original
    );
    let saved = f
        .store
        .load_metadata("explore-board", f.workspace.id().as_str())
        .unwrap()
        .unwrap();
    let board: Value = serde_json::from_slice(&saved.bytes).unwrap();
    assert_eq!(board["findings"].as_array().unwrap().len(), 3);
    assert_eq!(board["findings"][0]["summary"], "Preserved finding");
    assert_eq!(board["findings"][1]["question"], "First question");
    assert_eq!(board["findings"][2]["question"], "Second question");
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["customType"] == "ag1.explore_tasks")
            .count(),
        1
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    for request in &requests[1..3] {
        assert!(request.to_string().contains("Preserved finding"));
        assert!(request.to_string().contains("Historical workspace-board summaries"));
        assert!(request.to_string().contains("stale"));
        assert!(request.to_string().contains("ageMs"));
        assert!(request.to_string().contains("not independently verified"));
        assert!(!request["tools"].as_array().unwrap().iter().any(|t| {
            ["write", "edit", "bash", "task", "explore"]
                .contains(&t["function"]["name"].as_str().unwrap())
        }));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explore_cancellation_settles_all_admitted_children_before_parent_completion() {
    struct PausedExploreResolver {
        calls: AtomicUsize,
    }
    impl ProviderResolver for PausedExploreResolver {
        fn resolve<'a>(
            &'a self,
            _: ProviderInput<'a>,
            _: &'a AtomicBool,
        ) -> PortFuture<'a, ResolvedRequest> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                std::future::pending().await
            })
        }
    }
    let (endpoint, server) = server(vec![calls(vec![(
        "explore-cancel",
        "explore",
        json!({"questions":["One","Two"]}),
    )])]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let resolver = Arc::new(PausedExploreResolver {
        calls: AtomicUsize::new(0),
    });
    enable_task(&mut f, resolver.clone()).await;
    let definition = Registry::builtins()
        .unwrap()
        .public_tools()
        .into_iter()
        .find(|v| v["name"] == "explore")
        .unwrap();
    f.host
        .select_tools(
            "explore-tools",
            "explore-tools",
            NOW,
            vec![definition],
            &ignore,
        )
        .await
        .unwrap();
    let host = f.host.clone();
    let files = f.files.clone();
    let run = tokio::spawn(async move {
        host.run(input(), &Resolver::new(false), files, &ignore)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while resolver.calls.load(Ordering::SeqCst) < 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    f.host.cancel().unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(outcome, Err(DomainError::Cancelled)),
        "error: {:?}",
        outcome.as_ref().err()
    );
    assert!(f.host.settled().unwrap());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1, "Queued local workers must not start their child deadlines");
    assert!(!f.root.join(".ag1/explore-board.json").exists());
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explore_reports_all_failed_mixed_and_completed_outcomes() {
    struct SelectiveResolver { calls: AtomicUsize, successes: usize, inner: Resolver }
    impl ProviderResolver for SelectiveResolver {
        fn resolve<'a>(&'a self, input: ProviderInput<'a>, cancel: &'a AtomicBool) -> PortFuture<'a, ResolvedRequest> {
            if self.calls.fetch_add(1,Ordering::SeqCst) < self.successes { self.inner.resolve(input,cancel) }
            else { Box::pin(async {Err(DomainError::Unavailable("Controlled worker failure".into()))}) }
        }
    }
    for successes in 0..=2 {
        let mut responses=vec![calls(vec![("explore-status","explore",json!({"questions":["One","Two"]}))])];
        responses.extend((0..=successes).map(|_|stop()));
        let (endpoint,server)=server(responses);
        let mut f=fixture_configured(&endpoint,"disabled",vec![],None,None,1);
        enable_task(&mut f,Arc::new(SelectiveResolver {calls:AtomicUsize::new(0),successes,inner:Resolver::new(false)})).await;
        let tool=Registry::builtins().unwrap().public_tools().into_iter().find(|t|t["name"]=="explore").unwrap();
        f.host.select_tools("explore-tools","explore-tools",NOW,vec![tool],&ignore).await.unwrap();
        f.host.run(input(),&Resolver::new(false),f.files.clone(),&ignore).await.unwrap();
        let records=f.host.client_records().unwrap();
        let result=&records.iter().find(|r|r["message"]["toolName"]=="explore").unwrap()["message"];
        assert_eq!(result["isError"],successes==0,"{result}");
        assert_eq!(result["details"]["completed"],successes,"{result}");
        assert_eq!(result["details"]["status"],if successes==0 {"failed"} else if successes==1 {"partial"} else {"completed"});
        assert!(f.host.settled().unwrap());
        assert_eq!(server.join().unwrap().len(),successes+2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn corrupt_explore_board_is_retained_and_blocks_child_admission() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "explore-corrupt",
            "explore",
            json!({"questions":["One"]}),
        )]),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    std::fs::create_dir_all(f.root.join(".ag1")).unwrap();
    std::fs::write(f.root.join(".ag1/explore-board.json"), "{corrupt").unwrap();
    let resolver = Arc::new(Resolver::new(false));
    enable_task(&mut f, resolver.clone()).await;
    let definition = Registry::builtins()
        .unwrap()
        .public_tools()
        .into_iter()
        .find(|v| v["name"] == "explore")
        .unwrap();
    f.host
        .select_tools(
            "explore-tools",
            "explore-tools",
            NOW,
            vec![definition],
            &ignore,
        )
        .await
        .unwrap();
    f.host
        .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        std::fs::read_to_string(f.root.join(".ag1/explore-board.json")).unwrap(),
        "{corrupt"
    );
    assert!(
        f.store
            .load_metadata("explore-board", f.workspace.id().as_str())
            .unwrap()
            .is_none()
    );
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_task_executes_child_on_a_single_parent_worker_and_acknowledges_usage() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "delegate",
            "task",
            json!({"description":"Write the owned fixture","type":"implement","prompt":"Write child.txt with delegated value and report the result","maxTurns":2.9}),
        )]),
        calls(vec![(
            "write",
            "write",
            json!({"path":"child.txt","content":"delegated value"}),
        )]),
        stop(),
        stop(),
    ]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let child_resolver = Arc::new(Resolver::new(false));
    enable_task(&mut f, child_resolver.clone()).await;
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        f.host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.generations, 2);
    assert!(f.host.settled().unwrap());
    assert_eq!(
        std::fs::read_to_string(f.root.join("child.txt")).unwrap(),
        "delegated value"
    );
    assert_eq!(child_resolver.calls.load(Ordering::SeqCst), 2);
    let parent = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&parent).unwrap();
    let rollups = branch
        .iter()
        .filter(|e| e["customType"] == "ag1.subagent_task")
        .collect::<Vec<_>>();
    assert_eq!(rollups.len(), 1);
    assert_eq!(rollups[0]["data"]["state"], "completed");
    assert_eq!(rollups[0]["data"]["usage"]["totalTokens"], 102);
    let task = branch
        .iter()
        .find(|e| e["message"]["toolName"] == "task")
        .unwrap();
    assert!(
        task["message"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Task completed:")
    );
    assert_eq!(
        task["message"]["details"]["resultSet"]["filesWritten"],
        json!(["child.txt"])
    );
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["message"]["toolName"] == "write")
            .count(),
        0
    );
    assert!(
        branch
            .iter()
            .position(|e| e["customType"] == "ag1.subagent_task")
            .unwrap()
            < branch
                .iter()
                .position(|e| e["message"]["toolName"] == "task")
                .unwrap()
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        !requests[1]
            .to_string()
            .contains("Use the file tools to do the owned fixture task")
    );
    assert!(
        !requests[3]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["tool_call_id"] == "write")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_task_cancellation_drains_child_and_publishes_one_final_task_outcome() {
    let (endpoint, server) = server(vec![calls(vec![(
        "delegate",
        "task",
        json!({"description":"Inspect the owned fixture","type":"explore","prompt":"Inspect"}),
    )])]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let resolver = Arc::new(Resolver::new(true));
    enable_task(&mut f, resolver.clone()).await;
    let host = f.host.clone();
    let files = f.files.clone();
    let progress_seen = Arc::new(Notify::new());
    let updates = Arc::new(Mutex::new(Vec::new()));
    let seen = progress_seen.clone();
    let captured = updates.clone();
    let run = tokio::spawn(async move {
        host.run(
            input(),
            &Resolver::new(false),
            files,
            &move |event: &SessionEvent| {
                if let SessionEvent::Recovery(value) = event
                    && value["type"] == "tool_execution_update"
                {
                    captured.lock().unwrap().push(value.clone());
                    seen.notify_one();
                }
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), progress_seen.notified())
        .await
        .unwrap();
    let pending = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert!(
        !pending
            .iter()
            .any(|e| e["message"]["toolName"] == "task" || e["customType"] == "ag1.subagent_task")
    );
    let update = updates.lock().unwrap()[0].clone();
    assert_eq!(update["toolCallId"], "delegate");
    assert_eq!(update["partialResult"]["details"]["state"], "running");
    assert_eq!(
        update["partialResult"]["details"]["progress"]["model"],
        "owned-fixture/initial"
    );
    assert_eq!(
        update["partialResult"]["details"]["progress"]["toolCalls"],
        0
    );
    assert!(
        update["partialResult"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("0 tool calls · ~0 tok")
    );
    f.host.cancel().unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(3), run)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    assert!(f.host.settled().unwrap());
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["message"]["toolName"] == "task")
            .count(),
        1
    );
    let rollup = branch
        .iter()
        .find(|e| e["customType"] == "ag1.subagent_task")
        .unwrap();
    assert_eq!(rollup["data"]["state"], "failed");
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_turn_waits_for_delegated_work_without_appending_or_resolving() {
    let (endpoint, server) = server(vec![stop()]);
    let f = fixture(&endpoint, "disabled");
    let resolver = Arc::new(Resolver::new(true));
    let child = f
        .host
        .start_subagent(
            request(Profile::Explore, "pending-child"),
            Services {
                tools: f.files.clone(),
                provider: resolver.clone(),
                models: vec![f.model.clone()],
            },
        )
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
        .await
        .unwrap();
    let before = f.host.journal_revision().unwrap();
    let parent_resolver = Resolver::new(false);
    assert!(
        matches!(f.host.run(input(), &parent_resolver, f.files.clone(), &ignore).await,
        Err(DomainError::Unavailable(message)) if message.contains("delegated work"))
    );
    assert_eq!(f.host.journal_revision().unwrap(), before);
    assert_eq!(parent_resolver.calls.load(Ordering::Acquire), 0);
    child.cancel();
    assert!(child.finish().await.unwrap().settled);
    f.host
        .run(input(), &parent_resolver, f.files.clone(), &ignore)
        .await
        .unwrap();
    assert_eq!(server.join().unwrap().len(), 1);
}

struct TaskRollupFault {
    point: ag1_storage::Boundary,
    fired: AtomicBool,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cancellation_leaves_live_delegation_running_and_live_cancellation_drains_it() {
    use ag1_platform::session::{
        commands::{RunSettings, SessionCommands},
        registry::{PreparedRuntime, Selection},
    };
    use ag1_protocol::wire as w;
    let (endpoint, server) = server(vec![calls(vec![(
        "delegate",
        "task",
        json!({"description":"Inspect","type":"explore","prompt":"Inspect the fixture"}),
    )])]);
    let mut f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let resolver = Arc::new(Resolver::new(true));
    enable_task(&mut f, resolver.clone()).await;
    let services = Arc::new(PreparedRuntime::from_shared(
        f.host.clone(),
        Arc::new(Resolver::new(false)),
        f.files.clone(),
    ));
    let commands = SessionCommands::new(
        Selection { epoch: 1, services },
        "task-instance".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings {
            max_generations: Some(8),
            ..Default::default()
        },
    )
    .unwrap();
    let envelope = |id: &str, command: w::Command| w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: commands.scope().clone(),
        expected_revision: w::Sequence::new(f.host.journal_revision().unwrap()),
        command,
        extension: None,
    };
    let mut admitted = vec![];
    for id in ["live", "queued"] {
        let reply = commands
            .dispatch(envelope(
                id,
                w::Command::Prompt {
                    input: w::PromptInput::Inline {
                        text: "Inspect".into(),
                    },
                },
            ))
            .await;
        let w::CommandResult::Accepted { operation } = reply.result else {
            panic!("{reply:?}")
        };
        admitted.push(operation);
    }
    let owner = commands.clone();
    let run = tokio::spawn(async move { owner.run_next(&ignore).await });
    tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
        .await
        .unwrap();
    let reply = commands
        .dispatch(envelope(
            "cancel-queued",
            w::Command::Cancel {
                operation: admitted[1].clone(),
            },
        ))
        .await;
    assert!(matches!(reply.result, w::CommandResult::Accepted { .. }));
    assert!(!run.is_finished());
    assert!(!f.host.settled().unwrap());
    let reply = commands
        .dispatch(envelope(
            "cancel-live",
            w::Command::Cancel {
                operation: admitted[0].clone(),
            },
        ))
        .await;
    assert!(matches!(reply.result, w::CommandResult::Accepted { .. }));
    let completed = tokio::time::timeout(Duration::from_secs(3), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(completed.operation, admitted[0]);
    assert_eq!(completed.status, w::OperationStatus::Cancelled);
    assert!(f.host.settled().unwrap());
    let queued = commands.run_next(&ignore).await.unwrap().unwrap();
    assert_eq!(queued.operation, admitted[1]);
    assert_eq!(queued.status, w::OperationStatus::Cancelled);
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["message"]["role"] == "user")
            .count(),
        1
    );
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["message"]["toolName"] == "task")
            .count(),
        1
    );
    assert_eq!(
        branch
            .iter()
            .find(|e| e["customType"] == "ag1.subagent_task")
            .unwrap()["data"]["state"],
        "failed"
    );
    assert_eq!(server.join().unwrap().len(), 1);
}
impl ag1_storage::FaultInjector for TaskRollupFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if point == self.point
            && command.ends_with("-state")
            && !self.fired.swap(true, Ordering::AcqRel)
        {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_rollup_failure_retains_child_result_and_blocks_later_parent_effects() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (endpoint, server) = server(vec![
            calls(vec![
                (
                    "delegate",
                    "task",
                    json!({"description":"Write the child fixture","type":"implement","prompt":"Write child.txt"}),
                ),
                (
                    "later",
                    "write",
                    json!({"path":"parent.txt","content":"must not run"}),
                ),
            ]),
            calls(vec![(
                "child-write",
                "write",
                json!({"path":"child.txt","content":"retained"}),
            )]),
            stop(),
        ]);
        let fault = Arc::new(TaskRollupFault {
            point,
            fired: AtomicBool::new(false),
        });
        let mut f = fixture_seeded_with_fault(&endpoint, "disabled", vec![], Some(fault.clone()));
        enable_task(&mut f, Arc::new(Resolver::new(false))).await;
        let tools = f
            .files
            .registry
            .public_tools()
            .into_iter()
            .filter(|tool| ["task", "write"].contains(&tool["name"].as_str().unwrap()))
            .collect();
        f.host
            .select_tools("parent-tools", "parent-tools", NOW, tools, &ignore)
            .await
            .unwrap();
        assert!(
            f.host
                .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
                .await
                .is_err()
        );
        assert!(fault.fired.load(Ordering::Acquire));
        assert!(!f.host.persistence_healthy().unwrap());
        assert_eq!(
            std::fs::read_to_string(f.root.join("child.txt")).unwrap(),
            "retained"
        );
        assert!(!f.root.join("parent.txt").exists());
        let artifacts = f
            .store
            .artifact_catalog("session", None, 100, Some("subagent-result"))
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        let page = f
            .store
            .page(
                "session",
                "session",
                artifacts[0].record["id"].as_str().unwrap(),
                0,
                32768,
            )
            .unwrap();
        let result: Value = serde_json::from_slice(&page.bytes).unwrap();
        assert_eq!(result["state"], "completed");
        assert_eq!(result["resultSet"]["filesWritten"], json!(["child.txt"]));
        let checkpoint = Manager::load(
            &*f.store,
            owner(&f),
            result["checkpointId"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            checkpoint.acknowledged().final_state,
            Some(State::Completed)
        );
        let revision = f.host.journal_revision().unwrap();
        let resolver = Resolver::new(false);
        assert!(
            f.host
                .run(input(), &resolver, f.files.clone(), &ignore)
                .await
                .is_err()
        );
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
        assert_eq!(f.host.journal_revision().unwrap(), revision);
        assert_eq!(server.join().unwrap().len(), 3);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_profiles_run_isolated_transcripts_with_real_capability_restrictions_and_checkpoints() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/subagent-loop-oracle.json"
    ))
    .unwrap();
    for profile in [
        Profile::Explore,
        Profile::Research,
        Profile::Implement,
        Profile::Verify,
        Profile::Plan,
    ] {
        let (endpoint, server) = server(vec![
            calls(vec![
                (
                    "mutate",
                    "write",
                    json!({"path":"child.txt","content":"child result"}),
                ),
                ("inspect", "read", json!({"path":"existing.txt"})),
                (
                    "recurse",
                    "task",
                    json!({"description":"nested","type":"explore","prompt":"do not recurse"}),
                ),
            ]),
            stop(),
        ]);
        let f = fixture_seeded(
            &endpoint,
            "disabled",
            vec![json!({"role":"user","content":"PARENT PRIVATE TRANSCRIPT","timestamp":1000})],
        );
        std::fs::write(f.root.join("existing.txt"), "owned evidence").unwrap();
        let before = f.host.journal_revision().unwrap();
        let result = f
            .host
            .start_subagent(request(profile, "profile-run"), services(&f))
            .unwrap()
            .finish()
            .await
            .unwrap();
        assert_eq!(result.state, State::Completed, "{result:?}");
        assert!(result.settled, "{result:?}");
        assert_eq!(result.turns, 2);
        assert_eq!(result.summary, "Done");
        let source = oracle["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["type"] == json!(profile))
            .unwrap();
        assert_eq!(json!(result.evidence), source["evidence"]);
        assert_eq!(json!(result.turns), source["turns"]);
        assert_eq!(json!(result.state), source["state"]);
        assert_eq!(
            f.root.join("child.txt").exists(),
            profile == Profile::Implement
        );
        assert_eq!(f.host.journal_revision().unwrap(), before);
        let checkpoint = Manager::load(&*f.store, owner(&f), &result.checkpoint_id).unwrap();
        assert_eq!(
            checkpoint.acknowledged().final_state,
            Some(State::Completed)
        );
        assert_eq!(checkpoint.acknowledged().tool_calls, 3);
        assert_eq!(
            checkpoint.acknowledged().evidence.len(),
            if profile == Profile::Implement { 2 } else { 1 }
        );
        let child = f
            .store
            .journal_snapshot(&SessionId::new(&result.child_session).unwrap())
            .unwrap();
        let branch = ag1_core::task::branch_values(&child).unwrap();
        assert_eq!(
            branch
                .iter()
                .filter(|e| e["message"]["role"] == "toolResult")
                .count(),
            3
        );
        assert!(!branch.iter().any(|e| e["customType"] == "task_state"));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(
            !json!(requests)
                .to_string()
                .contains("PARENT PRIVATE TRANSCRIPT")
        );
        for sent in requests {
            let names = sent["tools"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t["function"]["name"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert!(!names.contains(&"task"));
            assert_eq!(names.contains(&"write"), profile == Profile::Implement);
        }
        assert_eq!(f.scheduler.status(&endpoint).unwrap().active, 0);
        // Stable operation identity is not permission to replay a settled child.
        let duplicate = f
            .host
            .start_subagent(request(profile, "profile-run"), services(&f))
            .unwrap()
            .finish()
            .await;
        assert!(
            matches!(duplicate, Err(DomainError::Conflict)),
            "{duplicate:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generation_budget_retains_partial_evidence_and_resume_revalidates_in_fresh_child() {
    let (endpoint, server) = server(vec![
        calls(vec![(
            "write",
            "write",
            json!({"path":"partial.txt","content":"acknowledged"}),
        )]),
        stop(),
    ]);
    let f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
    let mut first = request(Profile::Implement, "budget-run");
    first.max_turns = Some(1);
    let result = f
        .host
        .start_subagent(first, services(&f))
        .unwrap()
        .finish()
        .await
        .unwrap();
    assert_eq!(result.state, State::Budget, "{result:?}");
    assert!(result.partial && result.settled);
    assert_eq!(result.evidence.len(), 1);
    assert_eq!(
        std::fs::read_to_string(f.root.join("partial.txt")).unwrap(),
        "acknowledged"
    );
    std::fs::write(f.root.join("partial.txt"), "changed externally").unwrap();
    let mut resumed = request(Profile::Explore, "resume-run");
    resumed.resume_from = Some(result.checkpoint_id.clone());
    let final_result = f
        .host
        .start_subagent(resumed, services(&f))
        .unwrap()
        .finish()
        .await
        .unwrap();
    assert_eq!(final_result.state, State::Completed);
    assert_ne!(final_result.child_session, result.child_session);
    assert_eq!(
        Manager::load(&*f.store, owner(&f), &result.checkpoint_id)
            .unwrap()
            .acknowledged()
            .final_state,
        Some(State::Budget)
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    let resume = requests[1].to_string();
    assert!(resume.contains("Prior findings are historical") && resume.contains("partial.txt"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_child_calls_stop_before_third_effect_and_settle_each_call_once() {
    let (endpoint, server) = server(vec![
        calls(vec![("one", "read", json!({"path":"existing"}))]),
        calls(vec![("two", "read", json!({"path":"existing"}))]),
        calls(vec![("three", "read", json!({"path":"existing"}))]),
    ]);
    let f = fixture(&endpoint, "disabled");
    std::fs::write(f.root.join("existing"), "evidence").unwrap();
    let result = f
        .host
        .start_subagent(request(Profile::Explore, "repeat-run"), services(&f))
        .unwrap()
        .finish()
        .await
        .unwrap();
    assert_eq!(result.state, State::Budget, "{result:?}");
    assert!(result.settled && result.partial);
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new(&result.child_session).unwrap())
            .unwrap(),
    )
    .unwrap();
    let results = branch
        .iter()
        .filter(|e| e["message"]["role"] == "toolResult")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 3);
    assert_eq!(results[2]["message"]["isError"], true);
    assert_eq!(result.evidence.len(), 2);
    assert_eq!(server.join().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn token_budget_blocks_tools_and_classifies_an_over_budget_final_answer() {
    for terminal in [false, true] {
        let mut response = if terminal {
            stop()
        } else {
            calls(vec![(
                "write",
                "write",
                json!({"path":"blocked","content":"must not run"}),
            )])
        };
        response["usage"] = json!({"prompt_tokens":100,"completion_tokens":2,"total_tokens":102});
        let (endpoint, server) = server(vec![response]);
        let f = fixture(&endpoint, "disabled");
        let mut req = request(Profile::Implement, "tokens");
        req.token_budget = Some(1);
        let result = f
            .host
            .start_subagent(req, services(&f))
            .unwrap()
            .finish()
            .await
            .unwrap();
        assert_eq!(result.state, State::Budget, "{result:?}");
        assert_eq!(result.usage["totalTokens"], 102);
        assert!(result.settled);
        assert!(!f.root.join("blocked").exists());
        assert_eq!(server.join().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_cancel_deadline_and_dropped_caller_drain_before_reporting_settlement() {
    for mode in ["parent", "handle", "drop", "timeout"] {
        let (endpoint, server) = server(vec![]);
        let f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
        let resolver = Arc::new(Resolver::new(true));
        let mut req = request(Profile::Explore, mode);
        if mode == "timeout" {
            req.timeout = Some(1.);
        }
        let task = f
            .host
            .start_subagent(
                req,
                Services {
                    tools: f.files.clone(),
                    provider: resolver.clone(),
                    models: vec![f.model.clone()],
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
            .await
            .unwrap();
        assert!(!f.host.settled().unwrap());
        let revision = f.host.journal_revision().unwrap();
        let admission_resolver = Resolver::new(false);
        assert!(
            matches!(f.host.run(input(), &admission_resolver, f.files.clone(), &ignore).await,
            Err(DomainError::Unavailable(message)) if message.contains("delegated work"))
        );
        assert_eq!(f.host.journal_revision().unwrap(), revision);
        assert_eq!(admission_resolver.calls.load(Ordering::Acquire), 0);
        match mode {
            "parent" => f.host.cancel().unwrap(),
            "handle" => task.cancel(),
            _ => (),
        }
        if mode == "drop" {
            drop(task);
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let record = f.host.recorded_subagent(mode).await.unwrap();
                if record["state"] == "completed" {
                    assert_eq!(record["result"]["state"], "failed");
                    assert_eq!(record["result"]["settled"], true);
                    // The receipt may become readable while its owned task is
                    // still releasing the parent lifecycle/retention guards.
                    if f.host.settled().unwrap() {
                        break;
                    }
                }
                assert!(Instant::now() < deadline);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        } else {
            let result = tokio::time::timeout(Duration::from_secs(3), task.finish())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                result.state,
                if mode == "timeout" {
                    State::Timeout
                } else {
                    State::Failed
                }
            );
            assert!(result.settled, "{result:?}");
        }
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        assert!(f.host.settled().unwrap());
        assert_eq!(f.scheduler.status(&endpoint).unwrap().active, 0);
        assert_eq!(server.join().unwrap().len(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_shell_uses_the_readonly_capability_and_retains_real_exit_status() {
    use ag1_platform::{
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellService},
    };
    let (endpoint, server) = server(vec![
        calls(vec![(
            "shell",
            "bash",
            json!({"command":"env touch blocked"}),
        )]),
        stop(),
    ]);
    let mut f = fixture(&endpoint, "disabled");
    let spools = f._directory.path().join("spools");
    std::fs::create_dir(&spools).unwrap();
    let shell = ShellService::new(
        f.workspace.clone(),
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap()),
        &spools,
        ShellConfig {
            executable: "/bin/bash".into(),
            env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
            command_prefix: Some("printf 'configured prefix\\n'".into()),
            command_template: None,
            sandbox: Sandbox::Off,
            bubblewrap: Some("/usr/bin/bwrap".into()),
            output_bytes: 64 * 1024,
            address_space: None,
        },
    )
    .unwrap();
    Arc::get_mut(&mut f.files).unwrap().adapters.shell = Some(Arc::new(shell));
    let result = f
        .host
        .start_subagent(request(Profile::Explore, "readonly"), services(&f))
        .unwrap()
        .finish()
        .await
        .unwrap();
    assert!(!f.root.join("blocked").exists());
    assert_eq!(result.state, State::Completed);
    assert!(result.settled);
    assert!(!result.unavailable_tools.iter().any(|tool| tool == "bash"));
    assert!(matches!(
        result.evidence.as_slice(),
        [ag1_core::runtime::subagent::checkpoint::Evidence::Bash {
            exit_code: Some(1),
            ..
        }]
    ));
    let requests = server.join().unwrap();
    assert!(requests[1].to_string().contains("configured prefix"));
    assert_eq!(std::fs::read_dir(spools).unwrap().count(), 0);
}

struct ChildDrainFault {
    point: ag1_storage::Boundary,
    marker: Mutex<Option<std::path::PathBuf>>,
    armed: AtomicBool,
}
impl ag1_storage::FaultInjector for ChildDrainFault {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point == self.point && self.armed.load(Ordering::Acquire) {
            let stopped = self
                .marker
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|path| std::fs::read_to_string(path).ok())
                .and_then(|text| text.parse::<u32>().ok())
                .is_some_and(|pid| !std::path::Path::new(&format!("/proc/{pid}")).exists());
            if stopped && self.armed.swap(false, Ordering::AcqRel) {
                return Err(ag1_storage::StorageError::Injected(point));
            }
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delegated_jobs_drain_with_durable_output_while_parent_jobs_remain_owned() {
    use ag1_platform::session::ToolRunner;
    for fault_point in [
        None,
        Some(ag1_storage::Boundary::BeforeCommit),
        Some(ag1_storage::Boundary::BeforeAck),
    ] {
        let start = |owner: &str| {
            calls(vec![(
                "start",
                "job",
                json!({"action":"start",
            "command":format!("printf '{owner} evidence\\n'; printf '%s' \"$$\" > {owner}.pid; exec /usr/bin/sleep 30") }),
            )])
        };
        let (endpoint, server) = server(vec![start("parent"), stop(), start("child"), stop()]);
        let fault = Arc::new(ChildDrainFault {
            point: fault_point.unwrap_or(ag1_storage::Boundary::BeforeCommit),
            marker: Mutex::new(None),
            armed: AtomicBool::new(false),
        });
        let mut f = fixture_seeded_with_fault(&endpoint, "disabled", vec![], Some(fault.clone()));
        f.files = native_adapters(&f, &["job"], None).await;
        f.host
            .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
            .await
            .unwrap();
        let mut resolver = Resolver::new(true);
        resolver.pause_index = 1;
        let resolver = Arc::new(resolver);
        let child = f
            .host
            .start_subagent(
                request(Profile::Implement, "owned-jobs"),
                Services {
                    tools: f.files.clone(),
                    provider: resolver.clone(),
                    models: vec![f.model.clone()],
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), resolver.entered.notified())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while ["parent.pid", "child.pid"].iter().any(|file| {
                std::fs::read_to_string(f.root.join(file))
                    .ok()
                    .is_none_or(|s| s.parse::<u32>().is_err())
            }) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let process = |owner: &str| {
            let pid = std::fs::read_to_string(f.root.join(format!("{owner}.pid")))
                .unwrap()
                .parse::<u32>()
                .unwrap();
            std::path::PathBuf::from(format!("/proc/{pid}"))
        };
        let parent_process = process("parent");
        let child_process = process("child");
        assert!(parent_process.exists() && child_process.exists());
        if fault_point.is_some() {
            *fault.marker.lock().unwrap() = Some(f.root.join("child.pid"));
            fault.armed.store(true, Ordering::Release);
        }
        resolver.release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(5), child.finish())
            .await
            .unwrap();
        if fault_point.is_some() {
            assert!(
                !fault.armed.load(Ordering::Acquire),
                "fault reached child output custody after process drainage"
            );
            assert!(!result.as_ref().is_ok_and(|result| result.settled));
            assert!(!f.host.settled().unwrap());
            f.host.drain_subagent("owned-jobs").await.unwrap();
            assert!(
                !f.host.settled().unwrap(),
                "resource cleanup cannot settle uncertain journal effects"
            );
        } else {
            let result = result.unwrap();
            assert!(result.settled, "{result:?}");
            assert!(!result.unavailable_tools.iter().any(|tool| tool == "job"));
        }
        assert!(
            !child_process.exists(),
            "child process must drain even after journal failure"
        );
        assert!(
            parent_process.exists(),
            "child cleanup must not retire the parent job"
        );
        assert!(
            f.files
                .adapters
                .jobs
                .as_ref()
                .unwrap()
                .notices()
                .unwrap()
                .is_empty()
        );
        let record = f.host.recorded_subagent("owned-jobs").await.unwrap();
        let child_session = record["childSession"].as_str().unwrap();
        let artifacts = f
            .store
            .artifact_catalog(child_session, None, 100, Some("job-result"))
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        let artifact = artifacts[0].record["id"].as_str().unwrap();
        let page = f
            .store
            .page(child_session, child_session, artifact, 0, 32768)
            .unwrap();
        let output: Value = serde_json::from_slice(&page.bytes).unwrap();
        assert_eq!(output["details"]["termination"], "cancelled");
        assert!(
            output["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("child evidence")
        );
        assert!(
            f.store
                .page("session", "session", artifact, 0, 32768)
                .is_err(),
            "child output cannot be rehydrated through parent custody"
        );
        f.files
            .quiesce(
                &ag1_core::tools::registry::Scope {
                    workspace: f.workspace.id().as_str().into(),
                    session: "session".into(),
                    epoch: 1,
                    revision: f.host.journal_revision().unwrap(),
                },
                "parent-cleanup",
            )
            .unwrap();
        assert!(!parent_process.exists());
        assert!(
            std::fs::read_dir(f._directory.path().join("tool-spools"))
                .unwrap()
                .next()
                .is_none()
        );
        assert_eq!(server.join().unwrap().len(), 4);
    }
}

struct MutationFault {
    marker: Mutex<Option<std::path::PathBuf>>,
    fired: AtomicBool,
    point: ag1_storage::Boundary,
}
impl ag1_storage::FaultInjector for MutationFault {
    fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        if point == self.point
            && self
                .marker
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|p| p.exists())
            && !self.fired.swap(true, Ordering::AcqRel)
        {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unacknowledged_child_mutation_is_retained_and_never_replayed_automatically() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (endpoint, server) = server(vec![calls(vec![
            (
                "first",
                "write",
                json!({"path":"first","content":"retained"}),
            ),
            (
                "second",
                "write",
                json!({"path":"second","content":"must not run"}),
            ),
        ])]);
        let fault = Arc::new(MutationFault {
            marker: Mutex::new(None),
            fired: AtomicBool::new(false),
            point,
        });
        let f = fixture_seeded_with_fault(&endpoint, "disabled", vec![], Some(fault.clone()));
        *fault.marker.lock().unwrap() = Some(f.root.join("first"));
        let result = f
            .host
            .start_subagent(request(Profile::Implement, "uncertain"), services(&f))
            .unwrap()
            .finish()
            .await;
        assert!(result.is_err() || result.is_ok_and(|r| !r.settled));
        assert!(fault.fired.load(Ordering::Acquire));
        assert_eq!(
            std::fs::read_to_string(f.root.join("first")).unwrap(),
            "retained"
        );
        assert!(!f.root.join("second").exists());
        assert!(!f.host.settled().unwrap());
        let revision = f.host.journal_revision().unwrap();
        let resolver = Resolver::new(false);
        assert!(
            matches!(f.host.run(input(), &resolver, f.files.clone(), &ignore).await,
            Err(DomainError::Unavailable(message)) if message.contains("delegated work"))
        );
        assert_eq!(f.host.journal_revision().unwrap(), revision);
        assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
        assert!(f.host.reconcile_subagent("uncertain").await.is_err());
        assert!(
            f.host
                .start_subagent(request(Profile::Implement, "uncertain"), services(&f))
                .unwrap()
                .finish()
                .await
                .is_err()
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_children_wait_behind_the_existing_endpoint_permit() {
    let (endpoint, server) = server(vec![stop(), stop()]);
    let f = fixture(&endpoint, "disabled");
    let ticket = f
        .scheduler
        .submit(
            &endpoint,
            ag1_core::OperationId::new("existing-parent-generation").unwrap(),
        )
        .unwrap();
    let permit = f.scheduler.start(&ticket).unwrap().unwrap();
    let a = f
        .host
        .start_subagent(request(Profile::Explore, "queued-a"), services(&f))
        .unwrap();
    let b = f
        .host
        .start_subagent(request(Profile::Plan, "queued-b"), services(&f))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let status = f.scheduler.status(&endpoint).unwrap();
        assert_eq!(status.active, 1);
        if status.waiting == 2 {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!f.host.settled().unwrap());
    permit.terminal().unwrap();
    let (a, b) = tokio::join!(a.finish(), b.finish());
    assert!(a.unwrap().settled && b.unwrap().settled);
    assert!(f.host.settled().unwrap());
    assert_eq!(f.scheduler.status(&endpoint).unwrap().active, 0);
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_requests_use_lighter_hygiene_while_original_results_remain_durable() {
    let replies = (0..4)
        .map(|i| {
            let id = format!("read-{i}");
            calls(vec![(&id, "read", json!({"path":format!("file-{i}")}))])
        })
        .chain([stop()])
        .collect();
    let (endpoint, server) = server(replies);
    let f = fixture(&endpoint, "disabled");
    let original = "original retained evidence ".repeat(500);
    for i in 0..4 {
        std::fs::write(
            f.root.join(format!("file-{i}")),
            if i == 0 {
                original.as_str()
            } else {
                "short evidence"
            },
        )
        .unwrap();
    }
    let result = f
        .host
        .start_subagent(request(Profile::Explore, "hygiene"), services(&f))
        .unwrap()
        .finish()
        .await
        .unwrap();
    assert_eq!(result.state, State::Completed, "{result:?}");
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 5);
    let old = requests[4]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == "read-0")
        .unwrap();
    assert!(
        old["content"]
            .as_str()
            .unwrap()
            .contains("[truncated] read result"),
        "{old}"
    );
    assert!(!old["content"].as_str().unwrap().contains(&original));
    assert!(
        !requests[4]
            .to_string()
            .contains("ag1-artifact-catalog-boundary")
    );
    let child = f
        .store
        .journal_snapshot(&SessionId::new(&result.child_session).unwrap())
        .unwrap();
    let branch = ag1_core::task::branch_values(&child).unwrap();
    assert!(
        branch
            .iter()
            .any(|entry| entry["message"]["role"] == "toolResult"
                && ag1_core::context::hygiene::content(&entry["message"], "\n")
                    .contains(&original))
    );
}
