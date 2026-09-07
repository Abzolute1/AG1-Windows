use super::*;

fn continuation() -> TurnInput {
    let mut turn = input();
    turn.command = "owned-continuation".into();
    turn.message = json!({"role":"custom","customType":"runtime_completion_gate","content":"Complete the existing task and verify the result.","display":false,"timestamp":1000});
    turn
}
fn branch(fixture: &Fixture) -> Vec<Value> {
    ag1_core::task::branch_values(
        &fixture
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_continuation_keeps_user_task_and_runs_current_model_after_custom_ack() {
    let mut promise = stop();
    promise["choices"][0]["delta"]["content"] = json!("Let me fix it:");
    let (origin, server) = server(vec![
        promise,
        calls(vec![(
            "complete-write",
            "write",
            json!({"path":"completed.txt","content":"done"}),
        )]),
        stop(),
    ]);
    let fixture = fixture(&origin, "disabled");
    let first = fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert!(ag1_core::runtime::completion::needs_continuation(
        &first.assistant
    ));
    let before = branch(&fixture)
        .into_iter()
        .filter(|e| e["customType"] == "task_state")
        .collect::<Vec<_>>();
    let mut model = fixture.model.clone();
    model["id"] = json!("continuation-model");
    fixture
        .host
        .select_model(
            "change-for-continuation",
            "change-for-continuation",
            NOW,
            model,
            &ignore,
        )
        .await
        .unwrap();
    let continued = fixture
        .host
        .continue_with(
            continuation(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert_eq!(continued.generations, 2);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("completed.txt")).unwrap(),
        "done"
    );
    let after = branch(&fixture);
    assert_eq!(
        after
            .iter()
            .filter(|e| e["message"]["role"] == "user")
            .count(),
        1
    );
    assert_eq!(
        after
            .iter()
            .filter(|e| e["customType"] == "task_state")
            .cloned()
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(
        after
            .iter()
            .filter(|e| e["message"]["customType"] == "runtime_completion_gate")
            .count(),
        1
    );
    let requests = server.join().unwrap();
    assert_eq!(requests[1]["model"], "continuation-model");
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("Complete the existing task")
    );
    assert!(fixture.host.settled().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn internal_continuation_keeps_repetition_observations_and_cannot_bypass_pause() {
    let read = |id: &str| calls(vec![(id, "read", json!({"path":"source.txt"}))]);
    let (origin, server) = server(vec![
        read("one"),
        read("two"),
        stop(),
        read("three"),
        read("four"),
    ]);
    let fixture = fixture(&origin, "disabled");
    std::fs::write(fixture.root.join("source.txt"), "same evidence").unwrap();
    fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    let result = fixture
        .host
        .continue_with(
            continuation(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    assert_eq!(result.generations, 2);
    assert_eq!(result.assistant["nativePolicy"], "repetition_guard");
    let revision = fixture.host.journal_revision().unwrap();
    let mut another = continuation();
    another.command = "cannot-bypass".into();
    assert!(matches!(
        fixture
            .host
            .continue_with(
                another,
                &Resolver::new(false),
                fixture.files.clone(),
                &ignore
            )
            .await,
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(fixture.host.journal_revision().unwrap(), revision);
    assert_eq!(server.join().unwrap().len(), 5);
    assert!(fixture.host.settled().unwrap());
}

struct ContinuationFault(ag1_storage::Boundary);
impl ag1_storage::FaultInjector for ContinuationFault {
    fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
        Ok(())
    }
    fn check_journal_command(
        &self,
        command: &str,
        point: ag1_storage::Boundary,
    ) -> ag1_storage::StorageResult<()> {
        if command == "owned-continuation" && point == self.0 {
            return Err(ag1_storage::StorageError::Injected(point));
        }
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuation_requires_its_receipt_and_preserves_prior_task_on_fault() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let (origin, server) = server(vec![stop()]);
        let fixture = fixture_seeded_with_fault(
            &origin,
            "disabled",
            vec![],
            Some(Arc::new(ContinuationFault(point))),
        );
        fixture
            .host
            .run(
                input(),
                &Resolver::new(false),
                fixture.files.clone(),
                &ignore,
            )
            .await
            .unwrap();
        assert!(
            fixture
                .host
                .continue_with(
                    continuation(),
                    &Resolver::new(false),
                    fixture.files.clone(),
                    &ignore
                )
                .await
                .is_err()
        );
        assert!(!fixture.host.persistence_healthy().unwrap());
        let branch = branch(&fixture);
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
                .filter(|e| e["customType"] == "task_state")
                .count(),
            1
        );
        assert_eq!(
            branch
                .iter()
                .filter(|e| e["message"]["customType"] == "runtime_completion_gate")
                .count(),
            usize::from(point == ag1_storage::Boundary::BeforeAck)
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn continuation_rejects_fresh_sessions_and_user_role_without_writes() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    for turn in [continuation(), input()] {
        assert!(matches!(
            fixture
                .host
                .continue_with(turn, &Resolver::new(false), fixture.files.clone(), &ignore)
                .await,
            Err(DomainError::InvalidInput(_))
        ));
    }
    assert_eq!(fixture.host.journal_revision().unwrap(), 0);
    assert!(branch(&fixture).is_empty());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_continuation_drains_preparation_and_keeps_custom_message() {
    let (origin, server) = server(vec![stop()]);
    let fixture = fixture(&origin, "disabled");
    fixture
        .host
        .run(
            input(),
            &Resolver::new(false),
            fixture.files.clone(),
            &ignore,
        )
        .await
        .unwrap();
    let resolver = Arc::new(Resolver::new(true));
    let worker = resolver.clone();
    let host = fixture.host.clone();
    let files = fixture.files.clone();
    let run = tokio::spawn(async move {
        host.continue_with(continuation(), &*worker, files, &ignore)
            .await
    });
    resolver.entered.notified().await;
    fixture.host.cancel().unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap(),
        Err(DomainError::Cancelled)
    ));
    assert!(fixture.host.settled().unwrap());
    assert!(fixture.host.persistence_healthy().unwrap());
    assert_eq!(
        branch(&fixture)
            .iter()
            .filter(|e| e["message"]["customType"] == "runtime_completion_gate")
            .count(),
        1
    );
    assert_eq!(server.join().unwrap().len(), 1);
}
