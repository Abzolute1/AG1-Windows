#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
};
use ag1_protocol as w;
use serde_json::json;

#[tokio::test]
async fn host_reopen_preserves_the_selected_tool_order_and_journal() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir(&workspace).unwrap();
    let authority = root.path().join("authority");
    std::fs::create_dir(&authority).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    // Hold a unique unused endpoint; this test never requests generation.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let config: Config = serde_json::from_value(json!({
        "models": [{"id":"fixture","provider":"fixture","api":"openai-completions",
            "baseUrl":format!("http://{}",listener.local_addr().unwrap()),
            "contextWindow":8192,"maxTokens":1024,"reasoning":false,"input":["text"]}],
        "selected":"fixture","credential_env":null,"system_prompt":"Restart regression",
        "fixture":true
    }))
    .unwrap();
    let host = Host::open_with_registry(&workspace, &state, config.clone(), &registry)
        .await
        .unwrap();
    // This order deliberately differs from the catalog's alphabetical order.
    let names = json!(["read", "artifact", "edit"]);
    let snapshot = host.commands.snapshot().unwrap();
    let reply = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: "select-tools".to_owned().try_into().unwrap(),
            scope: snapshot.scope.clone(),
            expected_revision: snapshot.revision,
            command: w::Command::Runtime {
                action: "tools".into(),
                args: json!({"names":names}),
            },
            extension: None,
        })
        .await
        .unwrap();
    assert!(matches!(reply,w::BridgeFrame::Reply {status,..} if status=="accepted"));
    let session = snapshot.scope.session;
    let before = host.store.snapshot(session.as_str()).unwrap();
    host.shutdown().await.unwrap();
    drop(host);
    let reopened = Host::open_with_registry(&workspace, &state, config, &registry)
        .await
        .unwrap();
    assert_eq!(reopened.commands.snapshot().unwrap().scope.session, session);
    let selected = reopened.registry.selected().unwrap();
    let configuration = selected.services.runtime().configuration().unwrap();
    let actual: Vec<_> = configuration
        .tools
        .iter()
        .map(|tool| tool["name"].clone())
        .collect();
    assert_eq!(json!(actual), names);
    assert_eq!(
        reopened.store.snapshot(session.as_str()).unwrap().entries,
        before.entries
    );
    reopened.shutdown().await.unwrap();
}
