//! Tauri composition host. Rust owns lifecycle/catalog; the SDK owns agent state.
pub mod themes;
use ag1_platform::{
    process::{OwnedProcess, ProcessSpec, ProcessSupervisor},
    workspace::{WorkspaceService, preserve_text},
};
use ag1_protocol::{BridgeFrame, BridgeSnapshot, CommandEnvelope, MAX_ENVELOPE, WireMessage};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Debug) -> String {
    format!("{e:?}")
}

/// Used only by the TS actor: native capabilities do not independently schedule work.
pub fn file_operation(request: Value) -> Result<Value> {
    let root = request["workspace"].as_str().ok_or("missing workspace")?;
    let service = WorkspaceService::new().map_err(err)?;
    let workspace = service.register(Path::new(root)).map_err(err)?;
    let args = &request["args"];
    match request["action"].as_str().ok_or("missing action")? {
        "list" => Ok(json!(workspace.scan(1000).map_err(err)?)),
        "read" | "save" => {
            let path = args["path"].as_str().ok_or("missing path")?;
            let current = workspace.snapshot(path, 32768).map_err(err)?;
            let token = format!("{:x}", Sha256::digest(serde_json::to_vec(&json!({"sha":current.revision.sha256,"dev":current.revision.device,"ino":current.revision.inode,"size":current.revision.size,"mtime":current.revision.modified_ns.to_string(),"ctime":current.revision.changed_ns.to_string(),"mode":current.revision.mode})).map_err(err)?));
            if request["action"] == "save" {
                if args["revision"].as_str() != Some(&token) {
                    return Err("File conflict: disk changed; your unsaved text is retained".into());
                }
                let bytes = preserve_text(&current, args["text"].as_str().ok_or("missing text")?)
                    .map_err(err)?;
                if bytes.len() > 32768 {
                    return Err("Editor save exceeds the 32 KiB bridge limit".into());
                }
                workspace
                    .replace(path, &current.revision, &bytes)
                    .map_err(err)?;
                return file_operation(
                    json!({"workspace":root,"action":"read","args":{"path":path}}),
                );
            }
            let text = String::from_utf8(current.bytes).map_err(err)?;
            Ok(json!({"path":path,"text":text,"revision":token}))
        }
        "diff" => {
            let helper = std::env::current_exe()
                .map_err(err)?
                .with_file_name("ag1-process-host");
            let scratch = std::env::temp_dir().join(format!("ag1-diff-{}", std::process::id()));
            fs::create_dir_all(&scratch).map_err(err)?;
            let git =
                ag1_platform::git::GitService::new(&helper, Path::new("/usr/bin/git"), &scratch)
                    .map_err(err)?;
            let result = git.status_diff(&workspace, None).map_err(err);
            let _ = fs::remove_dir_all(scratch);
            result.map(|view|json!({"view":format!("HEAD {}\n{}\n{}",view.head,String::from_utf8_lossy(&view.status).replace('\0', "\n"),String::from_utf8_lossy(&view.diff))}))
        }
        _ => Err("Unknown file capability".into()),
    }
}

