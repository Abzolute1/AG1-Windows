use super::*;
use ag1_platform::session::commands::QueueMode;

fn queued(c: &SessionCommands, id: &str, action: &str) -> w::CommandEnvelope {
    let mut envelope = prompt(c, id, c.snapshot().unwrap().revision.get(), "");
    envelope.command = w::Command::Session {
        action: action.into(),
        args: json!({"input":{"kind":"inline","text":format!("Instruction {id}")}}),
    };
    envelope
}

async fn enqueue(c: &Arc<SessionCommands>) {
    for (id, action) in [
        ("follow-one", "follow_up"),
        ("steer-one", "steer"),
        ("follow-two", "follow_up"),
        ("steer-two", "steer"),
    ] {
        let envelope = queued(c, id, action);
        let operation = accepted(c.dispatch(envelope.clone()).await.result);
        assert_eq!(accepted(c.dispatch(envelope).await.result), operation);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steering_precedes_followups_after_acknowledged_tools_in_both_queue_modes() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/runtime/queue-oracle.json"
    ))
    .unwrap();
    for mode in [QueueMode::OneAtATime, QueueMode::All] {
        for before in [false, true] {
            let mut replies = vec![calls(vec![(
                "write",
                "write",
                json!({"path":"output.txt","content":"first"}),
            )])];
            let expected = oracle["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| {
                    row["mode"]
                        == if mode == QueueMode::All {
                            "all"
                        } else {
                            "one-at-a-time"
                        }
                        && row["timing"] == if before { "before" } else { "during" }
                })
                .unwrap();
            replies.extend(std::iter::repeat_n(
                stop(),
                expected["requests"].as_array().unwrap().len() - 1,
            ));
            let (endpoint, server) = server(replies);
            let resolver = Arc::new(Resolver::new(true));
            let f = attach_with_modes(
                fixture_configured(&endpoint, "disabled", vec![], None, None, 1),
                resolver.clone(),
                vec![],
                mode,
            );
            if before {
                enqueue(&f.commands).await;
            }
            accepted(
                f.commands
                    .dispatch(prompt(
                        &f.commands,
                        "run",
                        0,
                        "Preserve the original objective",
                    ))
                    .await
                    .result,
            );
            let run = {
                let c = f.commands.clone();
                tokio::spawn(async move { c.run_next(&ignore).await })
            };
            resolver.entered.notified().await;
            if !before {
                enqueue(&f.commands).await;
            }
            resolver.release.notify_one();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(3), run)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .status,
                w::OperationStatus::Completed
            );
            let requests = server.join().unwrap();
            assert_eq!(
                requests[0].to_string().contains("Instruction steer-one"),
                before
            );
            assert!(requests[1].to_string().contains("Instruction steer-one"));
            assert!(!requests[1].to_string().contains("Instruction follow-one"));
            let users = user_messages(&f.store);
            assert_eq!(users.len(), 5);
            for (i, id) in ["steer-one", "steer-two", "follow-one", "follow-two"]
                .iter()
                .enumerate()
            {
                assert_eq!(
                    users[i + 1]["content"][0]["text"],
                    format!("Instruction {id}")
                );
                assert!(
                    matches!(lookup(&f.commands, id).await, w::CommandResult::Settled { operation } if operation.status == w::OperationStatus::Completed)
                );
            }
            assert_eq!(
                requests.len(),
                expected["requests"].as_array().unwrap().len()
            );
            assert_eq!(
                json!(
                    users
                        .iter()
                        .map(|m| m["content"][0]["text"].clone())
                        .collect::<Vec<_>>()
                ),
                expected["users"]
            );
            assert_eq!(
                requests[1].to_string().contains("Instruction steer-two"),
                before || mode == QueueMode::All
            );
            assert_eq!(
                std::fs::read_to_string(f.root.join("output.txt")).unwrap(),
                "first"
            );
            assert!(f.commands.snapshot().unwrap().queued.is_empty());
            assert!(f.host.settled().unwrap());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_steering_append_blocks_generation_and_never_requeues_delivered_input() {
    struct Fault(ag1_storage::Boundary);
    impl ag1_storage::FaultInjector for Fault {
        fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            Ok(())
        }
        fn check_journal_command(
            &self,
            command: &str,
            point: ag1_storage::Boundary,
        ) -> ag1_storage::StorageResult<()> {
            if command.starts_with("queued-") && point == self.0 {
                Err(ag1_storage::StorageError::Injected(point))
            } else {
                Ok(())
            }
        }
    }
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (endpoint, server) = server(vec![stop()]);
        let resolver = Arc::new(Resolver::new(true));
        let f = attach(
            fixture_configured(
                &endpoint,
                "disabled",
                vec![],
                Some(Arc::new(Fault(point))),
                None,
                1,
            ),
            resolver.clone(),
        );
        accepted(
            f.commands
                .dispatch(prompt(&f.commands, "run", 0, "Original"))
                .await
                .result,
        );
        let run = {
            let c = f.commands.clone();
            tokio::spawn(async move { c.run_next(&ignore).await })
        };
        resolver.entered.notified().await;
        let steer = queued(&f.commands, "steer", "steer");
        accepted(f.commands.dispatch(steer.clone()).await.result);
        resolver.release.notify_one();
        assert!(matches!(
            run.await.unwrap(),
            Err(DomainError::DurabilityUnknown)
        ));
        assert!(!f.host.persistence_healthy().unwrap());
        accepted(f.commands.dispatch(steer).await.result);
        assert!(matches!(
            rejected(lookup(&f.commands, "steer").await),
            w::ProtocolError::Unavailable { .. }
        ));
        assert!(f.commands.run_next(&ignore).await.is_err());
        assert_eq!(server.join().unwrap().len(), 1);
        assert_eq!(
            user_messages(&f.store).len(),
            if point == ag1_storage::Boundary::BeforeAck {
                2
            } else {
                1
            }
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lost_queued_delivery_receipt_recovers_publication_without_another_user_append() {
    let fault = Arc::new(ArmedStorageFault {
        point: ag1_storage::Boundary::BeforeAck,
        armed: AtomicBool::new(false),
    });
    let (endpoint, server) = server(vec![stop()]);
    let resolver = Arc::new(Resolver::new(true));
    let f = attach(
        fixture_configured(&endpoint, "disabled", vec![], Some(fault.clone()), None, 1),
        resolver.clone(),
    );
    accepted(
        f.commands
            .dispatch(prompt(&f.commands, "run", 0, "Original"))
            .await
            .result,
    );
    let run = {
        let c = f.commands.clone();
        let store = f.store.clone();
        tokio::spawn(async move {
            let listener = |event: &SessionEvent| {
                if matches!(event, SessionEvent::Committed { .. }) {
                    let journal = store.snapshot("session").unwrap();
                    let value: Value =
                        serde_json::from_slice(&journal.entries.last().unwrap().normalized)
                            .unwrap();
                    if value["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("queued-") && id.ends_with("-task"))
                    {
                        fault.armed.store(true, Ordering::SeqCst);
                    }
                }
            };
            c.run_next(&listener).await
        })
    };
    resolver.entered.notified().await;
    accepted(
        f.commands
            .dispatch(queued(&f.commands, "steer", "steer"))
            .await
            .result,
    );
    resolver.release.notify_one();
    assert_eq!(
        run.await.unwrap().unwrap().unwrap().status,
        w::OperationStatus::Failed
    );
    assert!(f.commands.run_next(&ignore).await.is_err());
    assert!(matches!(
        lookup(&f.commands, "steer").await,
        w::CommandResult::Settled { .. }
    ));
    f.commands
        .reconcile_terminal("steer".to_owned().try_into().unwrap())
        .await
        .unwrap();
    assert!(f.commands.run_next(&ignore).await.unwrap().is_none());
    assert_eq!(user_messages(&f.store).len(), 2);
    assert_eq!(server.join().unwrap().len(), 1);
}
