// This contract drives AG1's Linux worker process and imports Linux-only
// desktop symbols. Native Windows has its own host contract tests instead.
#![cfg(target_os = "linux")]

use ag1_desktop::{Desktop, file_operation};
use ag1_protocol::{BridgeFrame, Command, CommandEnvelope, PromptInput, Sequence};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap()
}
#[test]
fn native_file_save_rejects_conflict_and_preserves_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.txt");
    fs::write(&path, b"\xef\xbb\xbffirst\r\n").unwrap();
    let read =
        file_operation(json!({"workspace":tmp.path(),"action":"read","args":{"path":"a.txt"}}))
            .unwrap();
    fs::write(&path, b"external\n").unwrap();
    let conflict = file_operation(
        json!({"workspace":tmp.path(),"action":"save","args":{"path":"a.txt","revision":read["revision"],"text":"mine\n"}}),
    );
    assert!(conflict.unwrap_err().contains("conflict"));
    assert_eq!(fs::read(&path).unwrap(), b"external\n");
    let escape = file_operation(
        json!({"workspace":tmp.path(),"action":"read","args":{"path":"../outside"}}),
    );
    assert!(escape.is_err());
}
#[test]
fn desktop_private_state_has_one_writer() {
    let dir = tempfile::tempdir().unwrap();
    let a = Desktop::new(root(), dir.path().to_path_buf(), true).unwrap();
    assert!(Desktop::new(root(), dir.path().to_path_buf(), true).is_err());
    drop(a);
    assert!(Desktop::new(root(), dir.path().to_path_buf(), true).is_ok());
}
#[test]
fn normal_desktop_opens_and_reopens_existing_project_with_saved_native_config() {
    assert!(std::env::var_os("AG1_NATIVE_CONFIG").is_none());
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    let state = dir.path().join("desktop");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&state).unwrap();
    fs::write(
        workspace.join("README.md"),
        b"Keep existing project files.\n",
    )
    .unwrap();
    let endpoint = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    fs::write(
        state.join("native-host.json"),
        serde_json::to_vec(&json!({
            "models":[{"provider":"fixture","id":"saved-model","api":"openai-completions",
                "baseUrl":format!("http://{}/v1",endpoint.local_addr().unwrap()),
                "contextWindow":32768,"maxTokens":4096,"input":["text"],"reasoning":false}],
            "selected":"saved-model","system_prompt":"Saved native configuration","fixture":true
        }))
        .unwrap(),
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    for _ in 0..2 {
        let desktop = Desktop::new(root(), state.clone(), false).unwrap();
        assert!(desktop.native.selects(&workspace).unwrap());
        let view = runtime
            .block_on(desktop.native.open(&workspace, &state))
            .unwrap();
        assert_eq!(view["snapshot"]["status"], "idle");
        let registry = ag1_platform::cutover::Registry::for_user().unwrap();
        let record = registry.inspect(&workspace).unwrap().unwrap();
        assert_eq!(record.owner, ag1_platform::cutover::Owner::Native);
        assert!(!record.isolated);
        assert!(record.transfer.is_none());
        runtime.block_on(desktop.native.shutdown()).unwrap();
    }
    assert_eq!(
        fs::read(workspace.join("README.md")).unwrap(),
        b"Keep existing project files.\n"
    );
    assert!(
        !state.join("workspaces").exists(),
        "Legacy worker was never started"
    );
}