struct Request {
    envelope: CommandEnvelope,
    answer: mpsc::SyncSender<Result<BridgeFrame>>,
}
enum Control {
    Command(Box<Request>),
    Stop(mpsc::SyncSender<()>),
}
#[derive(Default)]
struct View {
    snapshot: Option<BridgeSnapshot>,
    fatal: Option<String>,
    progress: String,
}
pub struct Worker {
    runtime_lease: Option<ag1_platform::cutover::EndpointLease>,
    authority: Option<ag1_platform::cutover::Lease>,
    tx: mpsc::SyncSender<Control>,
    view: Arc<Mutex<View>>,
    thread: Option<thread::JoinHandle<bool>>,
}
impl Worker {
    pub fn spawn(spec: ProcessSpec, helper: &Path, spool: &Path) -> Result<Self> {
        Self::spawn_guarded(spec, helper, spool, None)
    }
    pub fn spawn_authorized(
        spec: ProcessSpec,
        helper: &Path,
        spool: &Path,
        authority: ag1_platform::cutover::Lease,
    ) -> Result<Self> {
        authority.admit().map_err(err)?;
        if authority.record().map_err(err)?.owner != ag1_platform::cutover::Owner::TypeScript {
            return Err("TS worker requires TS execution authority".into());
        }
        Self::spawn_guarded(spec, helper, spool, Some(authority))
    }
    fn spawn_guarded(
        spec: ProcessSpec,
        helper: &Path,
        spool: &Path,
        authority: Option<ag1_platform::cutover::Lease>,
    ) -> Result<Self> {
        let process = ProcessSupervisor::new(helper)
            .map_err(err)?
            .spawn_protocol(spec, spool)
            .map_err(err)?;
        let (tx, rx) = mpsc::sync_channel(256);
        let view = Arc::new(Mutex::new(View::default()));
        let actor_view = view.clone();
        let thread = thread::spawn(move || actor(process, rx, actor_view));
        Ok(Self {
            runtime_lease: None,
            authority,
            tx,
            view,
            thread: Some(thread),
        })
    }
    pub fn command(&self, envelope: CommandEnvelope) -> Result<BridgeFrame> {
        if let Some(lease) = &self.authority {
            lease.admit().map_err(err)?;
        }
        envelope.version.negotiate().map_err(err)?;
        if serde_json::to_vec(&WireMessage::Command(envelope.clone()))
            .map_err(err)?
            .len()
            > MAX_ENVELOPE
        {
            return Err("Envelope exceeds 64 KiB".into());
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx
            .try_send(Control::Command(Box::new(Request {
                envelope,
                answer: tx,
            })))
            .map_err(|_| "Worker queue full or closed")?;
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| {
            "Command acknowledgment timed out; query its ID before retrying".to_owned()
        })?
    }
    pub fn view(&self) -> Value {
        let view = self.view.lock().unwrap();
        json!({"snapshot":view.snapshot,"error":view.fatal,"progress":view.progress})
    }
    pub fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let (tx, rx) = mpsc::sync_channel(1);
            let _ = self.tx.send(Control::Stop(tx));
            let _ = rx.recv_timeout(Duration::from_secs(8));
            let quiescent = thread.join().unwrap_or(false);
            if quiescent {
                self.runtime_lease.take();
            }
            if quiescent && let Some(lease) = self.authority.as_mut() {
                // Failure leaves the marker non-quiescent and blocks transfer.
                let _ = lease.quiesced();
            }
        }
    }
    pub fn drain_for_cutover(&mut self) -> Result<()> {
        self.authority
            .as_mut()
            .ok_or("Worker has no workspace authority lease")?
            .begin_drain()
            .map_err(err)?;
        self.stop();
        if !self
            .authority
            .as_ref()
            .unwrap()
            .record()
            .map_err(err)?
            .quiescent
        {
            return Err("Worker drain requires recovery".into());
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
fn actor(mut process: OwnedProcess, rx: mpsc::Receiver<Control>, view: Arc<Mutex<View>>) -> bool {
    let mut pending: HashMap<String, mpsc::SyncSender<Result<BridgeFrame>>> = HashMap::new();
    let mut incoming = Vec::new();
    let mut outgoing = Vec::new();
    let mut sent = 0;
    let mut stop: Option<(Instant, mpsc::SyncSender<()>)> = None;
    let outcome: Result<()> = (|| {
        loop {
            if outgoing.is_empty() && stop.is_none() {
                match rx.try_recv() {
                    Ok(Control::Command(request)) => {
                        let id = request.envelope.id.as_str().to_owned();
                        if pending.len() >= 256 || pending.contains_key(&id) {
                            let _ = request
                                .answer
                                .send(Err("Command queue full or ID already in flight".into()));
                        } else {
                            outgoing = serde_json::to_vec(&WireMessage::Command(request.envelope))
                                .map_err(err)?;
                            outgoing.push(b'\n');
                            sent = 0;
                            pending.insert(id, request.answer);
                        }
                    }
                    Ok(Control::Stop(answer)) => {
                        process.close_input();
                        stop = Some((Instant::now(), answer));
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }
            if !outgoing.is_empty() {
                match process.write_input(&outgoing[sent..outgoing.len().min(sent + 32768)]) {
                    Ok(n) => {
                        sent += n;
                        if sent == outgoing.len() {
                            outgoing.clear();
                        }
                    }
                    Err(ag1_core::DomainError::Io(message))
                        if message.contains("temporarily unavailable") => {}
                    Err(e) => return Err(err(e)),
                }
            }
            let chunk = process.read_stdout(32768).map_err(err)?;
            incoming.extend(chunk);
            while let Some(end) = incoming.iter().position(|b| *b == b'\n') {
                if end > MAX_ENVELOPE {
                    return Err("Worker frame too large".into());
                }
                let line: Vec<_> = incoming.drain(..=end).collect();
                let message = ag1_protocol::decode(&line).map_err(err)?;
                let WireMessage::Bridge(frame) = message else {
                    return Err("Unexpected worker frame".into());
                };
                match frame {
                    BridgeFrame::Snapshot { snapshot } => {
                        snapshot.version.negotiate().map_err(err)?;
                        view.lock().unwrap().snapshot = Some(snapshot);
                    }
                    BridgeFrame::Reply { ref id, .. } => {
                        if let Some(answer) = pending.remove(id.as_str()) {
                            let _ = answer.send(Ok(frame));
                        }
                    }
                    BridgeFrame::Progress { scope, text, .. } => {
                        let mut v = view.lock().unwrap();
                        if v.snapshot.as_ref().is_some_and(|s| s.scope == scope) {
                            v.progress = text;
                        }
                    }
                    BridgeFrame::Fatal { detail } => return Err(detail),
                }
            }
            if incoming.len() > MAX_ENVELOPE {
                return Err("Unterminated worker frame too large".into());
            }
            if process.try_wait().map_err(err)?.is_some() {
                if stop.is_some() {
                    break;
                }
                return Err(
                    "Worker exited; durable command outcomes require reconciliation on restart"
                        .into(),
                );
            }
            if stop
                .as_ref()
                .is_some_and(|(at, _)| at.elapsed() > Duration::from_secs(5))
            {
                process.stop().map_err(err)?;
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    let detail = outcome.err().unwrap_or_else(|| "Worker stopped".into());
    view.lock().unwrap().fatal = Some(detail.clone());
    for (_, answer) in pending {
        let _ = answer.send(Err(detail.clone()));
    }
    let quiescent = process.stop().is_ok();
    if let Some((_, answer)) = stop {
        let _ = answer.send(());
    }
    quiescent
}

pub struct Desktop {
    pub github: github::GitHub,
    pub transition: tokio::sync::Mutex<()>,
    pub native: native_client::NativeClient,
    pub worker: Mutex<Option<Worker>>,
    pub root: PathBuf,
    pub state: PathBuf,
    pub fixture: bool,
    catalog_guard: Mutex<()>,
    _lock: fs::File,
}
impl Desktop {
    pub fn new(root: PathBuf, state: PathBuf, fixture: bool) -> Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(&state).map_err(err)?;
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).map_err(err)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(state.join("host.lock"))
            .map_err(err)?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| "Desktop state already has a writer")?;
        #[cfg(feature = "visual-test")]
        let github = github_fixture::install(&state)?.unwrap_or_default();
        #[cfg(not(feature = "visual-test"))]
        let github = github::GitHub::default();
        Ok(Self {
            github,
            transition: tokio::sync::Mutex::new(()),
            native: native_client::NativeClient::from_desktop(&state, fixture)?,
            worker: Mutex::new(None),
            root,
            state,
            fixture,
            catalog_guard: Mutex::new(()),
            _lock: lock,
        })
    }
    pub fn open(&self, path: &str) -> Result<Value> {
        let workspace = fs::canonicalize(path).map_err(err)?;
        for protected in [
            PathBuf::from("/home/alex/Projects/Cruzible"),
            self.root.join("vendor"),
            self.root.join("reference"),
            PathBuf::from("/home/alex/Projects/llama-cpp-turboquant"),
            PathBuf::from("/home/alex/Projects/llama-cpp-qwen4exp"),
            PathBuf::from("/home/alex/Projects/llama-cpp-qwen4exp-mtp-b10715"),
        ] {
            if workspace.starts_with(&protected) {
                return Err(
                    "Read-only reference/runtime cannot become an executing workspace".into(),
                );
            }
        }
        let mut slot = self.worker.lock().unwrap();
        if let Some(worker) = slot.as_mut() {
            let value = worker.view();
            if value["snapshot"]["status"] != "idle" && value["error"].is_null() {
                return Err(
                    "Cancel and drain the active session before switching workspaces".into(),
                );
            }
            worker.stop();
        }
        *slot = None;
        let id = format!(
            "{:x}",
            Sha256::digest(workspace.to_string_lossy().as_bytes())
        );
        let registry = ag1_platform::cutover::Registry::for_user().map_err(err)?;
        let recorded = registry.inspect(&workspace).map_err(err)?;
        let private = match recorded.as_ref() {
            Some(record)
                if record.owner == ag1_platform::cutover::Owner::TypeScript
                    && record.rollback.is_some() =>
            {
                record
                    .state
                    .clone()
                    .ok_or("Rollback destination is unavailable")?
            }
            _ => self.state.join("workspaces").join(id),
        };
        fs::create_dir_all(&private).map_err(err)?;
        let runtime_lease = if self.fixture {
            None
        } else {
            Some(registry.runtime_lease().map_err(err)?)
        };
        let authority = registry
            .open_typescript(&workspace, &private)
            .map_err(err)?;
        let executable = self
            .root
            .join(".local/toolchains/node-v24.14.0-linux-x64/bin/node");
        let host = std::env::current_exe().map_err(err)?;
        let helper = host.with_file_name("ag1-process-host");
        let argv = vec![
            self.root
                .join("product/compat/agent-worker/worker.ts")
                .display()
                .to_string(),
            self.root.join("reference/ag1-bridge").display().to_string(),
            private.display().to_string(),
            workspace.display().to_string(),
            ag1_protocol::schema_hash().to_owned(),
            host.display().to_string(),
            self.root
                .join(".local/bridge-runtime.json")
                .display()
                .to_string(),
            self.fixture.to_string(),
        ];
        let env = BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), self.state.join("home").display().to_string()),
            ("AG1_LLAMA_CPP".into(), "0".into()),
            ("LANG".into(), "C.UTF-8".into()),
        ]);
        // Spools are create-new evidence. Preserve each launch, including failed startup logs.
        let logs = private.join("worker-logs");
        fs::create_dir_all(&logs).map_err(err)?;
        let launch = tempfile::Builder::new()
            .prefix("launch-")
            .tempdir_in(logs)
            .map_err(err)?
            .keep();
        let mut worker = Worker::spawn_authorized(
            ProcessSpec {
                resident: false,
                executable,
                argv,
                cwd: workspace.clone(),
                env,
                timeout_ms: 86400000,
                grace_ms: 1000,
            },
            &helper,
            &launch.join("worker.log"),
            authority,
        )?;
        worker.runtime_lease = runtime_lease;
        let deadline = Instant::now() + Duration::from_secs(20);
        while worker.view()["snapshot"].is_null() {
            if !worker.view()["error"].is_null() {
                return Err(worker.view().to_string());
            }
            if Instant::now() > deadline {
                return Err("Worker startup timeout".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let result = worker.view();
        *slot = Some(worker);
        self.add_project(&workspace)?;
        Ok(result)
    }
    fn read_catalog(&self) -> Result<Value> {
        let path = self.state.join("catalog.json");
        if path.exists() {
            serde_json::from_slice(&fs::read(path).map_err(err)?).map_err(err)
        } else {
            Ok(json!({"projects":[],"theme":"dark","chats":[]}))
        }
    }
    pub fn catalog(&self) -> Result<Value> {
        let _guard = self.catalog_guard.lock().unwrap();
        self.read_catalog()
    }
    pub fn save_chat(&self, mut chat: Value) -> Result<()> {
        if chat.to_string().len() > 4096
            || ["session", "workspace", "title"]
                .iter()
                .any(|k| chat[k].as_str().is_none())
        {
            return Err("Invalid chat metadata".into());
        }
        let discovered =
            chat.as_object_mut().and_then(|c| c.remove("discovered")) == Some(json!(true));
        let _guard = self.catalog_guard.lock().map_err(|_| "Catalog lock")?;
        let mut value = self.read_catalog()?;
        if !value["chats"].is_array() {
            value["chats"] = json!([]);
        }
        let chats = value["chats"]
            .as_array_mut()
            .ok_or("Invalid chat catalog")?;
        if let Some(old) = chats
            .iter_mut()
            .find(|c| c["session"] == chat["session"] && c["workspace"] == chat["workspace"])
        {
            if discovered {
                for key in ["archived", "deleted", "renamed"] {
                    chat[key] = old[key].clone();
                }
                if old["renamed"] == true {
                    chat["title"] = old["title"].clone();
                }
            }
            *old = chat;
        } else {
            // Empty native sessions are drafts, not saved conversations.
            if discovered && chat["empty"] == true { return Ok(()); }
            if chats.len() >= 2000 {
                return Err("Chat catalog is full".into());
            }
            chats.push(chat);
        }
        self.write_preferences(value)
    }
    pub fn import_theme(&self, plugin: Value) -> Result<()> {
        let plugin = serde_json::to_value(themes::validate(plugin)?).map_err(err)?;
        let _guard = self.catalog_guard.lock().map_err(|_| "Catalog lock")?;
        let mut value = self.read_catalog()?;
        if !value["themes"].is_array() {
            value["themes"] = json!([]);
        }
        let themes = value["themes"]
            .as_array_mut()
            .ok_or("Invalid theme catalog")?;
        if themes.iter().any(|t| t["id"] == plugin["id"]) {
            return Err("A theme with this ID is already installed. Remove it before importing a replacement.".into());
        }
        if themes.len() >= 64 {
            return Err("Theme library is full".into());
        }
        themes.push(plugin);
        self.write_preferences(value)
    }
    pub fn export_theme(&self, plugin: Value) -> Result<String> {
        use std::io::Write;
        let plugin = themes::validate(plugin)?;
        let directory = self.state.join("theme-exports");
        fs::create_dir_all(&directory).map_err(err)?;
        for index in 0..1000 {
            let name = if index == 0 {
                format!("{}.ag1-theme.json", plugin.id)
            } else {
                format!("{}-{index}.ag1-theme.json", plugin.id)
            };
            let path = directory.join(name);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(&serde_json::to_vec_pretty(&plugin).map_err(err)?)
                        .map_err(err)?;
                    file.sync_all().map_err(err)?;
                    return Ok(path.to_string_lossy().into_owned());
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(err(e)),
            }
        }
        Err("Theme export folder is full".into())
    }
    pub fn remove_theme(&self, id: &str) -> Result<()> {
        let _guard = self.catalog_guard.lock().map_err(|_| "Catalog lock")?;
        let mut value = self.read_catalog()?;
        let themes = value["themes"]
            .as_array_mut()
            .ok_or("No installed themes")?;
        themes.retain(|t| t["id"] != id);
        if value["theme"] == id {
            value["theme"] = json!("dark");
        }
        self.write_preferences(value)
    }
    pub fn set_theme(&self, theme: &str) -> Result<()> {
        let _guard = self.catalog_guard.lock().unwrap();
        let mut value = self.read_catalog()?;
        if !themes::is_builtin(theme)
            && !value["themes"]
                .as_array()
                .is_some_and(|ts| ts.iter().any(|t| t["id"] == theme))
        {
            return Err("Theme is not installed".into());
        }
        value["theme"] = json!(theme);
        self.write_preferences(value)
    }
    pub fn add_project(&self, path: &Path) -> Result<()> {
        let _guard = self.catalog_guard.lock().unwrap();
        let mut value = self.read_catalog()?;
        let projects = value["projects"]
            .as_array_mut()
            .ok_or("Corrupt catalog retained")?;
        let path = json!(path);
        if !projects.contains(&path) {
            projects.push(path);
        }
        self.write_preferences(value)
    }
    fn write_preferences(&self, value: Value) -> Result<()> {
        use std::io::Write;
        let target = self.state.join("catalog.json");
        let tmp = self.state.join("catalog.tmp");
        let mut file = fs::File::create(&tmp).map_err(err)?;
        file.write_all(&serde_json::to_vec(&value).map_err(err)?)
            .map_err(err)?;
        file.sync_all().map_err(err)?;
        fs::rename(tmp, target).map_err(err)?;
        fs::File::open(&self.state)
            .map_err(err)?
            .sync_all()
            .map_err(err)
    }
}

pub mod project_preview;

pub mod github;
#[cfg(feature = "visual-test")]
mod github_fixture;
pub mod native_client;
#[cfg(feature = "visual-test")]
pub mod cancellation_fixture;
