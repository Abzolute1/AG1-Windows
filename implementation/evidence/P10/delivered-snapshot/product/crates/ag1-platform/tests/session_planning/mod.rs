use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planning_text_fallback_builds_workflow_state_and_generic_goals_do_not_inspect_files() {
    use ag1_core::runtime::planning::Status;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-markdown-oracle.json"
    ))
    .unwrap();
    for generic in [true, false] {
        let mut response = stop();
        response["choices"][0]["delta"]["content"] = if generic {
            json!("Sure, let us plan.")
        } else {
            oracle["rows"][0]["input"]["assistantText"].clone()
        };
        let responses = if generic {
            vec![
                calls(vec![("premature", "read", json!({"path":"private.txt"}))]),
                response,
            ]
        } else {
            vec![response]
        };
        let (endpoint, server) = server(responses);
        let f = fixture_configured(&endpoint, "disabled", vec![], None, None, 1);
        std::fs::write(
            f.root.join("private.txt"),
            "fixture inspection must wait for choices",
        )
        .unwrap();
        let mut input = input();
        input.message["content"] = if generic {
            json!("Plan a snake game")
        } else {
            oracle["rows"][0]["input"]["goal"].clone()
        };
        let result = ag1_platform::session::planning::run(
            f.host.clone(),
            input,
            Arc::new(Resolver::new(false)),
            f.files.clone(),
            Arc::new(ignore),
            Default::default(),
        )
        .await
        .unwrap();
        let plan = result.plan.unwrap();
        assert_eq!(
            plan.status,
            if generic {
                Status::Clarifying
            } else {
                Status::Draft
            }
        );
        if generic {
            assert_eq!(
                plan.open_questions,
                vec![
                    "Target language?",
                    "Target interface/platform?",
                    "Feature scope?"
                ]
            );
            let branch = ag1_core::task::branch_values(
                &f.store
                    .journal_snapshot(&SessionId::new("session").unwrap())
                    .unwrap(),
            )
            .unwrap();
            let rejected = branch
                .iter()
                .find(|e| e["message"]["toolCallId"] == "premature")
                .unwrap();
            assert_eq!(rejected["message"]["isError"], true);
            assert_eq!(
                rejected["message"]["details"]["rejectedBeforeExecution"],
                true
            );
            assert!(
                !serde_json::to_string(&branch)
                    .unwrap()
                    .contains("fixture inspection must wait for choices")
            );
        } else {
            assert_eq!(json!(plan.steps), oracle["rows"][0]["result"]["steps"]);
        }
        assert!(f.host.settled().unwrap());
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), if generic { 2 } else { 1 });
        assert!(!requests.iter().any(|r| {
            r.to_string()
                .contains("fixture inspection must wait for choices")
        }));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wire_planning_interview_revision_acceptance_and_execution_share_one_durable_queue() {
    use ag1_core::runtime::planning::Status;
    use ag1_platform::session::{
        commands::{RunSettings, SessionCommands},
        registry::{PreparedRuntime, RuntimeRegistry},
    };
    use ag1_protocol::wire as w;
    let mut review = stop();
    review["choices"][0]["delta"]["content"] =
        json!("Execution Status:\ncompleted\nExecution Summary:\nFirst step done.");
    let (endpoint, server) = server(vec![
        calls(vec![(
            "questions",
            "submit_plan",
            json!({"summary":"Need a target","openQuestions":[{"question":"Platform?","options":["Web","Native"]}]}),
        )]),
        stop(),
        calls(vec![(
            "draft",
            "submit_plan",
            json!({"summary":"Native endpoint","steps":["Implement first","Verify second"]}),
        )]),
        stop(),
        calls(vec![(
            "write",
            "write",
            json!({"path":"wire.txt","content":"native"}),
        )]),
        review,
    ]);
    let f = fixture(&endpoint, "disabled");
    let registry = RuntimeRegistry::new(
        PreparedRuntime::from_shared(
            f.host.clone(),
            Arc::new(Resolver::new(false)),
            f.files.clone(),
        ),
        1,
    )
    .unwrap();
    let commands = SessionCommands::new(
        registry.selected().unwrap(),
        "planner".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    let request = |id: &str, action: &str, args: Value| w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: commands.scope().clone(),
        expected_revision: commands.snapshot().unwrap().revision,
        extension: None,
        command: w::Command::Plan {
            action: action.into(),
            args,
        },
    };
    let draft = request(
        "draft",
        "draft",
        json!({"input":{"kind":"inline","text":"Plan the endpoint"}}),
    );
    assert!(matches!(
        commands.dispatch(draft.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    assert_eq!(
        commands.run_next(&ignore).await.unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    let question = f.host.plan_snapshot(1).unwrap().plans.remove(0);
    assert_eq!(question.status, Status::Clarifying);
    assert!(!f.root.join("wire.txt").exists());
    assert_plan_command(
        &commands,
        request(
            "answer",
            "answer",
            json!({"planId":question.plan_id,"revision":1,"questionIndex":0,"answer":"Native"}),
        ),
    )
    .await;
    let revise = request(
        "revise",
        "draft",
        json!({"input":{"kind":"inline","text":"Continue with saved answers"},"previous":question.reference()}),
    );
    assert!(matches!(
        commands.dispatch(revise).await.result,
        w::CommandResult::Accepted { .. }
    ));
    assert_eq!(
        commands.run_next(&ignore).await.unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    let plan = f.host.plan_snapshot(1).unwrap().plans.remove(0);
    assert_eq!(plan.revision, 2);
    assert_eq!(plan.goal, question.goal);
    assert_eq!(plan.clarification_answers, Some(vec!["Native".into()]));
    assert_eq!(
        f.host.plan(&question.reference()).unwrap().status,
        Status::Superseded
    );
    assert_plan_command(
        &commands,
        request(
            "accept",
            "accept",
            json!({"planId":plan.plan_id,"revision":2}),
        ),
    )
    .await;
    let execute = request(
        "execute",
        "execute",
        json!({"planId":plan.plan_id,"revision":2,"stepIndex":0}),
    );
    assert!(matches!(
        commands.dispatch(execute.clone()).await.result,
        w::CommandResult::Accepted { .. }
    ));
    assert_eq!(
        commands.run_next(&ignore).await.unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    assert_eq!(
        std::fs::read_to_string(f.root.join("wire.txt")).unwrap(),
        "native"
    );
    let finished = f.host.plan(&plan.reference()).unwrap();
    assert_eq!(finished.status, Status::Accepted);
    assert_eq!(finished.next_step(), Some(1));
    let revision = f.host.journal_revision().unwrap();
    for envelope in [execute, draft] {
        assert!(matches!(
            commands.dispatch(envelope).await.result,
            w::CommandResult::Accepted { .. }
        ));
    }
    assert!(commands.run_next(&ignore).await.unwrap().is_none());
    assert_eq!(f.host.journal_revision().unwrap(), revision);
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[2].to_string().contains("A1: Native"));
    assert!(!requests[4].to_string().contains("planning-mode-guidance"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_plan_executes_one_step_with_native_tools_and_retains_review_without_automatic_next_step()
 {
    use ag1_core::runtime::planning::{self, Plan, Selection, Status};
    use ag1_platform::session::planning as mode;
    let mut review = stop();
    review["choices"][0]["delta"]["content"] = json!(
        "Execution Status:\ncompleted\nExecution Summary:\nImplemented the first step.\nFiles Touched:\n- first.txt\nVerification:\n- inspected output\nRemaining:\n- second step"
    );
    let (endpoint, server) = server(vec![
        calls(vec![(
            "first",
            "write",
            json!({"path":"first.txt","content":"first step"}),
        )]),
        review,
    ]);
    let f = fixture(&endpoint, "disabled");
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-state-oracle.json"
    ))
    .unwrap();
    let draft: Plan = serde_json::from_value(oracle["draft"].clone()).unwrap();
    let mut selection = Selection {
        version: 1,
        planning: Some(draft.reference()),
        accepted: None,
        executing: None,
        updated_at: NOW.into(),
    };
    f.host
        .update_plan(
            "save-plan",
            NOW,
            Some(draft.clone()),
            selection.clone(),
            &ignore,
        )
        .await
        .unwrap();
    let resolver = Arc::new(Resolver::new(false));
    assert!(
        mode::execute(
            f.host.clone(),
            input(),
            resolver.clone(),
            f.files.clone(),
            Arc::new(ignore),
            mode::Execution {
                plan: draft.reference(),
                step_index: Some(0)
            }
        )
        .await
        .is_err()
    );
    assert_eq!(resolver.calls.load(Ordering::Acquire), 0);
    let accepted = draft.transition(Status::Accepted, NOW).unwrap();
    selection.planning = None;
    selection.accepted = Some(accepted.reference());
    f.host
        .update_plan(
            "accept-plan",
            NOW,
            Some(accepted.clone()),
            selection,
            &ignore,
        )
        .await
        .unwrap();
    let result = mode::execute(
        f.host.clone(),
        input(),
        resolver.clone(),
        f.files.clone(),
        Arc::new(ignore),
        mode::Execution {
            plan: accepted.reference(),
            step_index: Some(0),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(f.root.join("first.txt")).unwrap(),
        "first step"
    );
    let plan = result.plan.unwrap();
    assert_eq!(plan.status, Status::Accepted);
    assert_eq!(plan.next_step(), Some(1));
    assert_eq!(plan.reviews()[0].verification, vec!["inspected output"]);
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        planning::resolve(&branch, &plan.reference())
            .unwrap()
            .next_step(),
        Some(1)
    );
    assert!(planning::selection(&branch).unwrap().executing.is_none());
    assert!(f.host.settled().unwrap());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].to_string().contains("Execute only step 1"));
    assert!(
        !requests[0]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["function"]["name"] == "submit_plan")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_plan_commands_answer_accept_and_clear_exact_revisions_without_execution() {
    use ag1_core::runtime::planning::{self, Plan, Selection, Status};
    use ag1_platform::session::{
        commands::{RunSettings, SessionCommands},
        registry::{PreparedRuntime, RuntimeRegistry},
    };
    use ag1_protocol::wire as w;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-state-oracle.json"
    ))
    .unwrap();
    let f = fixture("http://127.0.0.1:1", "disabled");
    for (key, name) in [("save-draft", "draft"), ("save-questions", "clarifying")] {
        let plan: Plan = serde_json::from_value(oracle[name].clone()).unwrap();
        let selection = Selection {
            version: 1,
            planning: Some(plan.reference()),
            accepted: None,
            executing: None,
            updated_at: NOW.into(),
        };
        f.host
            .update_plan(key, NOW, Some(plan), selection, &ignore)
            .await
            .unwrap();
    }
    let registry = RuntimeRegistry::new(
        PreparedRuntime::from_shared(
            f.host.clone(),
            Arc::new(Resolver::new(false)),
            f.files.clone(),
        ),
        1,
    )
    .unwrap();
    let commands = SessionCommands::new(
        registry.selected().unwrap(),
        "planner".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings::default(),
    )
    .unwrap();
    let request = |id: &str, revision: u64, action: &str, args: Value| w::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: commands.scope().clone(),
        expected_revision: w::Sequence::new(revision),
        extension: None,
        command: w::Command::Plan {
            action: action.into(),
            args,
        },
    };
    let answer = request(
        "answer",
        4,
        "answer",
        json!({"planId":"plan-b","revision":1,"questionIndex":0,"answer":"Web"}),
    );
    assert_plan_command(&commands, answer).await;
    let accept = request(
        "accept",
        6,
        "accept",
        json!({"planId":"plan-a","revision":1}),
    );
    let accepted = commands.dispatch(accept.clone()).await;
    assert!(
        matches!(accepted.result, w::CommandResult::Accepted { .. }),
        "{accepted:?}"
    );
    assert_eq!(commands.dispatch(accept).await.result, accepted.result);
    assert_eq!(f.host.journal_revision().unwrap(), 8);
    let stale = request(
        "stale-answer",
        4,
        "answer",
        json!({"planId":"plan-b","revision":1,"questionIndex":0,"answer":"Native"}),
    );
    assert!(matches!(
        commands.dispatch(stale).await.result,
        w::CommandResult::Rejected {
            error: w::ProtocolError::StaleContext
        }
    ));
    for (id, revision, action, index) in [
        ("step-one", 8, "complete_step", 0),
        ("step-two", 10, "complete_step", 1),
        ("reopen", 12, "reopen_step", 0),
    ] {
        assert_plan_command(
            &commands,
            request(
                id,
                revision,
                action,
                json!({"planId":"plan-a","revision":1,"stepIndex":index}),
            ),
        )
        .await;
    }
    let clear = request(
        "clear",
        14,
        "clear",
        json!({"planId":"plan-a","revision":1}),
    );
    assert_plan_command(&commands, clear).await;
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        planning::resolve(
            &branch,
            &planning::PlanRef {
                plan_id: "plan-a".into(),
                revision: 1
            }
        )
        .unwrap()
        .status,
        Status::Discarded
    );
    assert_eq!(
        planning::resolve(
            &branch,
            &planning::PlanRef {
                plan_id: "plan-b".into(),
                revision: 1
            }
        )
        .unwrap()
        .clarification_answers,
        Some(vec!["Web".into()])
    );
    assert!(planning::selection(&branch).unwrap().accepted.is_none());
    assert_eq!(branch.len(), 16);
    assert!(commands.run_next(&ignore).await.unwrap().is_none());
}

struct PlanFault(ag1_storage::Boundary, AtomicBool);
impl ag1_storage::FaultInjector for PlanFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        boundary: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command == "plan-write" && boundary == self.0 && self.1.swap(false, Ordering::AcqRel) {
            return Err(ag1_storage::StorageError::Injected(boundary));
        }
        Ok(())
    }
}

