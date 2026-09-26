#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
};
use ag1_protocol as w;
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command, sync::Arc};
fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("/usr/bin/git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Owned fixture")
        .env("GIT_COMMITTER_NAME", "Owned fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
async fn request(host: &Arc<Host>, id: &str, command: w::Command) -> w::BridgeFrame {
    let s = host.commands.snapshot().unwrap();
    host.dispatch(w::CommandEnvelope {
        version: Default::default(),
        id: id.to_string().try_into().unwrap(),
        scope: s.scope,
        expected_revision: s.revision,
        command,
        extension: None,
    })
    .await
    .unwrap()
}
fn data(frame: w::BridgeFrame) -> Value {
    let w::BridgeFrame::Reply { data, .. } = frame else {
        panic!("reply required")
    };
    data
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_local_git_confirmation_is_scoped_durable_one_shot_and_grant_bound() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let authority = temp.path().join("authority");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&authority).unwrap();
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&root).unwrap();
    git(&root, &["init", "--quiet"]);
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owned/repo.git",
        ],
    );
    std::fs::write(root.join("notes.txt"), "original\n").unwrap();
    git(&root, &["add", "notes.txt"]);
    git(&root, &["commit", "--quiet", "-m", "baseline"]);
    let config:Config=serde_json::from_value(json!({"models":[{"provider":"fixture","id":"no-inference","api":"openai-completions","baseUrl":"http://127.0.0.1:1","contextWindow":16000,"maxTokens":512}],"selected":"no-inference","credential_env":null,"system_prompt":"Test","fixture":true,"tooling":{"process_host":env!("CARGO_BIN_EXE_ag1-process-host"),"git":"/usr/bin/git","rg":"/usr/bin/rg","fd":"/usr/bin/fd","bubblewrap":"/usr/bin/bwrap","targets":{"node":null,"npm":null,"cargo":null,"rustc":null,"godot":null,"mounts":[]}}})).unwrap();
    let owner = Host::open_with_registry(&root, &temp.path().join("state"), config, &registry)
        .await
        .unwrap();
    let host = owner.open_conversation(None).await.unwrap();
    let sibling = owner.open_conversation(None).await.unwrap();
    std::fs::write(root.join("notes.txt"), "dirty bytes\n").unwrap();
    let args = json!({"action":"stage","repo":"owned/repo","paths":["notes.txt"]});
    let preview = data(
        request(
            &host,
            "preview",
            w::Command::Files {
                action: "git_preview".into(),
                args: args.clone(),
            },
        )
        .await,
    );
    assert_eq!(git(&root, &["show", ":notes.txt"]), "original\n");
    let confirm = w::Command::Files {
        action: "git_confirm".into(),
        args: json!({"ticket":preview["ticket"]}),
    };
    let rejected = data(request(&sibling, "wrong-owner", confirm.clone()).await);
    assert!(rejected.get("error").is_some());
    let first = request(&host, "apply", confirm.clone()).await;
    let w::BridgeFrame::Reply {
        durable,
        data: result,
        ..
    } = &first
    else {
        panic!()
    };
    assert!(*durable);
    assert_eq!(result["ok"], true);
    assert_eq!(git(&root, &["show", ":notes.txt"]), "dirty bytes\n");
    let replay = request(&host, "apply", confirm.clone()).await;
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    assert!(
        data(request(&host, "new-id-replay", confirm).await)
            .get("error")
            .is_some()
    );
    let preview = data(
        request(
            &host,
            "preview-revoke",
            w::Command::Files {
                action: "git_preview".into(),
                args,
            },
        )
        .await,
    );
    let (_, revision) = host
        .registry
        .selected()
        .unwrap()
        .services
        .tools()
        .folder_access()
        .unwrap();
    request(&host,"revoke",w::Command::Runtime {action:"folder_access".into(),args:json!({"revision":revision,"action":"mode","path":root,"mode":"read_only"})}).await;
    let s = host.commands.snapshot().unwrap();
    let result = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: "revoked".to_string().try_into().unwrap(),
            scope: s.scope,
            expected_revision: s.revision,
            command: w::Command::Files {
                action: "git_confirm".into(),
                args: json!({"ticket":preview["ticket"]}),
            },
            extension: None,
        })
        .await;
    assert!(result.is_err() || result.is_ok_and(|r|data(r).get("error").is_some()));
    assert_eq!(
        std::fs::read_to_string(root.join("notes.txt")).unwrap(),
        "dirty bytes\n"
    );
    host.shutdown().await.unwrap();
    sibling.shutdown().await.unwrap();
    owner.shutdown().await.unwrap();
}
