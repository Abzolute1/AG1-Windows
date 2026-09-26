#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
};
use ag1_protocol as w;
use serde_json::json;

#[tokio::test]
async fn historical_receipts_remain_queryable_after_session_switch_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    let authority = root.path().join("authority");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&authority).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let config:Config=serde_json::from_value(json!({"models":[{"id":"fixture","provider":"fixture","api":"openai-completions","baseUrl":format!("http://{}",listener.local_addr().unwrap()),"contextWindow":8192,"maxTokens":1024,"reasoning":false,"input":["text"]}],"selected":"fixture","credential_env":null,"system_prompt":"Historical receipt regression","fixture":true})).unwrap();
    let host = Host::open_with_registry(&workspace, &state, config.clone(), &registry)
        .await
        .unwrap();
    let snapshot = host.commands.snapshot().unwrap();
    let submitted = w::CommandEnvelope {
        version: Default::default(),
        id: "create-session".to_owned().try_into().unwrap(),
        scope: snapshot.scope,
        expected_revision: snapshot.revision,
        command: w::Command::Session {
            action: "new".into(),
            args: json!({}),
        },
        extension: None,
    };
    let reply = host.dispatch(submitted.clone()).await.unwrap();
    let w::BridgeFrame::Reply { status, data, .. } = reply else {
        panic!("Missing reply")
    };
    assert_eq!(status, "accepted");
    let operation = match serde_json::from_value::<w::CommandResult>(data).unwrap() {
        w::CommandResult::Accepted { operation } => operation,
        other => panic!("{other:?}"),
    };
    let mut pending_query = submitted.clone();
    pending_query.id = "inspect-pending-command".to_owned().try_into().unwrap();
    pending_query.command = w::Command::CommandResult {
        command: submitted.id.clone(),
    };
    let pending_reply = host.dispatch(pending_query).await.unwrap();
    assert!(
        matches!(pending_reply,w::BridgeFrame::Reply {ref status,..} if status == "accepted"),
        "{pending_reply:?}"
    );
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let _ = host.settled_view().await.unwrap();
            if host
                .commands
                .snapshot()
                .unwrap()
                .last_terminal
                .as_ref()
                .is_some_and(|t| t.operation == operation)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_ne!(
        host.commands.snapshot().unwrap().scope.session,
        submitted.scope.session
    );
    let mut query = submitted.clone();
    query.id = "inspect-old-command".to_owned().try_into().unwrap();
    query.command = w::Command::CommandResult {
        command: submitted.id.clone(),
    };
    let assert_completed = |reply: w::BridgeFrame| {
        let w::BridgeFrame::Reply { status, data, .. } = reply else {
            panic!("Missing reply")
        };
        assert_eq!(status, "accepted", "{data}");
        assert_eq!(
            serde_json::from_value::<w::CommandResult>(data).unwrap(),
            w::CommandResult::Settled {
                operation: w::OperationView {
                    operation: operation.clone(),
                    status: w::OperationStatus::Completed
                }
            }
        );
    };
    assert_completed(host.dispatch(query.clone()).await.unwrap());
    // Historical scope grants observation only, never a replay or a new prompt.
    assert!(host.dispatch(submitted.clone()).await.is_err());
    let mut stale = submitted.clone();
    stale.command = w::Command::Prompt {
        input: w::PromptInput::Inline {
            text: "must never generate".into(),
        },
    };
    assert!(host.dispatch(stale).await.is_err());
    let mut foreign = query.clone();
    foreign.scope.workspace = "another-workspace".to_owned().try_into().unwrap();
    assert!(host.dispatch(foreign).await.is_err());
    let selected = host.commands.snapshot().unwrap().scope.session;
    let before = host.store.snapshot(selected.as_str()).unwrap().entries;
    host.shutdown().await.unwrap();
    drop(host);
    let reopened = Host::open_with_registry(&workspace, &state, config, &registry)
        .await
        .unwrap();
    assert_completed(reopened.dispatch(query.clone()).await.unwrap());
    let mut forged = query.clone();
    forged.scope.instance = "forged-incarnation".to_owned().try_into().unwrap();
    assert!(
        matches!(reopened.dispatch(forged).await.unwrap(), w::BridgeFrame::Reply {status,..} if status == "rejected")
    );
    let mut unknown = query;
    unknown.command = w::Command::CommandResult {
        command: "never-admitted".to_owned().try_into().unwrap(),
    };
    assert!(
        matches!(reopened.dispatch(unknown).await.unwrap(), w::BridgeFrame::Reply {status,..} if status == "rejected")
    );
    assert_eq!(
        reopened.store.snapshot(selected.as_str()).unwrap().entries,
        before
    );
    reopened.shutdown().await.unwrap();
}
