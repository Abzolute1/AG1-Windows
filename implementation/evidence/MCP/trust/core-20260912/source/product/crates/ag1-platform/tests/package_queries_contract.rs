#![cfg(target_os = "linux")]
use ag1_core::{
    DomainError, PortFuture,
    resources::{
        package_lifecycle::Scope,
        package_queries::{self, Port, Query},
    },
};
use ag1_platform::{
    package_queries::{Evidence, GitQueries, QueryPort},
    resource_fs::{ResourceFs, ScanControl},
    web_http::{WebHttp, WebPolicy},
    workspace::WorkspaceService,
};
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
struct NoGit;
impl GitQueries for NoGit {
    fn capture<'a>(
        &'a self,
        _: &'a str,
        _: Vec<String>,
        _: bool,
        _: Instant,
        _: &'a AtomicBool,
    ) -> PortFuture<'a, String> {
        panic!("no Git capability in registry fixture")
    }
}
#[derive(Clone, Default)]
struct Captures {
    rows: Arc<Mutex<Vec<CaptureRow>>>,
    fail_registry: bool,
}
type CaptureRow = (String, Value, Vec<u8>);

#[test]
fn native_package_queries_acknowledge_p05_artifacts_with_session_acl_and_exact_bytes() {
    use ag1_core::{SessionId, context::features::Features};
    use ag1_platform::package_queries::StoredEvidence;
    use ag1_storage::{Store, context::ContextStore};
    use base64::{Engine, engine::general_purpose::STANDARD};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("package.json"), r#"{"version":"1"}"#).unwrap();
    let fs = ResourceFs::new(vec![
        WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap(),
    ])
    .unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    let now = "2026-09-05T00:00:00.000Z";
    for session in ["session", "other"] {
        store.create_session(session, serde_json::json!({"type":"session","version":3,"id":session,"cwd":dir.path(),"timestamp":now}).to_string().as_bytes()).unwrap();
    }
    store
        .register_origin("origin", "session", &[4; 32])
        .unwrap();
    let artifacts = ContextStore::new(
        &store,
        SessionId::new("session").unwrap(),
        "origin",
        "package-query",
        now,
        Features::default(),
    )
    .unwrap();
    let evidence = StoredEvidence::new(artifacts);
    evidence
        .capture(
            "package_git_stdout",
            serde_json::json!({"fixture":true}),
            &[0, 255, 10],
        )
        .unwrap();
    let (origin, server) = server(r#"{"version":"2"}"#, None);
    let http =
        WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
    let cancel = AtomicBool::new(false);
    let port = QueryPort::new(
        &fs,
        &http,
        &NoGit,
        &evidence,
        &origin,
        ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(2),
        },
    )
    .unwrap();
    let query = Query::new(
        "npm:fixture".into(),
        Scope::Project,
        dir.path().to_str().unwrap().into(),
    )
    .unwrap();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(package_queries::check_all(&port, &[query]))
        .unwrap();
    assert_eq!(result.len(), 1);
    server.join().unwrap();
    let artifacts = evidence.artifacts().unwrap();
    assert_eq!(artifacts.len(), 3);
    let first = store
        .page("session", "origin", &artifacts[0].id, 0, 32768)
        .unwrap();
    let envelope: Value = serde_json::from_slice(&first.bytes).unwrap();
    assert_eq!(
        STANDARD
            .decode(envelope["bytesBase64"].as_str().unwrap())
            .unwrap(),
        [0, 255, 10]
    );
    assert!(
        store
            .page("other", "origin", &artifacts[0].id, 0, 32768)
            .is_err()
    );
}
impl Evidence for Captures {
    fn capture(&self, kind: &str, metadata: Value, bytes: &[u8]) -> Result<(), DomainError> {
        if self.fail_registry && kind == "package_registry" {
            return Err(DomainError::Io("fixture artifact failure".into()));
        }
        self.rows
            .lock()
            .unwrap()
            .push((kind.into(), metadata, bytes.into()));
        Ok(())
    }
}
fn server(
    body: &'static str,
    cancel: Option<Arc<AtomicBool>>,
) -> (String, std::thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => panic!("{e}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).unwrap(), 1);
            request.push(byte[0]);
            assert!(request.len() < 16384);
        }
        if let Some(cancel) = cancel {
            cancel.store(true, Ordering::Release);
            std::thread::sleep(Duration::from_millis(200));
        }
        let _ = write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        String::from_utf8(request).unwrap()
    });
    (origin, task)
}
#[test]
fn native_registry_queries_capture_originals_before_results_and_obey_owner_cancellation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for mode in ["success", "artifact_failure", "cancel"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"version":"1.0.0","unknown":"preserved"}"#,
        )
        .unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        let fs = ResourceFs::new(vec![workspace]).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (origin, server) = server(
            r#"{"version":"2.0.0","unknown":"registry original"}"#,
            (mode == "cancel").then(|| cancel.clone()),
        );
        let http =
            WebHttp::new(WebPolicy::with_private_origins(std::slice::from_ref(&origin)).unwrap());
        let evidence = Captures {
            fail_registry: mode == "artifact_failure",
            ..Captures::default()
        };
        let port = QueryPort::new(
            &fs,
            &http,
            &NoGit,
            &evidence,
            &origin,
            ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(2),
            },
        )
        .unwrap();
        let query = Query::new(
            "npm:@scope/fixture".into(),
            Scope::Project,
            dir.path().to_str().unwrap().into(),
        )
        .unwrap();
        let start = Instant::now();
        let result = runtime.block_on(package_queries::check_all(&port, &[query]));
        assert!(start.elapsed() < Duration::from_secs(1));
        match mode {
            "success" => {
                assert_eq!(result.unwrap().len(), 1);
                let rows = evidence.rows.lock().unwrap();
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0].0, "package_manifest");
                assert_eq!(rows[1].0, "package_registry");
                assert_eq!(
                    serde_json::from_slice::<Value>(&rows[1].2).unwrap()["unknown"],
                    "registry original"
                );
                assert_eq!(
                    rows[1].1["requestedUrl"],
                    format!("{origin}/@scope/fixture/latest")
                );
            }
            "artifact_failure" => assert_eq!(result, Err(DomainError::DurabilityUnknown)),
            "cancel" => assert_eq!(result, Err(DomainError::Cancelled)),
            _ => unreachable!(),
        }
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /@scope/fixture/latest HTTP/1.1\r\n"));
        assert!(!request.to_lowercase().contains("authorization:"));
        assert!(!request.to_lowercase().contains("cookie:"));
    }
}
#[test]
fn native_query_inputs_cannot_select_endpoints_or_arbitrary_git_commands() {
    let dir = tempfile::tempdir().unwrap();
    let fs = ResourceFs::new(vec![
        WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap(),
    ])
    .unwrap();
    let http = WebHttp::new(WebPolicy::default());
    let cancel = AtomicBool::new(false);
    let evidence = Captures::default();
    let control = || ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(1),
    };
    for url in [
        "https://user:secret@example.com",
        "https://example.com/path",
        "https://example.com/?q=1",
        "file:///tmp",
        "https://example.com/#x",
    ] {
        assert!(QueryPort::new(&fs, &http, &NoGit, &evidence, url, control()).is_err());
    }
    let port = QueryPort::new(
        &fs,
        &http,
        &NoGit,
        &evidence,
        "https://registry.npmjs.org",
        control(),
    )
    .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for name in [
            "x?registry=https://example.com",
            "x#fragment",
            "x%2f..",
            "x@1",
            "../escape",
        ] {
            assert!(port.latest_npm(name, 10000).await.is_err());
        }
        assert_eq!(
            port.git(
                dir.path().to_str().unwrap(),
                vec!["fetch".into()],
                true,
                10000
            )
            .await,
            Err(DomainError::PermissionDenied)
        );
        assert_eq!(
            port.git(
                "/unregistered",
                vec!["rev-parse".into(), "HEAD".into()],
                false,
                10000
            )
            .await,
            Err(DomainError::PermissionDenied)
        );
    });
    assert!(evidence.rows.lock().unwrap().is_empty());
}