#[test]
fn normal_desktop_without_config_reports_setup_before_registering_workspace() {
    assert!(std::env::var_os("AG1_NATIVE_CONFIG").is_none());
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    fs::create_dir(&workspace).unwrap();
    let state = dir.path().join("desktop");
    let desktop = Desktop::new(root(), state.clone(), false).unwrap();
    assert!(desktop.native.selects(&workspace).unwrap());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let error = runtime
        .block_on(desktop.native.open(&workspace, &state))
        .unwrap_err();
    assert!(error.contains("native-host.json"), "{error}");
    assert!(
        ag1_platform::cutover::Registry::for_user()
            .unwrap()
            .inspect(&workspace)
            .unwrap()
            .is_none()
    );
}
#[test]
fn real_worker_is_supervised_and_scoped() {
    // Test executable lives in deps; Desktop::open resolves its own host path, so use Worker directly.
    use ag1_desktop::Worker;
    use ag1_platform::process::ProcessSpec;
    use std::collections::BTreeMap;
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    let state = tmp.path().join("state");
    fs::create_dir_all(&ws).unwrap();
    fs::create_dir_all(&state).unwrap();
    fs::write(ws.join("README.md"), "# Native bridge fixture\n").unwrap();
    let root = root();
    let argv = vec![
        root.join("product/compat/agent-worker/worker.ts")
            .display()
            .to_string(),
        root.join("reference/ag1-bridge").display().to_string(),
        state.display().to_string(),
        ws.display().to_string(),
        ag1_protocol::schema_hash().into(),
        root.join("product/target/debug/ag1-desktop")
            .display()
            .to_string(),
        root.join(".local/bridge-runtime.json")
            .display()
            .to_string(),
        "true".into(),
    ];
    let mut worker = Worker::spawn(
        ProcessSpec {
            resident: false,
            executable: root.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
            argv,
            cwd: ws,
            env: BTreeMap::from([
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), tmp.path().join("home").display().to_string()),
                ("AG1_LLAMA_CPP".into(), "0".into()),
            ]),
            timeout_ms: 60000,
            grace_ms: 1000,
        },
        &root.join("product/target/debug/ag1-process-host"),
        &state.join("stderr.log"),
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while worker.view()["snapshot"].is_null() {
        assert!(Instant::now() < until, "{}", worker.view());
        thread::sleep(Duration::from_millis(20));
    }
    let snapshot = worker.view()["snapshot"].clone();
    let envelope = CommandEnvelope {
        version: serde_json::from_value(snapshot["version"].clone()).unwrap(),
        id: "native-test".to_owned().try_into().unwrap(),
        scope: serde_json::from_value(snapshot["scope"].clone()).unwrap(),
        expected_revision: Sequence::new(0),
        command: Command::Prompt {
            input: PromptInput::Inline {
                text: "[tool] Read README.md".into(),
            },
        },
        extension: None,
    };
    assert!(
        matches!(worker.command(envelope).unwrap(),BridgeFrame::Reply{durable:true,status,..} if status=="accepted")
    );
    while worker.view()["snapshot"]["status"] != "idle" {
        assert!(Instant::now() < until, "{}", worker.view());
        thread::sleep(Duration::from_millis(20));
    }
    assert!(worker.view()["snapshot"]["message_count"].as_u64().unwrap() >= 4);
    worker.stop();
    assert!(worker.view()["error"].as_str().unwrap().contains("stopped"));
}

#[test]
fn theme_plugins_and_trash_survive_discovery_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = Desktop::new(root(), dir.path().to_path_buf(), false).unwrap();
    let mut themes: Value =
        serde_json::from_str(include_str!("../../src/builtin-themes.json")).unwrap();
    themes[0]["id"] = json!("personal-graphite");
    let exported = desktop.export_theme(themes[0].clone()).unwrap();
    assert!(std::path::Path::new(&exported).exists());
    assert_ne!(desktop.export_theme(themes[0].clone()).unwrap(), exported);
    desktop.import_theme(themes[0].clone()).unwrap();
    assert!(desktop.import_theme(themes[0].clone()).is_err());
    desktop.set_theme("personal-graphite").unwrap();
    assert!(desktop.set_theme("missing-plugin").is_err());
    desktop.save_chat(json!({"session":"chat","workspace":"/fixture","title":"Renamed","renamed":true,"deleted":true})).unwrap();
    desktop.save_chat(json!({"session":"chat","workspace":"/fixture","title":"Discovered old title","discovered":true,"deleted":false})).unwrap();
    let catalog = desktop.catalog().unwrap();
    assert_eq!(catalog["chats"][0]["deleted"], true);
    assert_eq!(catalog["chats"][0]["title"], "Renamed");
    drop(desktop);
    let desktop = Desktop::new(root(), dir.path().to_path_buf(), false).unwrap();
    assert_eq!(desktop.catalog().unwrap()["theme"], "personal-graphite");
    assert_eq!(desktop.catalog().unwrap()["chats"][0]["deleted"], true);
    desktop.remove_theme("personal-graphite").unwrap();
    assert_eq!(desktop.catalog().unwrap()["theme"], "dark");
}

#[test]
fn empty_discovered_sessions_are_not_saved_as_chats() {
    let dir = tempfile::tempdir().unwrap();
    let desktop = Desktop::new(root(), dir.path().to_path_buf(), false).unwrap();
    for id in ["draft1", "draft2", "draft3"] {
        desktop.save_chat(json!({"session":id,"workspace":"/fixture","title":"New chat","empty":true,"discovered":true})).unwrap();
    }
    assert!(
        desktop.catalog().unwrap()["chats"]
            .as_array()
            .is_none_or(|c| c.is_empty())
    );
    // A real conversation titled New chat must not be mistaken for a draft.
    desktop.save_chat(json!({"session":"real","workspace":"/fixture","title":"New chat","empty":false,"discovered":true})).unwrap();
    assert_eq!(
        desktop.catalog().unwrap()["chats"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
