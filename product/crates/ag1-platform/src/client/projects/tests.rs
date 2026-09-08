use super::*;
use crate::{
    cutover::Registry,
    projects::{Runtime, Toolchains},
};

async fn fixture() -> (tempfile::TempDir, Arc<Host>) {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let authority = root.path().join("authority");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&authority).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&authority, fs::Permissions::from_mode(0o700)).unwrap();
    let registry = Registry::at(&authority).unwrap();
    registry.initialize_isolated(&workspace).unwrap();
    let config = serde_json::from_value(json!({
        "models":[{"id":"fixture","provider":"fixture","api":"openai-completions",
            "baseUrl":"http://127.0.0.1:9/v1","contextWindow":8192,"maxTokens":1024,
            "reasoning":false,"input":["text"]}],
        "selected":"fixture","credential_env":null,"system_prompt":"No generation",
        "fixture":true
    }))
    .unwrap();
    let host = Host::open_with_registry(&workspace, &root.path().join("state"), config, &registry)
        .await
        .unwrap();
    let project_state = root.path().join("project-state");
    fs::create_dir(&project_state).unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/ag1-process-host");
    let runtime = Runtime::load(
        host.workspace.clone(),
        WorkspaceService::new()
            .unwrap()
            .register(&project_state)
            .unwrap(),
        &helper,
        Toolchains {
            node: None,
            npm: None,
            cargo: None,
            rustc: None,
            godot: None,
            mounts: vec![],
        },
    )
    .unwrap();
    *host.project.lock().unwrap() = Some(runtime);
    (root, host)
}

#[tokio::test]
async fn explicit_preview_control_waits_for_poll_and_never_replays_callback() {
    let (_root, host) = fixture().await;
    let scope = host.commands.snapshot().unwrap().scope;
    let called = AtomicBool::new(false);
    // Reproduce the exact lock order used by the native 100ms project poll.
    // Keep it occupied across a timer to verify that the async caller yields.
    let control = host.controls.lock().await;
    let project = host.project.lock().unwrap();
    let mut action = Box::pin(host.with_project_control(&scope, |_| {
        assert!(!called.swap(true, Ordering::SeqCst));
        Ok("closed")
    }));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut action)
            .await
            .is_err(),
        "An explicit preview action must wait for the in-flight poll, not return QueueFull"
    );
    assert!(!called.load(Ordering::SeqCst));
    drop(project);
    drop(control);
    assert_eq!(action.await.unwrap(), "closed");
    assert!(called.load(Ordering::SeqCst));
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_preview_scope_and_cancelled_wait_cannot_run_an_effect() {
    let (_root, host) = fixture().await;
    let scope = host.commands.snapshot().unwrap().scope;
    let called = AtomicBool::new(false);
    let control = host.controls.lock().await;
    let mut cancelled = Box::pin(host.with_project_control(&scope, |_| {
        called.store(true, Ordering::SeqCst);
        Ok(())
    }));
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut cancelled)
            .await
            .is_err()
    );
    drop(cancelled);
    drop(control);
    let mut stale = scope.clone();
    stale.session = "foreign-session".to_string().try_into().unwrap();
    assert_eq!(
        host.with_project_control(&stale, |_| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await,
        Err(DomainError::Conflict)
    );
    assert!(!called.load(Ordering::SeqCst));
    assert!(host.controls.try_lock().is_ok());
    host.shutdown().await.unwrap();
}