#[test]
fn git_queries_use_bound_owned_processes_capture_both_streams_and_reject_stale_binaries() {
    use ag1_platform::{
        package_execution::Binary, package_query_git::NativeGit, process::ProcessSupervisor,
    };
    use std::{collections::BTreeMap, path::Path};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for mode in ["success", "failure", "cancel", "flood", "stale"] {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("installed");
        let spools = dir.path().join("spools");
        std::fs::create_dir(&installed).unwrap();
        std::fs::create_dir(&spools).unwrap();
        let binary = dir.path().join("query-fixture");
        std::fs::copy(env!("CARGO_BIN_EXE_ag1-platform-fixture"), &binary).unwrap();
        let workspaces = WorkspaceService::new().unwrap();
        let root = workspaces.register(&installed).unwrap();
        let fs = ResourceFs::new(vec![root.clone()]).unwrap();
        let evidence = Captures::default();
        let mut arguments = vec!["package".into(), "git-query".into()];
        match mode {
            "failure" => arguments.push("fail".into()),
            "cancel" => arguments.push("slow".into()),
            "flood" => arguments.push("flood".into()),
            _ => {}
        }
        let git = NativeGit::new(
            Arc::new(
                ProcessSupervisor::new(Path::new(env!("CARGO_BIN_EXE_ag1-process-host"))).unwrap(),
            ),
            vec![root],
            workspaces.register(&spools).unwrap(),
            Binary {
                executable: binary.clone(),
                arguments,
            },
            BTreeMap::from([(
                "AG1_PACKAGE_FIXTURE_ROOT".into(),
                dir.path().to_str().unwrap().into(),
            )]),
            evidence.clone(),
        )
        .unwrap();
        if mode == "stale" {
            std::fs::write(&binary, "changed executable").unwrap();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = (mode == "cancel").then(|| {
            let started = dir.path().join("started");
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !started.exists() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
                cancel.store(true, Ordering::Release);
            })
        });
        let http = WebHttp::new(WebPolicy::default());
        let port = QueryPort::new(
            &fs,
            &http,
            &git,
            &evidence,
            "https://registry.npmjs.org",
            ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(4),
            },
        )
        .unwrap();
        let query = Query::new(
            "git:github.com/acme/repo".into(),
            Scope::Project,
            installed.to_str().unwrap().into(),
        )
        .unwrap();
        let result = runtime.block_on(package_queries::check_all(&port, &[query]));
        match mode {
            "success" => {
                assert_eq!(result.unwrap().len(), 1);
                let rows = evidence.rows.lock().unwrap();
                assert_eq!(rows.len(), 6);
                for pair in rows.chunks(2) {
                    assert_eq!(pair[0].0, "package_git_stdout");
                    assert_eq!(pair[1].0, "package_git_stderr");
                    assert!(String::from_utf8_lossy(&pair[1].2).contains("fixture diagnostic"));
                }
                assert_eq!(std::fs::read_dir(&spools).unwrap().count(), 0);
            }
            "failure" => {
                assert!(result.unwrap().is_empty());
                assert_eq!(evidence.rows.lock().unwrap().len(), 2);
            }
            "cancel" => assert_eq!(result, Err(DomainError::Cancelled)),
            "flood" => {
                assert!(matches!(result, Err(DomainError::LimitExceeded(_))));
                assert!(
                    std::fs::read_dir(&spools).unwrap().count() > 0
                        || evidence
                            .rows
                            .lock()
                            .unwrap()
                            .iter()
                            .any(|(kind, _, bytes)| kind == "package_git_stdout"
                                && bytes.len() == 65536)
                );
            }
            "stale" => {
                assert_eq!(result, Err(DomainError::Conflict));
                assert!(!dir.path().join("commands.jsonl").exists());
            }
            _ => unreachable!(),
        }
        if let Some(task) = cancel_thread {
            task.join().unwrap();
            let pid = std::fs::read_to_string(dir.path().join("started")).unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Path::new(&format!("/proc/{pid}")).exists() {
                assert!(
                    Instant::now() < deadline,
                    "owned query process survived cancellation"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
