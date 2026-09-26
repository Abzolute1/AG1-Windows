#![cfg(target_os = "linux")]
use ag1_core::tools::registry::{Registry, Scope};
use ag1_platform::{
    change_receipts::{ChangeReceipt, PendingChange, read_snapshot},
    change_undo,
    workspace::{UndoStage, WorkspaceService},
};
use ag1_storage::{SessionBootstrap, SessionSeed, Store};
use serde_json::{Value, json};

fn records() -> Vec<Value> {
    vec![
        json!({"type":"message","id":"crash-user","parentId":null,"timestamp":"2026-09-10T00:00:00Z","message":{"role":"user","content":"Edit notes"}}),
        json!({"type":"message","id":"crash-assistant","parentId":"crash-user","timestamp":"2026-09-10T00:00:01Z","message":{"role":"assistant","api":"openai-completions","provider":"fixture","model":"fixture","stopReason":"toolUse","content":[{"type":"toolCall","id":"crash-call","name":"write","arguments":{"path":"notes.txt","content":"Model postimage\n"}}]}}),
    ]
}

#[test]
fn undo_crash_child() {
    let Some(path) = std::env::var_os("AG1_TEST_UNDO_CRASH_DIR") else {
        return;
    };
    let directory = std::path::PathBuf::from(path);
    let project = directory.join("project");
    std::fs::create_dir(&project).unwrap();
    let workspace = WorkspaceService::new().unwrap().register(&project).unwrap();
    let store = Store::open(&directory.join("state")).unwrap();
    let branch = records();
    let raw = branch
        .iter()
        .map(|v| serde_json::to_vec(v).unwrap())
        .collect::<Vec<_>>();
    store.bootstrap_session(SessionBootstrap { command: "create", session: "one", origin: "one", cursor_key: &[7;32], seed: SessionSeed::Fresh {
        header: &serde_json::to_vec(&json!({"type":"session","version":3,"id":"one","cwd":project,"timestamp":"2026-09-10T00:00:00Z"})).unwrap(), records: &raw,
    }}).unwrap();
    std::fs::write(project.join("notes.txt"), b"Dirty user original\r\n").unwrap();
    let before = workspace.snapshot("notes.txt", 1000).unwrap();
    let mut registry = Registry::builtins().unwrap();
    let names = vec!["write".into()];
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let call = registry
        .prepare(
            "crash-operation",
            "crash-call",
            "write",
            &json!({"path":"notes.txt","content":"Model postimage\n"}),
            true,
        )
        .unwrap();
    let scope = Scope {
        workspace: workspace.id().as_str().into(),
        session: "one".into(),
        epoch: 1,
        revision: 1,
    };
    let pending = PendingChange::prepare(
        &store,
        &workspace,
        &scope,
        &call,
        "notes.txt",
        Some(&before.revision),
        b"Model postimage\n",
        12,
    )
    .unwrap()
    .unwrap();
    let id = pending.record.id.clone();
    let published = workspace.replace("notes.txt", &before.revision, b"Model postimage\n");
    pending.settle(&store, &published).unwrap();
    let receipt: ChangeReceipt =
        serde_json::from_value(store.native_file_change("one", &id).unwrap()).unwrap();
    let point = std::env::var("AG1_TEST_UNDO_CRASH_STAGE").unwrap();
    change_undo::execute(
        &store,
        &workspace,
        &receipt,
        &branch,
        "one",
        &scope.workspace,
        |stage| {
            if format!("{stage:?}") == point {
                std::process::exit(73);
            }
            Ok(())
        },
    )
    .unwrap();
    panic!("crash point was not reached");
}

#[test]
fn abrupt_process_exit_preserves_intent_snapshots_and_recovery_at_every_publication_boundary() {
    for point in [
        UndoStage::BeforeMove,
        UndoStage::AfterMove,
        UndoStage::BeforePublish,
        UndoStage::AfterPublish,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "undo_crash_child", "--nocapture"])
            .env("AG1_TEST_UNDO_CRASH_DIR", directory.path())
            .env("AG1_TEST_UNDO_CRASH_STAGE", format!("{point:?}"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "{point:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let project = directory.path().join("project");
        let store = Store::open(&directory.path().join("state")).unwrap();
        let receipt: ChangeReceipt = serde_json::from_value(
            store
                .native_file_changes("one", &["crash-call".into()])
                .unwrap()
                .remove(0),
        )
        .unwrap();
        let intent = store.native_file_undo("one", &receipt.id).unwrap().unwrap();
        assert_eq!(intent["state"], "prepared");
        assert_eq!(
            read_snapshot(&store, "one", receipt.before.as_ref().unwrap()).unwrap(),
            b"Dirty user original\r\n"
        );
        assert_eq!(
            read_snapshot(&store, "one", &receipt.after).unwrap(),
            b"Model postimage\n"
        );
        if point == UndoStage::BeforeMove {
            assert_eq!(
                std::fs::read(project.join("notes.txt")).unwrap(),
                b"Model postimage\n"
            );
        } else {
            assert_eq!(
                std::fs::read(project.join(intent["recovery"].as_str().unwrap())).unwrap(),
                b"Model postimage\n"
            );
            if point == UndoStage::AfterPublish {
                assert_eq!(
                    std::fs::read(project.join("notes.txt")).unwrap(),
                    b"Dirty user original\r\n"
                );
            } else {
                assert!(!project.join("notes.txt").exists());
            }
        }
        let workspace = WorkspaceService::new().unwrap().register(&project).unwrap();
        assert!(
            change_undo::execute(
                &store,
                &workspace,
                &receipt,
                &records(),
                "one",
                &receipt.workspace,
                |_| Ok(())
            )
            .is_err()
        );
        if matches!(point, UndoStage::AfterMove | UndoStage::BeforePublish) {
            let recovered = change_undo::recover(
                &store,
                &workspace,
                &receipt,
                &records(),
                "one",
                &receipt.workspace,
                |_| Ok(()),
            )
            .unwrap();
            assert_eq!(recovered["state"], "restored_postimage");
            assert_eq!(
                std::fs::read(project.join("notes.txt")).unwrap(),
                b"Model postimage\n"
            );
        }
    }
}
