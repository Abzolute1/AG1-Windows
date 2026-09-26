#![cfg(target_os = "linux")]
use ag1_core::{
    DomainError,
    tools::{
        approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
        policy::Policy,
        registry::{Registry, Scope},
    },
};
use ag1_platform::{
    change_receipts::{ChangeReceipt, PendingChange, read_snapshot, turn_calls, verify_branch},
    file_tools::{FileExecution, FileService},
    workspace::{Workspace, WorkspaceService},
};
use ag1_storage::{SessionBootstrap, SessionSeed, Store};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: Arc<Workspace>,
    store: Arc<Store>,
    registry: Registry,
    scope: Scope,
}
impl Fixture {
    fn new() -> Self {
        Self::with_records(&[])
    }
    fn with_records(records: &[Value]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
        let store = Arc::new(Store::open(&temp.path().join("state")).unwrap());
        let raw = records
            .iter()
            .map(|r| serde_json::to_vec(r).unwrap())
            .collect::<Vec<_>>();
        for session in ["one", "two"] {
            let header = serde_json::to_vec(&json!({"type":"session","version":3,"id":session,"cwd":root,"timestamp":"2026-09-10T00:00:00Z"})).unwrap();
            store
                .bootstrap_session(SessionBootstrap {
                    command: &format!("create-{session}"),
                    session,
                    origin: session,
                    cursor_key: &[7; 32],
                    seed: SessionSeed::Fresh {
                        header: &header,
                        records: &raw,
                    },
                })
                .unwrap();
        }
        let mut registry = Registry::builtins().unwrap();
        let names = vec!["write".into()];
        registry
            .activate(&names, &names.iter().cloned().collect())
            .unwrap();
        let scope = Scope {
            workspace: workspace.id().as_str().into(),
            session: "one".into(),
            epoch: 1,
            revision: 1,
        };
        Self {
            _temp: temp,
            workspace,
            store,
            registry,
            scope,
        }
    }
    fn write(&self, id: &str, content: &str, cancel: bool) -> Result<Value, DomainError> {
        let service = FileService::new(self.workspace.clone()).with_reviews(self.store.clone());
        let call = self.registry.prepare(
            id,
            id,
            "write",
            &json!({"path":"notes.txt","content":content}),
            true,
        )?;
        let plan = service.prepare(&call, &self.scope)?;
        let policy: Policy =
            serde_json::from_value(json!({"mode":"interactive","cwd":self.workspace.root()}))
                .unwrap();
        let mut authorizer = Authorizer::new("review-fixture").unwrap();
        let authorization = match authorizer.preflight(
            call,
            PreflightContext {
                scope: &self.scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: plan.target_revision(),
                now: 10,
                ttl: 10000,
                untrusted_extensions: false,
            },
        )? {
            Preflight::Ready(value) => value,
            Preflight::Approval(request) => authorizer
                .answer(
                    &request.binding,
                    true,
                    &self.registry,
                    ExecutionFence {
                        scope: &self.scope,
                        policy_revision: 1,
                        target_revision: plan.target_revision(),
                        now: 11,
                    },
                )?
                .unwrap(),
            Preflight::Rejected(reason) => panic!("{reason}"),
        };
        let outcome = service.execute(
            plan,
            authorization,
            FileExecution {
                registry: &self.registry,
                scope: &self.scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(cancel),
                deadline: Instant::now() + Duration::from_secs(5),
            },
        )?;
        Ok(outcome.result)
    }
    fn receipts(&self, id: &str) -> Vec<ChangeReceipt> {
        self.store
            .native_file_changes("one", &[id.into()])
            .unwrap()
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect()
    }
}

