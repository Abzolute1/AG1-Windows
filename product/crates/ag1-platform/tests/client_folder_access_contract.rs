#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
    session::ToolSelection,
};
use ag1_protocol as w;
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
static SERIAL: AtomicUsize = AtomicUsize::new(1);
async fn dispatch(host: &Arc<Host>, command: w::Command) -> Result<Value, ag1_core::DomainError> {
    let snapshot = host.commands.snapshot()?;
    let frame = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: format!("folders-{}", SERIAL.fetch_add(1, Ordering::SeqCst))
                .try_into()
                .unwrap(),
            scope: snapshot.scope,
            expected_revision: snapshot.revision,
            command,
            extension: None,
        })
        .await?;
    let w::BridgeFrame::Reply { status, data, .. } = frame else {
        panic!("reply")
    };
    assert_ne!(status, "rejected", "{data}");
    Ok(data)
}
async fn change(
    host: &Arc<Host>,
    action: &str,
    path: &std::path::Path,
    mode: Option<&str>,
) -> Result<Value, ag1_core::DomainError> {
    let selection = dispatch(
        host,
        w::Command::Runtime {
            action: "folder_access".into(),
            args: json!({}),
        },
    )
    .await?;
    dispatch(
        host,
        w::Command::Runtime {
            action: "folder_access".into(),
            args: json!({"revision":selection["revision"],"action":action,"path":path,"mode":mode}),
        },
    )
    .await
}
fn tool(host: &Arc<Host>, name: &str, args: Value) -> Value {
    let selected = host.registry.selected().unwrap();
    let snapshot = host.commands.snapshot().unwrap();
    let scope = ag1_core::tools::registry::Scope {
        workspace: snapshot.scope.workspace.as_str().into(),
        session: snapshot.scope.session.as_str().into(),
        epoch: selected.epoch,
        revision: selected.services.runtime().journal_revision().unwrap(),
    };
    let id = format!("call-{}", SERIAL.fetch_add(1, Ordering::SeqCst));
    let outcomes = selected
        .services
        .tools()
        .execute(
            &scope,
            &[ag1_core::runtime::session::PendingCall {
                id: id.clone(),
                name: name.into(),
                block: json!({"type":"toolCall","id":id,"name":name,"arguments":args}),
            }],
            &ToolSelection {
                journal: None,
                names: vec![name.into()],
                research_profile: ag1_core::tools::research::profile(90000., 0.),
                loaded_memory: Default::default(),
            },
            &id,
            &AtomicBool::new(false),
            &AtomicU64::new(scope.revision),
        )
        .unwrap();
    outcomes.into_iter().next().unwrap().outcome.result
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_folder_grants_rebind_same_chat_and_enforce_file_effects() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let other = temp.path().join("other folder");
    let state = temp.path().join("state");
    let authority = temp.path().join("authority");
    for path in [&workspace, &other, &authority] {
        std::fs::create_dir(path).unwrap();
    }
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(other.join("notes.txt"), "unchanged").unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    let config:Config=serde_json::from_value(json!({"models":[{"provider":"fixture","id":"no-inference","api":"openai-completions","baseUrl":"http://127.0.0.1:1","contextWindow":16000,"maxTokens":512}],"selected":"no-inference","credential_env":null,"system_prompt":"Test","fixture":true,"tooling":{"process_host":env!("CARGO_BIN_EXE_ag1-process-host"),"git":"/usr/bin/git","rg":"/usr/bin/rg","fd":"/usr/bin/fd","bubblewrap":"/usr/bin/bwrap","targets":{"node":null,"npm":null,"cargo":null,"rustc":null,"godot":null,"mounts":[]}}})).unwrap();
    let root = Host::open_with_registry(&workspace, &state, config, &registry)
        .await
        .unwrap();
    let host = root.open_conversation(None).await.unwrap();
    let sibling = root.open_conversation(None).await.unwrap();
    let before = host.commands.snapshot().unwrap().scope;
    let saved = change(&host, "add", &other, None).await.unwrap();
    assert!(
        host.dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: "stale-folders".to_owned().try_into().unwrap(),
            scope: before.clone(),
            expected_revision: host.commands.snapshot().unwrap().revision,
            command: w::Command::Runtime {
                action: "folder_access".into(),
                args: json!({"revision":saved["revision"],"action":"remove","path":other})
            },
            extension: None
        })
        .await
        .is_err()
    );
    assert_eq!(saved["folders"][1]["mode"], "read_only");
    let after = host.commands.snapshot().unwrap().scope;
    assert_eq!(before.workspace, after.workspace);
    assert_eq!(before.session, after.session);
    assert_ne!(before.epoch, after.epoch);
    assert_eq!(
        sibling.view().unwrap().details.context["folder_access"]["folders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        tool(&host, "ls", json!({"path":other}))["content"]
            .to_string()
            .contains("notes.txt")
    );
    assert!(
        tool(&host, "read", json!({"path":other.join("notes.txt")}))["content"]
            .to_string()
            .contains("unchanged")
    );
    dispatch(
        &host,
        w::Command::Runtime {
            action: "approval_policy".into(),
            args: json!({"mode":"disabled"}),
        },
    )
    .await
    .unwrap();
    assert!(
        tool(&host, "grep", json!({"path":other,"pattern":"unchanged"}))["content"]
            .to_string()
            .contains("unchanged")
    );
    assert!(
        tool(&host, "find", json!({"path":other,"pattern":"*.txt"}))["content"]
            .to_string()
            .contains("notes.txt")
    );
    let shell = tool(
        &host,
        "bash",
        json!({"command":format!("printf forbidden > '{}/shell-denied.txt'",other.display())}),
    );
    assert_eq!(shell["isError"], true, "{shell}");
    assert!(!other.join("shell-denied.txt").exists());
    assert_eq!(
        tool(
            &host,
            "write",
            json!({"path":other.join("notes.txt"),"content":"forbidden"})
        )["isError"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(other.join("notes.txt")).unwrap(),
        "unchanged"
    );
    change(&host, "mode", &other, Some("read_write"))
        .await
        .unwrap();
    dispatch(
        &host,
        w::Command::Runtime {
            action: "approval_policy".into(),
            args: json!({"mode":"autonomous"}),
        },
    )
    .await
    .unwrap();
    let job = tool(
        &host,
        "job",
        json!({"action":"start","command":"sleep 30","timeout":60}),
    );
    assert_ne!(job["isError"], true, "{job}");
    assert!(
        change(&host, "primary", &other, None).await.is_err(),
        "Running jobs must block permission changes"
    );
    let killed = tool(
        &host,
        "job",
        json!({"action":"kill","id":job["details"]["id"]}),
    );
    assert_ne!(killed["isError"], true, "{killed}");
    let shell = tool(
        &host,
        "bash",
        json!({"command":format!("printf granted > '{}/shell-allowed.txt'",other.display())}),
    );
    assert_ne!(shell["isError"], true, "{shell}");
    assert_eq!(
        std::fs::read_to_string(other.join("shell-allowed.txt")).unwrap(),
        "granted"
    );
    assert_ne!(
        tool(
            &host,
            "write",
            json!({"path":other.join("notes.txt"),"content":"allowed"})
        )["isError"],
        true
    );
    change(&host, "primary", &other, None).await.unwrap();
    assert_ne!(
        tool(
            &host,
            "write",
            json!({"path":"relative.txt","content":"new primary"})
        )["isError"],
        true
    );
    assert_eq!(
        std::fs::read_to_string(other.join("relative.txt")).unwrap(),
        "new primary"
    );
    assert!(!workspace.join("relative.txt").exists());
    assert!(
        tool(&host, "bash", json!({"command":"pwd"}))["content"]
            .to_string()
            .contains(other.to_str().unwrap())
    );
    assert_eq!(host.working_folder(false).unwrap().root(), other);
    change(&host, "mode", &other, Some("read_only"))
        .await
        .unwrap();
    assert!(host.working_folder(true).is_err());
    assert_eq!(
        tool(
            &host,
            "bash",
            json!({"command":"printf denied > primary-readonly.txt"})
        )["isError"],
        true
    );
    assert!(!other.join("primary-readonly.txt").exists());
    let editor = dispatch(
        &host,
        w::Command::Files {
            action: "read".into(),
            args: json!({"path":"relative.txt"}),
        },
    )
    .await
    .unwrap();
    assert_eq!(editor["text"], "new primary");
    change(&host, "primary", &workspace, None).await.unwrap();
    change(&host, "remove", &other, None).await.unwrap();
    assert_eq!(
        tool(&host, "read", json!({"path":other.join("notes.txt")}))["isError"],
        true
    );
    let session = after.session.as_str().to_owned();
    host.shutdown().await.unwrap();
    let reopened = root.open_conversation(Some(&session)).await.unwrap();
    assert_eq!(
        reopened.view().unwrap().details.context["folder_access"]["folders"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    change(&reopened, "add", &other, None).await.unwrap();
    change(&reopened, "primary", &other, None).await.unwrap();
    reopened.shutdown().await.unwrap();
    std::fs::rename(&other, temp.path().join("moved-reference")).unwrap();
    let recovered = root.open_conversation(Some(&session)).await.unwrap();
    assert_eq!(
        recovered.view().unwrap().details.context["folder_access"]["folders"][1]["available"],
        false
    );
    assert!(recovered.working_folder(false).is_err());
    change(&recovered, "primary", &workspace, None)
        .await
        .unwrap();
    change(&recovered, "remove", &other, None).await.unwrap();
    recovered.shutdown().await.unwrap();
    sibling.shutdown().await.unwrap();
    root.shutdown().await.unwrap();
}
