use ag1_desktop::{Desktop, file_operation};
use ag1_protocol::{BridgeFrame, Command, CommandEnvelope, PromptInput, Sequence};
use serde_json::json;
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
