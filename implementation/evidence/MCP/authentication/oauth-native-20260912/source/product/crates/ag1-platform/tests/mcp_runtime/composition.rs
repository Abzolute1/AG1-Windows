use super::*;
use ag1_platform::{
    client::{Config, Host, McpServices},
    cutover,
    mcp::service::Service,
};
use ag1_protocol as w;
use std::os::unix::fs::PermissionsExt;

static COMMAND: AtomicUsize = AtomicUsize::new(1);
async fn dispatch(host: &Arc<Host>, action: &str, args: Value) -> Result<Value, DomainError> {
    let snapshot = host.commands.snapshot()?;
    let reply = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: format!("mcp-composition-{}", COMMAND.fetch_add(1, Ordering::SeqCst))
                .try_into()
                .unwrap(),
            scope: snapshot.scope,
            expected_revision: snapshot.revision,
            command: w::Command::Runtime {
                action: action.into(),
                args,
            },
            extension: None,
        })
        .await?;
    let w::BridgeFrame::Reply { status, data, .. } = reply else {
        panic!("reply required")
    };
    assert_ne!(status, "rejected", "{data}");
    Ok(data)
}
async fn manage(host: &Arc<Host>, args: Value) -> Value {
    dispatch(host, "mcp_manager", args).await.unwrap()
}
async fn folder(host: &Arc<Host>, action: &str, path: &std::path::Path) {
    let saved = dispatch(host, "folder_access", json!({})).await.unwrap();
    dispatch(
        host,
        "folder_access",
        json!({"revision":saved["revision"],"action":action,"path":path,"mode":null}),
    )
    .await
    .unwrap();
}
async fn connect(host: &Arc<Host>, peer: &super::super::Case) -> String {
    let list = manage(host, json!({"action":"list"})).await;
    let review = manage(host,json!({"action":"review","id":peer.server,"revision":list["servers"][0]["record"]["revision"],"test":false})).await;
    let id =
        manage(host, json!({"action":"confirm","review":review["review"]})).await["connection"]
            .as_str()
            .unwrap()
            .to_owned();
    super::super::until(|| {
        peer.manager.status(&id).unwrap().health
            == ag1_platform::mcp::manager::ConnectionHealth::Ready
    });
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn desktop_factory_binds_canonical_folder_availability_and_retires_only_its_session() {
    let peer = super::super::Case::new("stdio", Revision::Current20260728, "structured");
    peer.manager.disconnect(&peer.connection).unwrap();
    peer.manager.wait_disconnected(&peer.connection).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let general = temp.path().join("general-chat");
    let project = temp.path().join("project");
    let authority = temp.path().join("authority");
    for dir in [&general, &project, &authority] {
        std::fs::create_dir(dir).unwrap();
    }
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let registry = cutover::Registry::at(&authority).unwrap();
    registry.initialize_isolated(&general).unwrap();
    registry.initialize_isolated(&project).unwrap();
    let config: Config = serde_json::from_value(json!({"models":[{"provider":"fixture","id":"no-inference","api":"openai-completions","baseUrl":"http://127.0.0.1:1","contextWindow":16000,"maxTokens":512}],"selected":"no-inference","credential_env":null,"system_prompt":"Owned MCP composition fixture","fixture":true})).unwrap();
    let service = Service::from_manager(peer.manager.clone(), Default::default());
    let root = Host::open_mcp_rehearsal(
        &general,
        &temp.path().join("general-state"),
        config.clone(),
        &registry,
        McpServices {
            service: service.clone(),
            general_chat: true,
        },
    )
    .await
    .unwrap();
    let chat = root.open_conversation(None).await.unwrap();
    let sibling = root.open_conversation(None).await.unwrap();
    let initial = manage(&chat, json!({"action":"list"})).await;
    assert_eq!(initial["scope"], json!({"kind":"generalChat"}));
    assert_eq!(initial["servers"][0]["availableHere"], true);
    let a = connect(&chat, &peer).await;
    let b = connect(&sibling, &peer).await;
    assert_ne!(a, b);
    assert_eq!(
        manage(&chat, json!({"action":"list"})).await["connections"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        dispatch(
            &sibling,
            "mcp_manager",
            json!({"action":"disconnect","connection":a})
        )
        .await
        .is_err()
    );
    let selected = chat.registry.selected().unwrap();
    let old_port = selected.services.tools().client_mcp().unwrap();
    let old_scope = chat.commands.snapshot().unwrap().scope;
    folder(&chat, "add", &project).await;
    folder(&chat, "primary", &project).await;
    let after = chat.commands.snapshot().unwrap().scope;
    assert_eq!(after.workspace, old_scope.workspace);
    assert_eq!(after.session, old_scope.session);
    assert_ne!(after.epoch, old_scope.epoch);
    let owner = Scope {
        workspace: old_scope.workspace.as_str().into(),
        session: old_scope.session.as_str().into(),
        epoch: selected.epoch,
        revision: 0,
    };
    assert!(
        old_port
            .dispatch(&owner, &json!({"action":"list"}))
            .is_err()
    );
    assert!(matches!(
        peer.manager.status(&a).unwrap().health,
        ag1_platform::mcp::manager::ConnectionHealth::Stopped
    ));
    assert_eq!(
        peer.manager.status(&b).unwrap().health,
        ag1_platform::mcp::manager::ConnectionHealth::Ready
    );
    let attached = manage(&chat, json!({"action":"list"})).await;
    assert_eq!(attached["scope"]["kind"], "workspace");
    assert_eq!(attached["servers"][0]["availableHere"], false);
    assert!(attached["connections"].as_array().unwrap().is_empty());
    // A direct project host and a folder attached to general chat must resolve
    // to the same persistent availability identity, despite different journals.
    let mut project_config = serde_json::to_value(&config).unwrap();
    project_config["models"][0]["baseUrl"] = json!("http://127.0.0.1:2");
    let project_root = Host::open_mcp_rehearsal(
        &project,
        &temp.path().join("project-state"),
        serde_json::from_value(project_config).unwrap(),
        &registry,
        McpServices {
            service,
            general_chat: false,
        },
    )
    .await
    .unwrap();
    let project_chat = project_root.open_conversation(None).await.unwrap();
    assert_eq!(
        manage(&project_chat, json!({"action":"list"})).await["scope"],
        attached["scope"]
    );
    manage(&chat,json!({"action":"availability","id":peer.server,"revision":attached["servers"][0]["record"]["revision"],"enabled":true,"target":"current"})).await;
    assert_eq!(
        manage(&project_chat, json!({"action":"list"})).await["servers"][0]["availableHere"],
        true
    );
    connect(&chat, &peer).await;
    chat.shutdown().await.unwrap();
    assert_eq!(
        peer.manager.status(&b).unwrap().health,
        ag1_platform::mcp::manager::ConnectionHealth::Ready
    );
    project_chat.shutdown().await.unwrap();
    project_root.shutdown().await.unwrap();
    sibling.shutdown().await.unwrap();
    root.shutdown().await.unwrap();
}
