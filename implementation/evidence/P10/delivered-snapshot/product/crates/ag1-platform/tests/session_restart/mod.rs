use super::*;
use ag1_core::{
    context::{ContextArtifact, ContextArtifactPort},
    tools::executor::Status,
};
use ag1_platform::session::{
    commands::{RunSettings, SessionCommands},
    registry::{PreparedRuntime, Selection},
    restart::ReconcileCall,
};
use ag1_protocol::wire as w;

#[test]
fn crash_child() {
    let Ok(control) = std::env::var("AG1_P10_CRASH_CONTROL") else {
        return;
    };
    struct CrashAfterWrite(Arc<Mutex<Option<std::path::PathBuf>>>);
    impl ag1_storage::FaultInjector for CrashAfterWrite {
        fn check(&self, point: ag1_storage::Boundary) -> ag1_storage::StorageResult<()> {
            if point == ag1_storage::Boundary::BeforeTransaction
                && self
                    .0
                    .lock()
                    .unwrap()
                    .as_ref()
                    .is_some_and(|root| root.join("output.txt").exists())
            {
                std::process::exit(73);
            }
            Ok(())
        }
    }
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap().block_on(async{
        let root=Arc::new(Mutex::new(None));
        let endpoint=std::env::var("AG1_P10_CRASH_ENDPOINT").unwrap();
        let f=fixture_seeded_with_fault(&endpoint,"disabled",vec![],Some(Arc::new(CrashAfterWrite(root.clone()))));
        *root.lock().unwrap()=Some(f.root.clone());
        std::fs::write(control,json!({"directory":f._directory.path(),"workspace":f.workspace.id().as_str(),"model":f.model,"tools":f.host.configuration().unwrap().tools}).to_string()).unwrap();
        let commands=SessionCommands::new(Selection{epoch:1,services:Arc::new(PreparedRuntime::from_shared(f.host.clone(),Arc::new(Resolver::new(false)),f.files.clone()))},"crashed-client".to_owned().try_into().unwrap(),Arc::new(FixedClock),RunSettings::default()).unwrap();
        let request=w::CommandEnvelope{version:Default::default(),id:"write-once".to_owned().try_into().unwrap(),scope:commands.scope().clone(),expected_revision:w::Sequence::new(0),command:w::Command::Prompt{input:w::PromptInput::Inline{text:"Write the file once".into()}},extension:None};
        assert!(matches!(commands.dispatch(request).await.result,w::CommandResult::Accepted{..}));
        commands.run_next(&ignore).await.unwrap();
        panic!("crash boundary was not reached");
    });
}
struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct NoReplay;
impl ag1_platform::session::ToolRunner for NoReplay {
    fn execute(
        &self,
        _: &ag1_core::tools::registry::Scope,
        _: &[ag1_core::runtime::session::PendingCall],
        _: &ag1_platform::session::ToolSelection,
        _: &str,
        _: &AtomicBool,
        _: &std::sync::atomic::AtomicU64,
    ) -> Result<Vec<ag1_platform::session::ToolOutcome>, DomainError> {
        panic!("restart attempted to replay a tool")
    }
}
fn evidence(
    store: &Store,
    workspace: &str,
    identity: (&str, &str),
    operation: &str,
) -> ContextArtifact {
    let mut artifacts = ag1_storage::context::ContextStore::new(
        store,
        SessionId::new("session").unwrap(),
        "session",
        operation,
        NOW,
        Default::default(),
    )
    .unwrap();
    let mut value = json!({"version":1,"workspace":workspace,"session":"session","executionSettled":true,"observations":"The owned host exited with code 73. Inspected output.txt and retained operation intent; effects are uncertain and must not be replayed."});
    value[identity.0] = json!(identity.1);
    artifacts
        .put_evidence(
            &value.to_string(),
            "host-restart-inspection",
            &json!({"type":"system","name":"restart-inspector"}),
            &[],
            None,
        )
        .unwrap()
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_crash_after_file_mutation_preserves_intent_and_requires_inspection_before_continuation()
 {
    let control = tempfile::tempdir().unwrap();
    let manifest = control.path().join("child.json");
    let (endpoint, server) = server(vec![
        calls(vec![(
            "write-call",
            "write",
            json!({"path":"output.txt","content":"written once"}),
        )]),
        stop(),
    ]);
    let executable = std::env::current_exe().unwrap();
    let path = manifest.clone();
    let origin = endpoint.clone();
    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new(executable)
            .args(["--exact", "restart::crash_child", "--nocapture"])
            .env("AG1_P10_CRASH_CONTROL", path)
            .env("AG1_P10_CRASH_ENDPOINT", origin)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    let directory = std::path::PathBuf::from(manifest["directory"].as_str().unwrap());
    let _cleanup = Cleanup(directory.clone());
    assert_eq!(
        status.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("workspace/output.txt")).unwrap(),
        "written once"
    );
    let store = Arc::new(Store::open(&directory.join("store")).unwrap());
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(&directory.join("workspace"))
        .unwrap();
    assert_eq!(workspace.id().as_str(), manifest["workspace"]);
    let saved = store
        .journal_snapshot(&SessionId::new("session").unwrap())
        .unwrap();
    let state = SessionRuntime::resume(
        SessionId::new("session").unwrap(),
        2,
        &saved,
        Configuration {
            model: manifest["model"].clone(),
            thinking: "off".into(),
            tools: manifest["tools"].as_array().unwrap().clone(),
            system_prompt: "Owned native fixture".into(),
            prompt: None,
            compiler: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(state.pending_tools().len(), 1);
    let scheduler = Scheduler::default();
    scheduler.register(&endpoint, EndpointKind::Local).unwrap();
    let host = Arc::new(
        NativeSession::new(
            state,
            store.clone(),
            "session".into(),
            workspace.clone(),
            Arc::new(BlockingPool::new(1, 32).unwrap()),
            scheduler,
            ProviderHttp::new(&[endpoint], Duration::from_secs(1)).unwrap(),
        )
        .unwrap(),
    );
    let page = host.inspect_restart(None, None, 32).await.unwrap();
    assert_eq!(page.pending_calls.len(), 1);
    assert_eq!(page.tool_records.len(), 1);
    assert_eq!(page.interrupted_commands.len(), 1);
    assert_eq!(page.tool_records[0].status, Status::Intent);
    assert!(host.restart_required());
    let resolver = Resolver::new(false);
    assert!(
        host.run(input(), &resolver, Arc::new(NoReplay), &ignore)
            .await
            .is_err()
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    let inspection = evidence(
        &store,
        workspace.id().as_str(),
        ("callId", "different"),
        "wrong-inspection",
    );
    assert!(matches!(
        host.reconcile_interrupted_call(
            ReconcileCall {
                command: "restore".into(),
                call_id: "write-call".into(),
                expected_revision: page.journal_revision,
                timestamp: NOW.into(),
                inspection
            },
            Arc::new(ignore)
        )
        .await,
        Err(DomainError::PermissionDenied)
    ));
    let inspection = evidence(
        &store,
        workspace.id().as_str(),
        ("callId", "write-call"),
        "call-inspection",
    );
    host.reconcile_interrupted_call(
        ReconcileCall {
            command: "restore".into(),
            call_id: "write-call".into(),
            expected_revision: page.journal_revision,
            timestamp: NOW.into(),
            inspection,
        },
        Arc::new(ignore),
    )
    .await
    .unwrap();
    assert!(
        host.run(input(), &resolver, Arc::new(NoReplay), &ignore)
            .await
            .is_err()
    );
    let orphan = &page.interrupted_commands[0];
    let inspection = evidence(
        &store,
        workspace.id().as_str(),
        ("commandKey", &orphan.key),
        "command-inspection",
    );
    host.reconcile_interrupted_command(orphan.key.clone(), inspection, NOW.into())
        .await
        .unwrap();
    assert!(!host.restart_required());
    std::fs::write(directory.join("workspace/output.txt"), "inspected state").unwrap();
    host.run(input(), &resolver, Arc::new(NoReplay), &ignore)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(directory.join("workspace/output.txt")).unwrap(),
        "inspected state"
    );
    assert_eq!(server.join().unwrap().len(), 2);
    let rows = ag1_core::task::branch_values(
        &store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
    )
    .unwrap();
    let results: Vec<_> = rows
        .iter()
        .filter(|e| e["message"]["toolCallId"] == "write-call")
        .collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["message"]["isError"], true);
    assert_eq!(
        results[0]["message"]["details"]["nativeRecovery"]["effect"],
        "unknown"
    );
    assert_eq!(
        store
            .session_tool_records(workspace.id().as_str(), "session", None, 32)
            .unwrap()[0]
            .status,
        Status::Intent
    );
}
