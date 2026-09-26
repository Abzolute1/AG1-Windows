use super::*;

fn owner(f: &Fixture, resolver: Arc<dyn ProviderResolver>) -> Arc<SessionCommands> {
    SessionCommands::new(
        Selection {
            epoch: 1,
            services: Arc::new(PreparedRuntime::from_shared(
                f.host.clone(),
                resolver,
                f.files.clone(),
            )),
        },
        "compact-client".to_owned().try_into().unwrap(),
        Arc::new(FixedClock),
        RunSettings {
            compaction: compact_input().settings,
            ..Default::default()
        },
    )
    .unwrap()
}
fn command(c: &SessionCommands, id: &str, revision: u64) -> w::CommandEnvelope {
    let mut envelope = prompt(c, id, revision, "");
    envelope.command = w::Command::Session {
        action: "compact".into(),
        args: json!({"focus":"Preserve exact evidence"}),
    };
    envelope
}
const OVERRIDE: &str = r#"import fs from 'node:fs';export default api=>{api.on('session_before_compact',(e,ctx)=>{fs.appendFileSync(ctx.cwd+'/wire-compactions','once\n');return {compaction:{summary:'Wire extension summary',firstKeptEntryId:e.preparation.firstKeptEntryId,tokensBefore:e.preparation.tokensBefore}};});};"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_command_selects_before_waiting_prompt_and_duplicate_delivery_cannot_branch_again() {
    let (endpoint, server) = server(vec![stop()]);
    let f = fixture_seeded(&endpoint, "disabled", summary_history());
    let c = owner(&f, Arc::new(Resolver::new(false)));
    let revision = f.host.journal_revision().unwrap();
    accepted(
        c.dispatch(prompt(&c, "later", revision, "A new direction"))
            .await
            .result,
    );
    let mut request = command(&c, "tree", revision);
    request.command = w::Command::Session {
        action: "tree".into(),
        args: json!({"target":"seed-2"}),
    };
    let operation = accepted(c.dispatch(request.clone()).await.result);
    assert_eq!(
        c.run_next(&ignore).await.unwrap().unwrap().operation,
        operation
    );
    let saved = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(saved.leaf.as_deref(), Some("seed-1"));
    assert_eq!(saved.entries.len(), 4);
    accepted(c.dispatch(request).await.result);
    assert_eq!(f.host.journal_revision().unwrap(), 5);
    c.run_next(&ignore).await.unwrap().unwrap();
    let requests = server.join().unwrap();
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("A new direction")
    );
    assert!(
        !requests[0]["messages"]
            .to_string()
            .contains("Current work has a partial investigation")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compaction_terminal_fault_preserves_the_checkpoint_without_replaying_the_callback() {
    for point in [
        ag1_storage::Boundary::BeforeCommit,
        ag1_storage::Boundary::BeforeAck,
    ] {
        let fault = Arc::new(ArmedStorageFault {
            point,
            armed: AtomicBool::new(false),
        });
        let mut f = fixture_seeded_with_fault(
            "http://127.0.0.1:1",
            "disabled",
            summary_history(),
            Some(fault.clone()),
        );
        let extension = super::super::extensions::extensions(&f, OVERRIDE);
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let resolver = Arc::new(Resolver::new(false));
        let c = owner(&f, resolver.clone());
        let request = command(&c, "compact", f.host.journal_revision().unwrap());
        accepted(c.dispatch(request.clone()).await.result);
        let listener = |event: &SessionEvent| {
            if matches!(event, SessionEvent::Committed { .. }) {
                fault.armed.store(true, Ordering::SeqCst);
            }
        };
        assert!(c.run_next(&listener).await.is_err());
        assert!(f.host.settled().unwrap());
        assert!(f.host.persistence_healthy().unwrap());
        accepted(c.dispatch(request).await.result);
        assert!(matches!(
            c.run_next(&ignore).await,
            Err(DomainError::Conflict)
        ));
        let id = "compact".to_owned().try_into().unwrap();
        if point == ag1_storage::Boundary::BeforeAck {
            assert_eq!(
                c.reconcile_terminal(id).await.unwrap().status,
                w::OperationStatus::Completed
            );
            assert!(c.run_next(&ignore).await.unwrap().is_none());
        } else {
            assert!(matches!(
                c.reconcile_terminal(id).await,
                Err(DomainError::DurabilityUnknown)
            ));
        }
        let entries = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap()
            .entries;
        assert_eq!(
            entries
                .iter()
                .filter(
                    |e| serde_json::from_slice::<Value>(&e.normalized).unwrap()["type"]
                        == "compaction"
                )
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(f.root.join("wire-compactions")).unwrap(),
            "once\n"
        );
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        extension.stop().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manual_compaction_precedes_waiting_prompts_and_duplicate_delivery_cannot_repeat_it() {
    let (endpoint, server) = server(vec![stop()]);
    let mut f = fixture_seeded(&endpoint, "disabled", summary_history());
    let extension = super::super::extensions::extensions(&f, OVERRIDE);
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(false));
    let c = owner(&f, resolver.clone());
    let revision = f.host.journal_revision().unwrap();
    assert!(
        c.handshake()
            .capabilities
            .iter()
            .any(|s| s == "session.compact")
    );
    accepted(
        c.dispatch(prompt(&c, "later", revision, "Continue after compaction"))
            .await
            .result,
    );
    let request = command(&c, "compact", revision);
    let operation = accepted(c.dispatch(request.clone()).await.result);
    assert_eq!(
        c.run_next(&ignore).await.unwrap().unwrap(),
        w::OperationView {
            operation,
            status: w::OperationStatus::Completed
        }
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(user_messages(&f.store).len(), 2);
    accepted(c.dispatch(request).await.result);
    assert!(
        matches!(lookup(&c,"compact").await,w::CommandResult::Settled {operation} if operation.status==w::OperationStatus::Completed)
    );
    assert_eq!(
        c.run_next(&ignore).await.unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    assert!(c.run_next(&ignore).await.unwrap().is_none());
    assert_eq!(user_messages(&f.store).len(), 3);
    assert_eq!(
        std::fs::read_to_string(f.root.join("wire-compactions")).unwrap(),
        "once\n"
    );
    assert!(
        server.join().unwrap()[0]["messages"]
            .to_string()
            .contains("Wire extension summary")
    );
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_manual_compaction_cancels_only_the_current_command_before_running() {
    let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
    let extension = super::super::extensions::extensions(&f, OVERRIDE);
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(true));
    let c = owner(&f, resolver.clone());
    accepted(
        c.dispatch(prompt(
            &c,
            "parent",
            f.host.journal_revision().unwrap(),
            "Pause the parent request",
        ))
        .await
        .result,
    );
    let worker = c.clone();
    let parent = tokio::spawn(async move { worker.run_next(&ignore).await });
    resolver.entered.notified().await;
    let request = command(&c, "compact", f.host.journal_revision().unwrap());
    accepted(c.dispatch(request).await.result);
    let result = tokio::time::timeout(Duration::from_secs(5), parent)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, w::OperationStatus::Cancelled);
    assert_eq!(
        c.run_next(&ignore).await.unwrap().unwrap().status,
        w::OperationStatus::Completed
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_to_string(f.root.join("wire-compactions")).unwrap(),
        "once\n"
    );
    assert!(f.host.settled().unwrap());
    extension.stop().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn targeted_cancel_drains_a_running_manual_compaction_callback() {
    let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
    let extension = super::super::extensions::extensions(
        &f,
        r#"import fs from 'node:fs';export default api=>{api.on('session_before_compact',async(e,ctx)=>{fs.writeFileSync(ctx.cwd+'/wire-compact-entered','yes');await new Promise(()=>{});});};"#,
    );
    Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
    let resolver = Arc::new(Resolver::new(false));
    let c = owner(&f, resolver.clone());
    let revision = f.host.journal_revision().unwrap();
    let operation = accepted(c.dispatch(command(&c, "compact", revision)).await.result);
    let worker = c.clone();
    let compact = tokio::spawn(async move { worker.run_next(&ignore).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f.root.join("wire-compact-entered").exists() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let mut cancel = prompt(&c, "cancel", revision, "");
    cancel.command = w::Command::Cancel { operation };
    accepted(c.dispatch(cancel).await.result);
    let result = tokio::time::timeout(Duration::from_secs(5), compact)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, w::OperationStatus::Cancelled);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.host.journal_revision().unwrap(), revision);
    assert!(f.host.settled().unwrap());
    assert!(
        matches!(lookup(&c,"compact").await,w::CommandResult::Settled {operation} if operation.status==w::OperationStatus::Cancelled)
    );
    extension.stop().unwrap();
}