#[test]
fn native_capture_binds_to_immutable_call_entry_and_rejects_forged_or_other_branch_claims() {
    let records = vec![
        json!({"type":"message","id":"user-a","parentId":null,"timestamp":"2026-09-10T00:00:00Z","message":{"role":"user","content":"Edit notes"}}),
        json!({"type":"message","id":"assistant-a","parentId":"user-a","timestamp":"2026-09-10T00:00:01Z","message":{"role":"assistant","api":"openai-completions","provider":"fixture","model":"fixture","stopReason":"toolUse","content":[{"type":"toolCall","id":"edit-a","name":"write","arguments":{"path":"notes.txt","content":"Changed"}}]}}),
    ];
    let f = Fixture::with_records(&records);
    f.write("edit-a", "Changed", false).unwrap();
    let record = f.receipts("edit-a").remove(0);
    assert_eq!(record.call_entry.as_deref(), Some("assistant-a"));
    verify_branch(&record, &records, "one", &f.scope.workspace).unwrap();
    assert_eq!(turn_calls(&records, "user-a").unwrap(), vec!["edit-a"]);
    assert!(turn_calls(&records, "assistant-a").is_err());
    assert!(verify_branch(&record, &records, "two", &f.scope.workspace).is_err());
    assert!(verify_branch(&record, &records, "one", "other-workspace").is_err());
    let mut off_branch = records.clone();
    off_branch[1]["id"] = json!("different-assistant");
    assert!(verify_branch(&record, &off_branch, "one", &f.scope.workspace).is_err());
    let forged = vec![
        records[0].clone(),
        json!({"type":"message","id":"assistant-a","message":{"role":"toolResult","toolCallId":"edit-a","details":{"call_entry":"assistant-a","state":"committed"},"content":[{"type":"toolCall","id":"edit-a","name":"write"}]}}),
    ];
    assert!(verify_branch(&record, &forged, "one", &f.scope.workspace).is_err());
    let mut duplicate = records.clone();
    duplicate.push(records[1].clone());
    assert!(verify_branch(&record, &duplicate, "one", &f.scope.workspace).is_err());
    let mut historical = record.clone();
    historical.call_entry = None;
    assert!(verify_branch(&historical, &records, "one", &f.scope.workspace).is_err());
    let mut next = records.clone();
    next.push(json!({"type":"message","id":"user-b","message":{"role":"user","content":"Next"}}));
    next.push(json!({"type":"message","id":"assistant-b","message":{"role":"assistant","content":[{"type":"toolCall","id":"edit-b","name":"write"}]}}));
    assert_eq!(turn_calls(&next, "user-a").unwrap(), vec!["edit-a"]);
    assert_eq!(turn_calls(&next, "user-b").unwrap(), vec!["edit-b"]);
}

fn undo_fixture(original: Option<&[u8]>) -> (Fixture, Vec<Value>, ChangeReceipt) {
    let records = vec![
        json!({"type":"message","id":"undo-user","parentId":null,"timestamp":"2026-09-10T00:00:00Z","message":{"role":"user","content":"Edit notes"}}),
        json!({"type":"message","id":"undo-assistant","parentId":"undo-user","timestamp":"2026-09-10T00:00:01Z","message":{"role":"assistant","api":"openai-completions","provider":"fixture","model":"fixture","stopReason":"toolUse","content":[{"type":"toolCall","id":"undo-call","name":"write","arguments":{"path":"notes.txt","content":"Model postimage\n"}}]}}),
    ];
    let f = Fixture::with_records(&records);
    if let Some(bytes) = original {
        std::fs::write(f.workspace.root().join("notes.txt"), bytes).unwrap();
    }
    f.write("undo-call", "Model postimage\n", false).unwrap();
    let receipt = f.receipts("undo-call").remove(0);
    (f, records, receipt)
}

#[test]
fn guarded_undo_restores_dirty_binary_originals_and_retains_the_postimage() {
    use ag1_platform::change_undo::execute;
    let original = [0, 255, 128, 13, 10];
    let (f, records, receipt) = undo_fixture(Some(&original));
    let outcome = execute(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(outcome["state"], "completed");
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        original
    );
    assert_eq!(
        std::fs::read(
            f.workspace
                .root()
                .join(outcome["recovery"].as_str().unwrap())
        )
        .unwrap(),
        b"Model postimage\n"
    );
    assert_eq!(
        f.store
            .native_file_undo("one", &receipt.id)
            .unwrap()
            .unwrap()["state"],
        "completed"
    );
    assert!(
        f.store
            .native_file_undo("two", &receipt.id)
            .unwrap()
            .is_none()
    );
    assert!(
        execute(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |_| Ok(())
        )
        .is_err()
    );
}

