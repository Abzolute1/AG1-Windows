#![cfg(target_os = "linux")]
use ag1_platform::{
    client::{Config, Host},
    cutover::Registry,
};
use ag1_protocol as w;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    os::unix::fs::PermissionsExt,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static SERIAL: AtomicUsize = AtomicUsize::new(1);
async fn query(host: &Arc<Host>, args: Value) -> Result<Value, ag1_core::DomainError> {
    let s = host.commands.snapshot()?;
    let reply = host
        .dispatch(w::CommandEnvelope {
            version: Default::default(),
            id: format!("review-{}", SERIAL.fetch_add(1, Ordering::SeqCst))
                .try_into()
                .unwrap(),
            scope: s.scope,
            expected_revision: s.revision,
            command: w::Command::Files {
                action: "diff".into(),
                args,
            },
            extension: None,
        })
        .await?;
    let w::BridgeFrame::Reply { data, .. } = reply else {
        panic!("query reply")
    };
    Ok(data)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn large_native_git_review_pages_are_immutable_bounded_and_conversation_scoped() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let authority = temp.path().join("authority");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&authority).unwrap();
    std::fs::set_permissions(&authority, std::fs::Permissions::from_mode(0o700)).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("/usr/bin/git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .current_dir(&workspace)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    git(&["init", "--quiet"]);
    std::fs::write(workspace.join("notes.txt"), "Original user baseline\n").unwrap();
    git(&["add", "--", "notes.txt"]);
    git(&["commit", "--quiet", "-m", "Owned baseline"]);
    let changed = "User dirty content in a large workspace diff\n".repeat(2000);
    std::fs::write(workspace.join("notes.txt"), &changed).unwrap();
    let config: Config = serde_json::from_value(json!({"models":[{"provider":"fixture","id":"no-inference","api":"openai-completions","baseUrl":"http://127.0.0.1:1","contextWindow":16000,"maxTokens":512}],"selected":"no-inference","credential_env":null,"system_prompt":"Test","fixture":true,
        "tooling":{"process_host":env!("CARGO_BIN_EXE_ag1-process-host"),"git":"/usr/bin/git","rg":"/usr/bin/rg","fd":"/usr/bin/fd","bubblewrap":"/usr/bin/bwrap","targets":{"node":null,"npm":null,"cargo":null,"rustc":null,"godot":null,"mounts":[]}}})).unwrap();
    let root = Host::open_with_registry(&workspace, &temp.path().join("state"), config, &registry)
        .await
        .unwrap();
    let host = root.open_conversation(None).await.unwrap();
    let sibling = root.open_conversation(None).await.unwrap();
    let first = query(&host, json!({"paged":true})).await.unwrap();
    assert!(first["bytes"].as_u64().unwrap() > 48000);
    assert!(serde_json::to_vec(&first).unwrap().len() < 48000);
    assert!(
        query(
            &sibling,
            json!({"paged":true,"offset":first["next"],"sha256":first["sha256"]})
        )
        .await
        .is_err()
    );
    assert!(
        query(
            &host,
            json!({"paged":true,"offset":first["next"],"sha256":"wrong-snapshot"})
        )
        .await
        .is_err()
    );
    // A concurrent external edit does not mix a later workspace into this snapshot.
    std::fs::write(workspace.join("notes.txt"), "Later user edit preserved\n").unwrap();
    let mut next = first["next"].clone();
    let mut bytes = STANDARD.decode(first["base64"].as_str().unwrap()).unwrap();
    while !next.is_null() {
        let page = query(
            &host,
            json!({"paged":true,"offset":next,"sha256":first["sha256"]}),
        )
        .await
        .unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() < 48000);
        assert_eq!(page["sha256"], first["sha256"]);
        bytes.extend(STANDARD.decode(page["base64"].as_str().unwrap()).unwrap());
        next = page["next"].clone();
    }
    assert_eq!(ag1_storage::digest(&bytes), first["sha256"]);
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("User dirty content in a large workspace diff")
    );
    let refreshed = query(&host, json!({"paged":true})).await.unwrap();
    assert_ne!(refreshed["sha256"], first["sha256"]);
    assert!(
        query(
            &host,
            json!({"paged":true,"offset":"0","sha256":first["sha256"]})
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("notes.txt")).unwrap(),
        "Later user edit preserved\n"
    );
    root.shutdown().await.unwrap();
}
