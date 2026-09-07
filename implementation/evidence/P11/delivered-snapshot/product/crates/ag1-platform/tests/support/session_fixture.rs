// Shared owned session fixture for contract tests and the measurement example.
use ag1_core::{
    Clock, DomainError, PortFuture, SecretBytes, SessionId,
    context::compiler::CompileOptions,
    inference::{EndpointKind, Scheduler},
    runtime::{SessionRuntime, session::Configuration},
    storage::StoragePort,
    tools::{policy::Policy, registry::Registry},
};
use ag1_platform::{
    executor::BlockingPool,
    file_tools::FileService,
    provider::{
        generation::{GenerationLimits, GenerationRequest},
        http::ProviderHttp,
        request_http::{HttpCredential, generation_http_request},
        requests::RequestContext,
    },
    session::{
        NativeSession, ProviderInput, ProviderResolver, ResolvedRequest, SessionEvent, TurnInput,
        files::{FileRunner, RejectApproval},
    },
    workspace::WorkspaceService,
};
use ag1_storage::Store;
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::Notify;
const NOW: &str = "2026-09-06T00:00:00.000Z";
struct FixedClock;
impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        1000
    }
}
struct Resolver {
    calls: AtomicUsize,
    pause: bool,
    pause_index: usize,
    fail_index: Option<usize>,
    output_limit: usize,
    entered: Notify,
    release: Notify,
}
impl Resolver {
    fn new(pause: bool) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            pause,
            pause_index: 0,
            fail_index: None,
            output_limit: 1024 * 1024,
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
}
impl ProviderResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        input: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest> {
        Box::pin(async move {
            let index = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.pause && index == self.pause_index {
                self.entered.notify_one();
                self.release.notified().await;
            }
            if cancel.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if self.fail_index == Some(index) {
                return Err(DomainError::Unavailable(
                    "fixture credential unavailable".into(),
                ));
            }
            let model = &input.configuration.model;
            let mut context = json!({"systemPrompt":input.configuration.system_prompt,"messages":input.projection.messages});
            if !input.configuration.tools.is_empty() {
                context["tools"] = json!(input.configuration.tools);
            }
            let options = input
                .options
                .cloned()
                .unwrap_or_else(|| json!({"maxTokens":4096}));
            let request = generation_http_request(
                &RequestContext {
                    model,
                    context: &context,
                    options: &options,
                    anthropic_oauth: false,
                    google_project: None,
                    request_id: input.operation,
                },
                HttpCredential::ApiKey(&SecretBytes::new(b"owned-fixture".to_vec())),
                self.output_limit,
            )?;
            Ok(ResolvedRequest {
                model: model.clone(),
                endpoint: model["baseUrl"].as_str().unwrap().into(),
                generation: GenerationRequest {
                    request_id: input.operation.into(),
                    tool_names: input
                        .configuration
                        .tools
                        .iter()
                        .map(|tool| tool["name"].as_str().unwrap().into())
                        .collect(),
                    family: serde_json::from_value(model["api"].clone()).unwrap(),
                    request,
                    limits: GenerationLimits {
                        max_output_bytes: self.output_limit,
                        prefill_ms: 0,
                        idle_ms: 0,
                        overall_ms: 0,
                    },
                },
                timestamp_ms: 1000,
            })
        })
    }
}
fn read_request(socket: &mut TcpStream) -> Value {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = vec![];
    let mut buffer = [0; 4096];
    loop {
        let n = socket.read(&mut buffer).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&bytes[..end]);
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|n| n.parse::<usize>().ok())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
        assert!(bytes.len() < 2 * 1024 * 1024);
    }
}
fn server(replies: Vec<Value>) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = vec![];
        let job_id = regex::Regex::new(r"job-[0-9a-f]{24}").unwrap();
        let episode_id = regex::Regex::new(r"ep_[0-9a-f]{12}").unwrap();
        for mut reply in replies {
            let until = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < until, "expected native provider request");
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            requests.push(read_request(&mut socket));
            if let Some(action) = reply["fixtureJobAction"].as_str() {
                let request = requests.last().unwrap();
                let id = job_id
                    .find(&request["messages"].to_string())
                    .unwrap()
                    .as_str()
                    .to_owned();
                reply = calls(vec![("job-log", "job", json!({"action":action,"id":id}))]);
            }
            if reply["fixtureEpisodeShow"] == true {
                let id = episode_id
                    .find(&requests.last().unwrap()["messages"].to_string())
                    .unwrap()
                    .as_str()
                    .to_owned();
                reply = calls(vec![("show", "episode", json!({"action":"show","id":id}))]);
            }
            if let Some(status) = reply["fixtureHttpStatus"].as_u64() {
                let body = reply["body"].to_string();
                write!(socket,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
                continue;
            }
            let body = if reply["fixtureDisconnect"] == true {
                format!(
                    "data: {}\n\n",
                    json!({"choices":[{"delta":{"content":"partial"}}]})
                )
            } else if let Some(frames) = reply["fixtureFrames"].as_array() {
                frames
                    .iter()
                    .map(|frame| format!("data: {frame}\n\n"))
                    .chain(["data: [DONE]\n\n".into()])
                    .collect()
            } else {
                format!("data: {reply}\n\ndata: [DONE]\n\n")
            };
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        }
        requests
    });
    (origin, handle)
}
fn calls(items: Vec<(&str, &str, Value)>) -> Value {
    json!({"choices":[{"delta":{"tool_calls":items.into_iter().enumerate().map(|(index,(id,name,args))|json!({"index":index,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect::<Vec<_>>()},"finish_reason":"tool_calls"}]})
}
fn stop() -> Value {
    json!({"choices":[{"delta":{"content":"Done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":2}})
}
struct Fixture {
    _directory: tempfile::TempDir,
    root: std::path::PathBuf,
    workspace: Arc<ag1_platform::workspace::Workspace>,
    store: Arc<Store>,
    host: Arc<NativeSession>,
    files: Arc<FileRunner>,
    scheduler: Scheduler,
    model: Value,
}
fn fixture(origin: &str, mode: &str) -> Fixture {
    fixture_seeded(origin, mode, vec![])
}
fn fixture_seeded(origin: &str, mode: &str, messages: Vec<Value>) -> Fixture {
    fixture_seeded_with_fault(origin, mode, messages, None)
}
fn fixture_seeded_with_fault(
    origin: &str,
    mode: &str,
    messages: Vec<Value>,
    faults: Option<Arc<dyn ag1_storage::FaultInjector>>,
) -> Fixture {
    fixture_with_imported_memory(origin, mode, messages, faults, None)
}
fn fixture_with_imported_memory(
    origin: &str,
    mode: &str,
    messages: Vec<Value>,
    faults: Option<Arc<dyn ag1_storage::FaultInjector>>,
    imported_memory: Option<&[u8]>,
) -> Fixture {
    fixture_configured(origin, mode, messages, faults, imported_memory, 3)
}
fn fixture_configured(
    origin: &str,
    mode: &str,
    messages: Vec<Value>,
    faults: Option<Arc<dyn ag1_storage::FaultInjector>>,
    imported_memory: Option<&[u8]>,
    workers: usize,
) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let files = Arc::new(FileService::new(workspace.clone()));
    let store = Arc::new(
        match faults {
            Some(faults) => Store::open_with(
                &directory.path().join("store"),
                Arc::new(FixedClock),
                faults,
            ),
            None => Store::open(&directory.path().join("store")),
        }
        .unwrap(),
    );
    if let Some(bytes) = imported_memory {
        let source = directory.path().join("frozen-memory");
        std::fs::create_dir(&source).unwrap();
        let id = workspace.id().as_str();
        std::fs::write(source.join(id), bytes).unwrap();
        store
            .import_frozen(
                &source,
                &ag1_storage::FrozenManifest {
                    version: 1,
                    frozen: true,
                    files: vec![ag1_storage::FrozenFile {
                        path: id.into(),
                        sha256: ag1_storage::digest(bytes),
                        bytes: bytes.len() as u64,
                        kind: ag1_storage::FileKind::Memory,
                        session: None,
                        origin: None,
                    }],
                },
            )
            .unwrap();
    }
    store
        .create_session(
            "session",
            &serde_json::to_vec(
                &json!({"type":"session","version":3,"id":"session","cwd":root,"timestamp":NOW}),
            )
            .unwrap(),
        )
        .unwrap();
    store
        .register_origin("session", "session", &[9; 32])
        .unwrap();
    let mut registry = Registry::builtins().unwrap();
    let names = ["read", "write", "append"].map(str::to_owned);
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let tools = registry
        .public_tools()
        .into_iter()
        .filter(|tool| names.iter().any(|name| tool["name"] == *name))
        .collect();
    let model = json!({"provider":"owned-fixture","id":"initial","api":"openai-completions","baseUrl":origin,"contextWindow":32768,"maxTokens":4096,"input":["text"],"reasoning":false});
    if !messages.is_empty() {
        let records=messages.into_iter().enumerate().map(|(i,message)|serde_json::to_vec(&json!({"type":"message","id":format!("seed-{i}"),"parentId":if i==0{Value::Null}else{json!(format!("seed-{}",i-1))},"timestamp":NOW,"message":message})).unwrap()).collect::<Vec<_>>();
        store.append_batch("session", "seed", 0, &records).unwrap();
    }
    let state = SessionRuntime::resume(
        SessionId::new("session").unwrap(),
        1,
        &store
            .journal_snapshot(&SessionId::new("session").unwrap())
            .unwrap(),
        Configuration {
            model: model.clone(),
            thinking: "off".into(),
            tools,
            system_prompt: "Owned native fixture".into(),
            prompt: None,
            compiler: CompileOptions::default(),
        },
    )
    .unwrap();
    let scheduler = Scheduler::default();
    scheduler.register(origin, EndpointKind::Local).unwrap();
    let host = Arc::new(
        NativeSession::new(
            state,
            store.clone(),
            "session".into(),
            workspace.clone(),
            Arc::new(BlockingPool::new(workers, 32).unwrap()),
            scheduler.clone(),
            ProviderHttp::new(&[origin.into()], Duration::from_secs(1)).unwrap(),
        )
        .unwrap(),
    );
    let runner = Arc::new(FileRunner {
        files,
        adapters: Default::default(),
        registry: Arc::new(registry),
        policy: serde_json::from_value::<Policy>(json!({"mode":mode,"cwd":root})).unwrap(),
        policy_revision: 1,
        approvals: Arc::new(RejectApproval),
        store: store.clone(),
        origin: "session".into(),
        timestamp: NOW.into(),
        clock: Arc::new(FixedClock),
        tool_timeout: Duration::from_secs(5),
    });
    Fixture {
        _directory: directory,
        root,
        workspace,
        store,
        host,
        files: runner,
        scheduler,
        model,
    }
}
fn input() -> TurnInput {
    TurnInput {
        command: "prompt".into(),
        message: json!({"role":"user","content":"Use the file tools to do the owned fixture task","timestamp":0}),
        timestamp: NOW.into(),
        max_generations: Some(8),
        compaction: Default::default(),
        compaction_timeout_ms: 900000,
        retry: Default::default(),
    }
}
fn ignore(_: &SessionEvent) {}