#[test]
fn undo_create_is_recoverable_and_later_edits_block_undo_before_any_move() {
    use ag1_platform::change_undo::execute;
    let (f, records, receipt) = undo_fixture(None);
    let outcome = execute(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(outcome["state"], "completed");
    assert!(!f.workspace.root().join("notes.txt").exists());
    assert_eq!(
        std::fs::read(
            f.workspace
                .root()
                .join(outcome["recovery"].as_str().unwrap())
        )
        .unwrap(),
        b"Model postimage\n"
    );
    let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
    std::fs::write(f.workspace.root().join("notes.txt"), b"Later user work\n").unwrap();
    assert!(
        execute(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        b"Later user work\n"
    );
    assert!(
        f.store
            .native_file_undo("one", &receipt.id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn undo_revocation_and_cancellation_preserve_originals_at_each_effect_boundary() {
    use ag1_platform::{change_undo::execute, workspace::UndoStage};
    for stage in [
        UndoStage::BeforeMove,
        UndoStage::AfterMove,
        UndoStage::BeforePublish,
        UndoStage::AfterPublish,
    ] {
        let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
        let result = execute(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |at| {
                if at == stage {
                    Err(DomainError::Cancelled)
                } else {
                    Ok(())
                }
            },
        )
        .unwrap();
        if stage == UndoStage::BeforeMove {
            assert_eq!(result["state"], "not_applied");
            assert_eq!(
                std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
                b"Model postimage\n"
            );
        } else {
            assert_eq!(result["state"], "recovery_required");
            assert_eq!(
                std::fs::read(
                    f.workspace
                        .root()
                        .join(result["recovery"].as_str().unwrap())
                )
                .unwrap(),
                b"Model postimage\n"
            );
            assert_eq!(
                read_snapshot(&f.store, "one", receipt.before.as_ref().unwrap()).unwrap(),
                b"User original\n"
            );
        }
        assert!(
            execute(
                &f.store,
                &f.workspace,
                &receipt,
                &records,
                "one",
                &f.scope.workspace,
                |_| Ok(())
            )
            .is_err()
        );
    }
    let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
    assert!(matches!(
        execute(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |_| Err(DomainError::PermissionDenied)
        ),
        Err(DomainError::PermissionDenied)
    ));
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        b"Model postimage\n"
    );
}

#[test]
fn a_racing_creator_keeps_its_file_and_displaced_bytes_remain_recoverable() {
    use ag1_platform::{change_undo::execute, workspace::UndoStage};
    let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
    let result = execute(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |at| {
            if at == UndoStage::BeforePublish {
                std::fs::write(
                    f.workspace.root().join("notes.txt"),
                    b"Racing user content\n",
                )
                .unwrap();
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result["state"], "recovery_required");
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        b"Racing user content\n"
    );
    assert_eq!(
        std::fs::read(
            f.workspace
                .root()
                .join(result["recovery"].as_str().unwrap())
        )
        .unwrap(),
        b"Model postimage\n"
    );
    assert_eq!(
        read_snapshot(&f.store, "one", receipt.before.as_ref().unwrap()).unwrap(),
        b"User original\n"
    );
}

#[test]
fn interrupted_undo_is_visible_after_storage_restart_and_never_replayed() {
    use ag1_platform::{change_undo::execute, workspace::UndoStage};
    let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |at| {
                if at == UndoStage::AfterMove {
                    panic!("simulated loss before outcome settlement");
                }
                Ok(())
            },
        )
    }));
    assert!(interrupted.is_err());
    let state = f._temp.path().join("state");
    drop(f.store);
    let reopened = Store::open(&state).unwrap();
    let intent = reopened
        .native_file_undo("one", &receipt.id)
        .unwrap()
        .unwrap();
    assert_eq!(intent["state"], "prepared");
    assert_eq!(
        std::fs::read(
            f.workspace
                .root()
                .join(intent["recovery"].as_str().unwrap())
        )
        .unwrap(),
        b"Model postimage\n"
    );
    assert!(
        execute(
            &reopened,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        read_snapshot(&reopened, "one", receipt.before.as_ref().unwrap()).unwrap(),
        b"User original\n"
    );
}

#[test]
fn explicit_recovery_restores_displaced_postimage_without_replaying_undo_or_overwriting_new_work() {
    use ag1_platform::{
        change_undo::{execute, recover},
        workspace::UndoStage,
    };
    let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
    let outcome = execute(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |at| {
            if at == UndoStage::AfterMove {
                Err(DomainError::Cancelled)
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    assert_eq!(outcome["state"], "recovery_required");
    let recovered = recover(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(recovered["state"], "restored_postimage");
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        b"Model postimage\n"
    );
    assert!(
        recover(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |_| Ok(())
        )
        .is_err()
    );
    let (f, records, receipt) = undo_fixture(Some(b"User original\n"));
    execute(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |at| {
            if at == UndoStage::AfterMove {
                Err(DomainError::Cancelled)
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    std::fs::write(f.workspace.root().join("notes.txt"), b"Later file\n").unwrap();
    assert!(
        recover(
            &f.store,
            &f.workspace,
            &receipt,
            &records,
            "one",
            &f.scope.workspace,
            |_| Ok(())
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        b"Later file\n"
    );
}

#[test]
fn undo_rejects_symlinks_and_parent_swaps_without_touching_outside_files() {
    use ag1_platform::{change_undo::execute, workspace::UndoStage};
    let (f, records, receipt) = undo_fixture(Some(b"Original\n"));
    let outside = f._temp.path().join("outside.txt");
    std::fs::write(&outside, b"Outside user work\n").unwrap();
    let original_path = f.workspace.root().join("notes.txt");
    let result = execute(
        &f.store,
        &f.workspace,
        &receipt,
        &records,
        "one",
        &f.scope.workspace,
        |at| {
            if at == UndoStage::BeforeMove {
                std::fs::rename(&original_path, f.workspace.root().join("user-moved.txt")).unwrap();
                std::os::unix::fs::symlink(&outside, &original_path).unwrap();
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result["state"], "not_applied");
    assert_eq!(std::fs::read(outside).unwrap(), b"Outside user work\n");
    assert_eq!(
        std::fs::read(f.workspace.root().join("user-moved.txt")).unwrap(),
        b"Model postimage\n"
    );
}

#[test]
fn turn_output_catalog_excludes_custody_other_calls_and_other_sessions() {
    let f = Fixture::new();
    for (index, kind, call) in [
        (1, "terminal-output", "output-call"),
        (2, "tool-arguments", "output-call"),
        (3, "native-change-bytes", "output-call"),
        (4, "tool-output", "other-call"),
    ] {
        let bytes = b"Retained fixture output";
        let id = format!("art_{index:032x}");
        let record = json!({"version":1,"id":id,"sessionId":"one","kind":kind,"producer":{"type":"tool","name":"fixture","callId":call},"encoding":"utf8","mimeType":"text/plain","sha256":ag1_storage::digest(bytes),"bytes":bytes.len(),"createdAt":"2026-09-10T00:00:00Z"});
        f.store
            .put_artifact(&ag1_storage::ArtifactInput {
                origin: "one".into(),
                session: "one".into(),
                record: serde_json::to_vec(&record).unwrap(),
                bytes: bytes.to_vec(),
            })
            .unwrap();
    }
    let output = f
        .store
        .call_artifact_catalog("one", &["output-call".into()], None, 20)
        .unwrap();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].record["kind"], "terminal-output");
    assert!(
        f.store
            .call_artifact_catalog("two", &["output-call".into()], None, 20)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.store
            .artifact_catalog("one", None, 20, None)
            .unwrap()
            .len(),
        4,
        "diagnostic records remain accessible by deliberate catalog inspection"
    );
}

#[test]
fn actual_writes_retain_user_dirty_bytes_and_session_bound_receipts() {
    let f = Fixture::new();
    let original = b"Uncommitted user edit\r\n";
    std::fs::write(f.workspace.root().join("notes.txt"), original).unwrap();
    f.write("edit-one", "New model content\n", false).unwrap();
    let rows = f.receipts("edit-one");
    assert_eq!(rows.len(), 1);
    let record = &rows[0];
    assert_eq!(record.state, "committed");
    assert_eq!(
        read_snapshot(&f.store, "one", record.before.as_ref().unwrap()).unwrap(),
        original
    );
    assert_eq!(
        read_snapshot(&f.store, "one", &record.after).unwrap(),
        b"New model content\n"
    );
    assert_eq!(
        record.after_revision.as_ref().unwrap().sha256,
        f.workspace
            .snapshot("notes.txt", 1000)
            .unwrap()
            .revision
            .sha256
    );
    assert!(f.store.native_file_change("two", &record.id).is_err());
    assert!(
        f.store
            .native_file_changes("two", &["edit-one".into()])
            .unwrap()
            .is_empty()
    );
    assert!(read_snapshot(&f.store, "two", &record.after).is_err());
    assert!(record.summary.is_some());
}

#[test]
fn creation_noop_and_cancelled_calls_have_truthful_receipts() {
    let f = Fixture::new();
    f.write("create", "New file\n", false).unwrap();
    let record = f.receipts("create").remove(0);
    assert!(record.before.is_none());
    assert_eq!(record.state, "committed");
    f.write("noop", "New file\n", false).unwrap();
    assert!(f.receipts("noop").is_empty());
    assert!(matches!(
        f.write("cancelled", "Never written", true),
        Err(DomainError::Cancelled)
    ));
    assert!(f.receipts("cancelled").is_empty());
    assert_eq!(
        std::fs::read(f.workspace.root().join("notes.txt")).unwrap(),
        b"New file\n"
    );
}

#[test]
fn a_prepared_receipt_does_not_claim_success_and_cannot_be_replayed() {
    let f = Fixture::new();
    let call = f
        .registry
        .prepare(
            "interrupted",
            "interrupted",
            "write",
            &json!({"path":"notes.txt","content":"intended"}),
            true,
        )
        .unwrap();
    let pending = PendingChange::prepare(
        &f.store,
        &f.workspace,
        &f.scope,
        &call,
        "notes.txt",
        None,
        b"intended",
        12,
    )
    .unwrap()
    .unwrap();
    assert_eq!(f.receipts("interrupted")[0].state, "prepared");
    assert!(!f.workspace.root().join("notes.txt").exists());
    assert!(matches!(
        PendingChange::prepare(
            &f.store,
            &f.workspace,
            &f.scope,
            &call,
            "notes.txt",
            None,
            b"intended",
            12
        ),
        Err(DomainError::DurabilityUnknown)
    ));
    pending
        .settle(&f.store, &Err(DomainError::DurabilityUnknown))
        .unwrap();
    assert_eq!(f.receipts("interrupted")[0].state, "uncertain");
}

#[test]
fn binary_originals_are_retained_exactly_and_stale_sources_are_rejected() {
    let f = Fixture::new();
    let raw = [0, 255, 128, 10, 13];
    std::fs::write(f.workspace.root().join("notes.txt"), raw).unwrap();
    let before = f.workspace.snapshot("notes.txt", 100).unwrap();
    let call = f
        .registry
        .prepare(
            "binary",
            "binary",
            "write",
            &json!({"path":"notes.txt","content":"after"}),
            true,
        )
        .unwrap();
    let pending = PendingChange::prepare(
        &f.store,
        &f.workspace,
        &f.scope,
        &call,
        "notes.txt",
        Some(&before.revision),
        b"after",
        12,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        read_snapshot(&f.store, "one", pending.record.before.as_ref().unwrap()).unwrap(),
        raw
    );
    assert!(pending.record.summary.is_none());
    std::fs::write(f.workspace.root().join("notes.txt"), b"later user edit").unwrap();
    assert!(matches!(
        PendingChange::prepare(
            &f.store,
            &f.workspace,
            &f.scope,
            &call,
            "notes.txt",
            Some(&before.revision),
            b"after",
            12
        ),
        Err(DomainError::Conflict)
    ));
    pending
        .settle(&f.store, &Err(DomainError::Conflict))
        .unwrap();
    assert_eq!(f.receipts("binary")[0].state, "not_committed");
}
