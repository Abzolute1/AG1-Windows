use super::*;
use ag1_platform::session::tree::{TreeInput, TreeOptions};
fn navigation(target: &str, summarize: bool) -> TreeInput {
    TreeInput {
        command: "navigate".into(),
        timestamp: NOW.into(),
        options: TreeOptions {
            target: target.into(),
            summarize,
            custom_instructions: None,
            replace_instructions: false,
            label: None,
        },
        reserve_tokens: 16384,
        summary_timeout_ms: 5000,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_navigation_keeps_originals_and_summarizes_through_the_shared_provider() {
    let (endpoint, server) = server(vec![stop(), stop()]);
    let f = fixture_seeded(&endpoint, "disabled", summary_history());
    let resolver = Arc::new(Resolver::new(false));
    let before = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let result = f
        .host
        .navigate_tree(
            navigation("seed-2", true),
            resolver.clone(),
            f.files.clone(),
            Arc::new(ignore),
        )
        .await
        .unwrap();
    assert_eq!(
        result.editor_text.as_deref(),
        Some("Continue with the current task.")
    );
    let summary = result.summary_entry.unwrap();
    assert_eq!(summary["parentId"], "seed-1");
    assert_eq!(summary["fromId"], "seed-3");
    assert!(
        summary["summary"]
            .as_str()
            .unwrap()
            .starts_with("The user explored a different conversation branch")
    );
    assert!(summary["nativeTrajectory"]["id"].is_string());
    assert_eq!(result.journal_revision, before.revision + 1);
    let after = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert!(after.entries.starts_with(&before.entries));
    f.host
        .run(input(), resolver.as_ref(), f.files.clone(), &ignore)
        .await
        .unwrap();
    let requests = server.join().unwrap();
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("Current work has a partial investigation")
    );
    assert!(
        !requests[1]["messages"]
            .to_string()
            .contains("Current work has a partial investigation")
    );
    assert!(
        requests[1]["messages"]
            .to_string()
            .contains("The user explored a different conversation branch")
    );
    assert!(requests[0].get("tools").is_none());
    assert_eq!(requests[0]["max_completion_tokens"], 2048);
    assert!(f.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_callbacks_veto_override_and_notification_crash_preserve_atomic_history() {
    for (code, expected) in [
        (
            "api.on('session_before_tree',e=>{if(!(e.signal instanceof AbortSignal))throw Error('signal');return {cancel:true};});",
            "veto",
        ),
        (
            "api.on('session_before_tree',()=>({summary:{summary:'Retained branch summary',details:{opaque:7}},label:'saved'}));api.on('session_tree',()=>process.exit(23));",
            "summary",
        ),
        (
            "api.on('session_before_tree',()=>({summary:{summary:null}}));",
            "invalid",
        ),
    ] {
        let mut f = fixture_seeded("http://127.0.0.1:1", "disabled", summary_history());
        let extension =
            super::extensions::extensions(&f, &format!("export default api=>{{{code}}};"));
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        let resolver = Arc::new(Resolver::new(false));
        let result = f
            .host
            .navigate_tree(
                navigation("seed-2", true),
                resolver.clone(),
                f.files.clone(),
                Arc::new(ignore),
            )
            .await;
        let saved = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        match expected {
            "veto" => {
                assert!(matches!(result, Err(DomainError::Cancelled)));
                assert_eq!(saved.revision, 4);
            }
            "invalid" => {
                assert!(matches!(result, Err(DomainError::InvalidInput(_))));
                assert_eq!(saved.revision, 4);
            }
            _ => {
                let result = result.unwrap();
                assert_eq!(result.journal_revision, 6);
                assert_eq!(result.summary_entry.unwrap()["details"]["opaque"], 7);
                assert_eq!(saved.entries.len(), 6);
                let label: Value = serde_json::from_slice(&saved.entries[5].normalized).unwrap();
                assert_eq!(label["label"], "saved");
            }
        }
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        assert!(f.host.settled().unwrap());
        extension.stop().unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_session_while_tree_summary_waits_cannot_select_stale_branch() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let f = fixture_seeded(
        &format!("http://{}", listener.local_addr().unwrap()),
        "disabled",
        summary_history(),
    );
    let resolver = Arc::new(Resolver::new(true));
    let host = f.host.clone();
    let provider = resolver.clone();
    let files = f.files.clone();
    let task = tokio::spawn(async move {
        host.navigate_tree(
            navigation("seed-2", true),
            provider,
            files,
            Arc::new(ignore),
        )
        .await
    });
    resolver.entered.notified().await;
    let mut model = f.model.clone();
    model["id"] = json!("replacement");
    f.host
        .select_model("new-model", "new-model", NOW, model, &ignore)
        .await
        .unwrap();
    resolver.release.notify_one();
    assert!(matches!(task.await.unwrap(), Err(DomainError::Conflict)));
    let saved = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(saved.leaf.as_deref(), Some("new-model"));
    assert!(matches!(listener.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock));
    assert!(f.host.settled().unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_leaf_navigation_leaves_active_generation_running() {
    let (endpoint, server) = server(vec![stop()]);
    let f = fixture_seeded(&endpoint, "disabled", summary_history());
    let resolver = Arc::new(Resolver::new(true));
    let provider = resolver.clone();
    let host = f.host.clone();
    let files = f.files.clone();
    let task =
        tokio::spawn(async move { host.run(input(), provider.as_ref(), files, &ignore).await });
    resolver.entered.notified().await;
    let saved = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let result = f
        .host
        .navigate_tree(
            navigation(saved.leaf.as_deref().unwrap(), true),
            resolver.clone(),
            f.files.clone(),
            Arc::new(ignore),
        )
        .await
        .unwrap();
    assert_eq!(result.journal_revision, saved.revision);
    assert!(result.summary_entry.is_none());
    resolver.release.notify_one();
    task.await.unwrap().unwrap();
    assert_eq!(server.join().unwrap().len(), 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn navigation_rejects_incomplete_tool_boundary_and_root_selection_preserves_originals() {
    let f = fixture_seeded(
        "http://127.0.0.1:1",
        "disabled",
        vec![
            json!({"role":"user","content":"Read a file"}),
            json!({"role":"assistant","content":[{"type":"toolCall","id":"call","name":"read","arguments":{"path":"a"}}],"stopReason":"toolUse"}),
            json!({"role":"toolResult","toolName":"read","toolCallId":"call","content":[{"type":"text","text":"Evidence"}],"isError":false}),
            json!({"role":"assistant","content":[{"type":"text","text":"Complete"}],"stopReason":"stop"}),
        ],
    );
    let resolver = Arc::new(Resolver::new(false));
    assert!(matches!(
        f.host
            .navigate_tree(
                navigation("seed-1", false),
                resolver.clone(),
                f.files.clone(),
                Arc::new(ignore)
            )
            .await,
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(f.host.journal_revision().unwrap(), 4);
    let result = f
        .host
        .navigate_tree(
            navigation("seed-0", false),
            resolver,
            f.files.clone(),
            Arc::new(ignore),
        )
        .await
        .unwrap();
    assert_eq!(result.editor_text.as_deref(), Some("Read a file"));
    assert_eq!(result.leaf, None);
    assert_eq!(result.journal_revision, 5);
    let saved = f
        .store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    assert_eq!(saved.entries.len(), 4);
    assert_eq!(saved.leaf, None);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tree_summary_and_label_commit_atomically_across_lost_acknowledgement() {
    struct TreeFault(ag1_storage::Boundary);
    impl ag1_storage::FaultInjector for TreeFault {
        fn check(&self, _: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            Ok(())
        }
        fn check_journal_command(
            &self,
            command: &str,
            point: ag1_storage::Boundary,
        ) -> ag1_storage::StorageResult<()> {
            if command == "navigate" && point == self.0 {
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
        let mut f = fixture_seeded_with_fault(
            "http://127.0.0.1:1",
            "disabled",
            summary_history(),
            Some(Arc::new(TreeFault(point))),
        );
        let extension = super::extensions::extensions(
            &f,
            "export default api=>{api.on('session_before_tree',()=>({summary:{summary:'Checkpoint'},label:'branch'}));};",
        );
        Arc::get_mut(&mut f.files).unwrap().adapters.extensions = Some(extension.clone());
        assert!(
            f.host
                .navigate_tree(
                    navigation("seed-2", true),
                    Arc::new(Resolver::new(false)),
                    f.files.clone(),
                    Arc::new(ignore)
                )
                .await
                .is_err()
        );
        let saved = f
            .store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap();
        if point == ag1_storage::Boundary::BeforeCommit {
            assert_eq!(saved.revision, 4);
            assert_eq!(saved.leaf.as_deref(), Some("seed-3"));
        } else {
            assert_eq!(saved.revision, 6);
            assert_eq!(saved.entries.len(), 6);
            let branch = ag1_core::task::branch_values(&saved).unwrap();
            assert_eq!(branch.len(), 4);
            assert_eq!(branch[2]["summary"], "Checkpoint");
            assert_eq!(branch[3]["label"], "branch");
        }
        assert!(
            f.host
                .run(input(), &Resolver::new(false), f.files.clone(), &ignore)
                .await
                .is_err()
        );
        let resumed = SessionRuntime::resume(
            SessionId::new("session").unwrap(),
            2,
            &saved,
            f.host.configuration().unwrap(),
        )
        .unwrap();
        assert_eq!(
            resumed.branch(),
            ag1_core::task::branch_values(&saved).unwrap()
        );
        extension.stop().unwrap();
    }
}