struct PlanningFault(ag1_storage::Boundary, AtomicBool);
impl ag1_storage::FaultInjector for PlanningFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        boundary: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command.starts_with("plan-")
            && boundary == self.0
            && self.1.swap(false, Ordering::AcqRel)
        {
            return Err(ag1_storage::StorageError::Injected(boundary));
        }
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn planning_mode_owns_dropped_response_and_failed_publication_never_leaks_tool_configuration()
{
    use ag1_platform::session::planning;
    for fault in [
        None,
        Some(ag1_storage::Boundary::BeforeCommit),
        Some(ag1_storage::Boundary::BeforeAck),
    ] {
        let (endpoint, server) = server(vec![
            calls(vec![(
                "questions",
                "submit_plan",
                json!({"summary":"Choose a target","openQuestions":[{"question":"Which platform?","options":["Web","Native"]}]}),
            )]),
            stop(),
        ]);
        let injector = fault.map(|b| {
            Arc::new(PlanningFault(b, AtomicBool::new(true))) as Arc<dyn ag1_storage::FaultInjector>
        });
        let f = fixture_configured(&endpoint, "disabled", vec![], injector, None, 1);
        let original = f.host.configuration().unwrap().tools;
        let resolver = Arc::new(Resolver::new(true));
        let response = {
            let host = f.host.clone();
            let tools = f.files.clone();
            let resolver = resolver.clone();
            tokio::spawn(async move {
                planning::run(
                    host,
                    input(),
                    resolver,
                    tools,
                    Arc::new(ignore),
                    planning::Options {
                        allow_inspection: false,
                        ..Default::default()
                    },
                )
                .await
            })
        };
        resolver.entered.notified().await;
        assert!(!f.host.settled().unwrap());
        assert_eq!(f.host.configuration().unwrap().tools, original);
        assert!(
            f.host
                .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
                .await
                .is_err()
        );
        if fault.is_none() {
            response.abort();
        }
        resolver.release.notify_one();
        if fault.is_some() {
            assert!(response.await.unwrap().is_err());
        } else {
            let _ = response.await;
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let saved = ag1_core::runtime::planning::plans(
                    &ag1_core::task::branch_values(
                        &f.store
                            .journal_snapshot(&SessionId::new("session").unwrap())
                            .unwrap(),
                    )
                    .unwrap(),
                );
                if (fault.is_some() && !f.host.persistence_healthy().unwrap())
                    || (f.host.settled().unwrap() && !saved.is_empty())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|e| {
            panic!(
                "{fault:?}: {e:?}, settled={:?}, healthy={:?}, calls={}, branch={:?}",
                f.host.settled(),
                f.host.persistence_healthy(),
                resolver.calls.load(Ordering::Acquire),
                ag1_core::task::branch_values(
                    &f.store
                        .journal_snapshot(&SessionId::new("session").unwrap())
                        .unwrap()
                )
            )
        });
        assert_eq!(f.host.configuration().unwrap().tools, original);
        let stored = ag1_core::task::branch_values(
            &f.store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        let plans = ag1_core::runtime::planning::plans(&stored);
        assert_eq!(
            plans.len(),
            usize::from(fault != Some(ag1_storage::Boundary::BeforeCommit))
        );
        if fault.is_none() {
            assert_eq!(
                plans[0].status,
                ag1_core::runtime::planning::Status::Clarifying
            );
        } else {
            assert!(
                f.host
                    .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
                    .await
                    .is_err()
            );
        }
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_and_selection_publish_atomically_and_failed_custody_cannot_advance_native_state() {
    use ag1_core::runtime::planning::{self, Plan, Selection, Status};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/plan-state-oracle.json"
    ))
    .unwrap();
    for fault in [
        None,
        Some(ag1_storage::Boundary::BeforeCommit),
        Some(ag1_storage::Boundary::BeforeAck),
    ] {
        let injector = fault.map(|b| {
            Arc::new(PlanFault(b, AtomicBool::new(true))) as Arc<dyn ag1_storage::FaultInjector>
        });
        let f = fixture_configured("http://127.0.0.1:1", "disabled", vec![], injector, None, 1);
        let plan: Plan = serde_json::from_value(oracle["draft"].clone()).unwrap();
        let mut selected = Selection {
            version: 1,
            planning: Some(plan.reference()),
            accepted: None,
            executing: None,
            updated_at: NOW.into(),
        };
        let result = f
            .host
            .update_plan(
                "plan-write",
                NOW,
                Some(plan.clone()),
                selected.clone(),
                &ignore,
            )
            .await;
        assert_eq!(result.is_ok(), fault.is_none(), "{fault:?}: {result:?}");
        let journal = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        assert_eq!(
            journal.entries.len(),
            if fault == Some(ag1_storage::Boundary::BeforeCommit) {
                0
            } else {
                2
            }
        );
        if fault.is_some() {
            assert_eq!(f.host.journal_revision().unwrap(), 0);
            assert!(!f.host.persistence_healthy().unwrap());
            assert!(
                f.host
                    .update_plan("must-not-advance", NOW, Some(plan), selected, &ignore)
                    .await
                    .is_err()
            );
            continue;
        }
        let accepted = plan.transition(Status::Accepted, NOW).unwrap();
        selected.planning = None;
        selected.accepted = Some(accepted.reference());
        f.host
            .update_plan(
                "plan-accept",
                NOW,
                Some(accepted.clone()),
                selected.clone(),
                &ignore,
            )
            .await
            .unwrap();
        assert_eq!(f.host.journal_revision().unwrap(), 4);
        selected.executing = Some(accepted.reference());
        assert!(
            f.host
                .update_plan(
                    "invalid-execution-selection",
                    NOW,
                    None,
                    selected.clone(),
                    &ignore
                )
                .await
                .is_err()
        );
        assert_eq!(f.host.journal_revision().unwrap(), 4);
        let executing = accepted.begin_step(0, NOW).unwrap();
        f.host
            .update_plan(
                "plan-begin",
                NOW,
                Some(executing.clone()),
                selected.clone(),
                &ignore,
            )
            .await
            .unwrap();
        let discarded = executing.transition(Status::Discarded, NOW).unwrap();
        selected.accepted = None;
        selected.executing = None;
        f.host
            .update_plan(
                "plan-discard",
                NOW,
                Some(discarded.clone()),
                selected,
                &ignore,
            )
            .await
            .unwrap();
        let branch = ag1_core::task::branch_values(
            &f.store
                .journal_snapshot(&SessionId::new("session").unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            planning::resolve(&branch, &discarded.reference())
                .unwrap()
                .status,
            Status::Discarded
        );
        assert!(planning::selection(&branch).unwrap().executing.is_none());
        assert_eq!(branch.len(), 8);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_planning_submission_retains_structured_evidence_without_authorizing_writes() {
    let (endpoint, server) = server(vec![
        calls(vec![
            (
                "plan",
                "submit_plan",
                json!({"summary":"  Review before execution  ","steps":["Inspect files","Implement changes"],"openQuestions":[{"question":"Which platform?","options":["Web","Native"]}]}),
            ),
            (
                "forbidden",
                "write",
                json!({"path":"must-not-exist","content":"not approved"}),
            ),
        ]),
        stop(),
    ]);
    let f = fixture(&endpoint, "disabled");
    let original = f.host.configuration().unwrap();
    let result = ag1_platform::session::planning::run(
        f.host.clone(),
        input(),
        Arc::new(Resolver::new(false)),
        f.files.clone(),
        Arc::new(ignore),
        Default::default(),
    )
    .await
    .unwrap();
    let plan = result.plan.unwrap();
    assert_eq!(plan.status, ag1_core::runtime::planning::Status::Draft);
    assert_eq!(plan.steps, vec!["Inspect files", "Implement changes"]);
    assert_eq!(f.host.configuration().unwrap().tools, original.tools);
    assert_eq!(
        f.host.configuration().unwrap().system_prompt,
        original.system_prompt
    );
    assert!(!f.root.join("must-not-exist").exists());
    let branch = ag1_core::task::branch_values(
        &f.store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let result = branch
        .iter()
        .find(|e| e["message"]["toolCallId"] == "plan")
        .unwrap();
    assert_eq!(
        result["message"]["details"]["plan"]["summary"],
        "Review before execution"
    );
    assert_eq!(
        result["message"]["details"]["plan"]["openQuestions"][0]["options"],
        json!(["Web", "Native"])
    );
    assert_eq!(
        branch
            .iter()
            .filter(|e| e["message"]["toolCallId"] == "plan")
            .count(),
        1
    );
    assert!(f.host.settled().unwrap());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    let names = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"submit_plan"));
    assert!(!names.contains(&"write") && !names.contains(&"edit"));
    assert!(requests[0].to_string().contains("planning-mode-guidance"));
}

async fn assert_plan_command(
    commands: &Arc<ag1_platform::session::commands::SessionCommands>,
    envelope: ag1_protocol::wire::CommandEnvelope,
) {
    use ag1_protocol::wire as w;
    let mut query = envelope.clone();
    query.command = w::Command::CommandResult {
        command: envelope.id.clone(),
    };
    query.id = format!("query-{}", envelope.id.as_str())
        .try_into()
        .unwrap();
    let reply = commands.dispatch(envelope).await;
    assert!(
        matches!(reply.result, w::CommandResult::Accepted { .. }),
        "{reply:?}"
    );
    let result = commands.dispatch(query).await;
    assert!(
        matches!(result.result,w::CommandResult::Settled{ref operation} if operation.status==w::OperationStatus::Completed),
        "{result:?}"
    );
}
