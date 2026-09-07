//! GUI-independent isolated client composition. No TypeScript parent session.
use crate::{
    executor::BlockingPool,
    file_tools::FileService,
    provider::{
        generation::{GenerationLimits, GenerationRequest},
        http::ProviderHttp,
        request_http::{HttpCredential, generation_http_request},
        requests::RequestContext,
    },
    session::{
        ProviderInput, ProviderResolver, ResolvedRequest, SessionEvent,
        approvals::PendingApprovals,
        commands::RunSettings,
        files::FileRunner,
        registry::{
            RuntimeFactory, RuntimeRegistry, ScopedServices, ServiceFactory, ServiceInput,
            StoredRuntimeFactory, Target,
        },
        runtime_commands::RuntimeCommands,
    },
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::{
    Clock, DomainError, PortFuture, SecretBytes, SessionId,
    context::compiler::CompileOptions,
    inference::{EndpointKind, Scheduler},
    runtime::session::Configuration,
};
use ag1_protocol as w;
use ag1_storage::{SessionBootstrap, SessionSeed, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
pub mod actions;
pub mod credentials;
mod details;
mod effects;
pub mod export;
pub mod extensions;
mod factory;
pub mod imported;
pub mod local_runtime;
mod observations;
pub mod projection;
mod projects;
mod queries;
pub mod resource_actions;
mod resources;
mod terminal;
pub mod tooling;
mod uploads;
pub mod workflow;

type Result<T> = std::result::Result<T, DomainError>;
fn json_error(_: serde_json::Error) -> DomainError {
    DomainError::InvalidInput("client JSON".into())
}
pub struct WallClock;
impl Clock for WallClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}
pub fn nonce() -> Result<String> {
    let mut bytes = [0; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(crate::io_error)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Explicit host configuration, loaded from a private file by CLI/Tauri. Models
/// contain public provider metadata only; secrets are resolved from the named
/// environment variable in the host and never included in views or exports.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub models: Vec<Value>,
    #[serde(default)]
    pub tooling: Option<tooling::Tooling>,
    #[serde(default)]
    pub extensions: Option<extensions::Extensions>,
    #[serde(default)]
    pub packages: Option<resource_actions::Packages>,
    #[serde(default)]
    pub max_generations: Option<u64>,
    #[serde(default = "interactive_policy")]
    pub policy_mode: String,
    pub selected: String,
    pub credential_env: Option<String>,
    #[serde(default)]
    pub credentials: std::collections::BTreeMap<String, credentials::Auth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported: Option<imported::Inputs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<local_runtime::Runtime>,
    pub system_prompt: String,
    pub fixture: bool,
}
fn interactive_policy() -> String {
    "interactive".into()
}
struct Resolver {
    models: Vec<Value>,
    credentials: credentials::HostCredentials,
}
impl ProviderResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        input: ProviderInput<'a>,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, ResolvedRequest> {
        Box::pin(async move {
            if cancel.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            let model = &input.configuration.model;
            if !self.models.iter().any(|m| m == model) {
                return Err(DomainError::Conflict);
            }
            if model
                .get("runtimeConflict")
                .is_some_and(|v| v != false && !v.is_null())
            {
                return Err(DomainError::Unavailable(
                    "Model runtime configuration conflict; resolve the pinned runtime explicitly"
                        .into(),
                ));
            }
            let credential = self.credentials.resolve_async(model).await?;
            let mut context = json!({"systemPrompt":input.configuration.system_prompt,"messages":input.projection.messages});
            if !input.configuration.tools.is_empty() {
                context["tools"] = json!(input.configuration.tools);
            }
            let options = input
                .options
                .cloned()
                .unwrap_or_else(|| json!({"maxTokens":model["maxTokens"]}));
            let request = generation_http_request(
                &RequestContext {
                    model,
                    context: &context,
                    options: &options,
                    anthropic_oauth: self.credentials.anthropic_oauth(model)
                        || (model["provider"] == "anthropic"
                            && matches!(
                                &credential,
                                crate::extension_models::Credential::Bearer(_)
                            )),
                    google_project: None,
                    request_id: input.operation,
                },
                credentials::borrowed(&credential),
                4 * 1024 * 1024,
            )?;
            Ok(ResolvedRequest {
                model: model.clone(),
                endpoint: crate::provider::http::endpoint_origin(
                    model["baseUrl"]
                        .as_str()
                        .ok_or_else(|| DomainError::InvalidInput("model baseUrl".into()))?,
                )?,
                generation: GenerationRequest {
                    request_id: input.operation.into(),
                    tool_names: input
                        .configuration
                        .tools
                        .iter()
                        .filter_map(|t| t["name"].as_str().map(str::to_owned))
                        .collect(),
                    family: serde_json::from_value(model["api"].clone())
                        .map_err(|_| DomainError::InvalidInput("provider API".into()))?,
                    request,
                    limits: GenerationLimits {
                        max_output_bytes: 4 * 1024 * 1024,
                        prefill_ms: 0,
                        idle_ms: 0,
                        overall_ms: 0,
                    },
                },
                timestamp_ms: WallClock.now_ms(),
            })
        })
    }
}
struct Services {
    imported_settings: imported::Settings,
    workspace: Arc<Workspace>,
    store: Arc<Store>,
    config: Config,
    provider: Arc<Resolver>,
    clock: Arc<dyn Clock>,
    state: PathBuf,
    ui: Arc<extensions::Ui>,
    project: Arc<Mutex<Option<crate::projects::Runtime>>>,
    scheduler: Scheduler,
    http: ProviderHttp,
    templates: Mutex<std::collections::BTreeMap<String, Arc<FileRunner>>>,
}
impl ServiceFactory for Services {
    fn prepare<'a>(&'a self, input: ServiceInput<'a>) -> PortFuture<'a, ScopedServices> {
        Box::pin(async move {
            let mut selected = self.config.selected.clone();
            let mut selected_provider = None;
            // StoredRuntimeFactory verifies all restored labels against this
            // catalog. A branch never silently resumes on the prior model.
            let branch = ag1_core::task::branch_values(input.journal)?;
            for value in &branch {
                if value["type"] == "model_change"
                    && let Some(id) = value["modelId"].as_str()
                {
                    selected = id.into();
                    selected_provider = value["provider"].as_str();
                }
            }
            let model = self
                .config
                .models
                .iter()
                .find(|m| {
                    m["id"] == selected && selected_provider.is_none_or(|p| m["provider"] == p)
                })
                .ok_or_else(|| {
                    DomainError::Unavailable(
                        "Selected model is absent from the host catalog".into(),
                    )
                })?
                .clone();
            let mut registry = ag1_core::tools::registry::Registry::builtins()?;
            // Browser-backed project capabilities need their existing schemas
            // before capability-filtered activation. Registration grants no
            // preview, vision or process authority.
            ag1_core::tools::targets::register_visual_tools(&mut registry)?;
            self.store.ensure_output_origin(
                input.target.session.as_str(),
                input.target.session.as_str(),
            )?;
            let mut adapters = tooling::adapters(
                self.workspace.clone(),
                self.store.clone(),
                &self.state,
                input.target.session.as_str(),
                input.epoch,
                self.clock.clone(),
                self.config.tooling.as_ref(),
            )?;
            if self
                .project
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .is_some()
            {
                let check = crate::projects::SharedCheck {
                    runtime: self.project.clone(),
                };
                let port = if let Some(tooling) = &self.config.tooling
                    && let Some(browser) = &tooling.browser
                {
                    crate::projects::ProjectPort::Shared(crate::projects::SharedProject {
                        check,
                        browser: browser.bind(&tooling.process_host, &self.state),
                        store: self.store.clone(),
                        session: input.target.session.as_str().into(),
                        caller: Mutex::new(Default::default()),
                    })
                } else {
                    crate::projects::ProjectPort::SharedCheck(check)
                };
                adapters.project = Some(Arc::new(Mutex::new(port)));
            }
            let resources = resources::load(
                &self.workspace,
                &self.state,
                &self.config.system_prompt,
                self.config.packages.as_ref(),
            )?;
            let (ws, root, session, epoch, revision, config, ui, discovered) = (
                self.workspace.clone(),
                self.state.clone(),
                input.target.session.as_str().to_owned(),
                input.epoch,
                input.journal.revision,
                self.config.clone(),
                self.ui.clone(),
                resources.catalog.extensions.clone(),
            );
            let scheduler = self.scheduler.clone();
            let http = self.http.clone();
            let store = self.store.clone();
            let credentials = self.provider.credentials.clone();
            adapters.extensions = tokio::task::spawn_blocking(move || {
                extensions::load(
                    ws,
                    &root,
                    &session,
                    epoch,
                    revision,
                    &config,
                    &discovered,
                    ui,
                    scheduler,
                    http,
                    store,
                    credentials,
                )
            })
            .await
            .map_err(|_| DomainError::Conflict)??;
            if let Some(extension) = &adapters.extensions {
                extension.install_tools(&mut registry)?;
            }
            let mut names = adapters.available_names();
            names.push("task".into());
            names.push("explore".into());
            registry.activate(&names, &names.iter().cloned().collect())?;
            let mut tools: Vec<_> = registry
                .public_tools()
                .into_iter()
                .filter(|t| names.iter().any(|n| t["name"] == *n))
                .collect();
            if let Some(entry) = branch
                .iter()
                .rev()
                .find(|v| v["customType"] == "ag1_active_tools")
            {
                let active = entry["data"].as_array().ok_or(DomainError::Conflict)?;
                if active.iter().any(|name| !names.iter().any(|n| name == n)) {
                    return Err(DomainError::Unavailable(
                        "A restored tool is not configured on this host".into(),
                    ));
                }
                tools.retain(|t| active.iter().any(|n| n == &t["name"]));
            }
            let prompt = crate::session::NativeSession::client_prompt(
                self.workspace.root(),
                &timestamp(),
                &resources,
                registry.clone(),
            )?;
            let approvals = Arc::new(PendingApprovals::new(self.clock.clone()));
            adapters.resources = Some(Arc::new(resource_actions::Actions::new(
                self.workspace.clone(),
                self.store.clone(),
                &self.state,
                self.config.packages.as_ref(),
                approvals.clone(),
                self.clock.clone(),
                &self.config.policy_mode,
            )?));
            let runner = Arc::new(FileRunner {
                files: Arc::new(FileService::new(self.workspace.clone())),
                adapters,
                registry: Arc::new(registry),
                policy: serde_json::from_value(
                    json!({"mode":self.config.policy_mode,"cwd":self.workspace.root()}),
                )
                .map_err(|_| DomainError::Conflict)?,
                policy_revision: 1,
                approvals,
                store: self.store.clone(),
                origin: input.target.session.as_str().into(),
                timestamp: timestamp(),
                clock: self.clock.clone(),
                tool_timeout: Duration::from_secs(60),
            });
            self.templates
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .insert(input.target.session.as_str().into(), runner.clone());
            Ok(ScopedServices {
                workspace: self.workspace.clone(),
                origin: input.target.session.as_str().into(),
                configuration: Configuration {
                    model,
                    thinking: self
                        .imported_settings
                        .thinking
                        .clone()
                        .unwrap_or_else(|| "off".into()),
                    tools,
                    system_prompt: self.config.system_prompt.clone(),
                    prompt: Some(Arc::new(prompt)),
                    compiler: CompileOptions {
                        context_window: self.imported_settings.context.unwrap_or(0),
                        ..Default::default()
                    },
                },
                provider: self.provider.clone(),
                tools: runner,
            })
        })
    }
}
pub fn timestamp() -> String {
    chrono::DateTime::from_timestamp_millis(WallClock.now_ms() as i64)
        .unwrap()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

pub struct Host {
    local_runtime: Option<Arc<dyn crate::provider::http::GenerationRuntime>>,
    authority: Mutex<crate::cutover::Lease>,
    endpoint_leases: Mutex<Vec<crate::cutover::EndpointLease>>,
    pub commands: Arc<RuntimeCommands>,
    pub registry: Arc<RuntimeRegistry>,
    pub store: Arc<Store>,
    pub workspace: Arc<Workspace>,
    pub config: Config,
    pub state: PathBuf,
    pub(crate) ui: Arc<extensions::Ui>,
    observations: Mutex<observations::Observations>,
    terminal: Mutex<Option<terminal::Terminal>>,
    pub(crate) progress: Mutex<String>,
    pub(crate) failure: Mutex<Option<String>>,
    pub(crate) uploads: Mutex<std::collections::BTreeMap<String, (String, Vec<u8>)>>,
    pub(crate) project: Arc<Mutex<Option<crate::projects::Runtime>>>,
    pub(crate) controls: tokio::sync::Mutex<()>,
    last_view: Mutex<Option<w::ClientView>>,
    wake: tokio::sync::Notify,
    driver: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    stopping: AtomicBool,
}
impl Host {
    /// The marker is created only by init in a new empty workspace. Normal TS
    /// workspaces cannot be opened through this P12 opt-in boundary.
    pub fn init(path: &Path) -> Result<()> {
        fs::create_dir(path).map_err(crate::io_error)?;
        crate::cutover::Registry::for_user()?.initialize_isolated(path)?;
        fs::write(
            path.join(".ag1-native-isolated"),
            b"P12 isolated workspace\n",
        )
        .map_err(crate::io_error)
    }
    pub async fn open(workspace: &Path, state: &Path, config: Config) -> Result<Arc<Self>> {
        Self::open_with_registry(
            workspace,
            state,
            config,
            &crate::cutover::Registry::for_user()?,
        )
        .await
    }
    /// Explicit registry injection supports disposable cutover rehearsals. All
    /// production clients use the OS-user registry through open.
    pub async fn open_with_registry(
        workspace: &Path,
        state: &Path,
        config: Config,
        authority_registry: &crate::cutover::Registry,
    ) -> Result<Arc<Self>> {
        Self::open_composed(workspace, state, config, authority_registry, None).await
    }
    /// Fixture composition exercises the same generation/drain path with an
    /// injected runtime controller. Production cannot select this constructor.
    pub async fn open_rehearsal_with_runtime(
        workspace: &Path,
        state: &Path,
        config: Config,
        authority_registry: &crate::cutover::Registry,
        runtime: Arc<dyn crate::provider::http::GenerationRuntime>,
    ) -> Result<Arc<Self>> {
        if !config.fixture || config.runtime.is_some() {
            return Err(DomainError::PermissionDenied);
        }
        Self::open_composed(workspace, state, config, authority_registry, Some(runtime)).await
    }
    async fn open_composed(
        workspace: &Path,
        state: &Path,
        config: Config,
        authority_registry: &crate::cutover::Registry,
        injected: Option<Arc<dyn crate::provider::http::GenerationRuntime>>,
    ) -> Result<Arc<Self>> {
        let workspace = WorkspaceService::new()?.register(workspace)?;
        fs::create_dir_all(state).map_err(crate::io_error)?;
        let state = fs::canonicalize(state).map_err(crate::io_error)?;
        if state.starts_with(workspace.root()) || workspace.root().starts_with(&state) {
            return Err(DomainError::PermissionDenied);
        }
        let authority = authority_registry.open_native(workspace.root(), &state)?;
        let imported_session = authority
            .record()?
            .transfer
            .as_ref()
            .map(|t| t.session.clone());
        if let Some(transfer) = authority.record()?.transfer.as_ref() {
            let raw = imported::Input {
                path: state.join("host.json"),
                sha256: transfer.config_sha256.clone(),
            }
            .read()?;
            let bound: Config = serde_json::from_slice(&raw).map_err(json_error)?;
            if ag1_storage::digest(&raw) != transfer.config_sha256
                || serde_json::to_value(&config).map_err(json_error)?
                    != serde_json::to_value(bound).map_err(json_error)?
            {
                return Err(DomainError::Conflict);
            }
        }
        if config.models.is_empty() || config.models.len() > 1024 {
            return Err(DomainError::InvalidInput("model catalog".into()));
        }
        let mut identities = std::collections::BTreeSet::new();
        for model in &config.models {
            if !identities.insert((model["provider"].as_str(), model["id"].as_str())) {
                return Err(DomainError::InvalidInput("Duplicate model identity".into()));
            }
        }
        if config
            .models
            .iter()
            .filter(|m| m["id"] == config.selected)
            .count()
            != 1
        {
            return Err(DomainError::InvalidInput(
                "Initial selected model must identify exactly one catalog entry".into(),
            ));
        }
        let origins = config
            .models
            .iter()
            .map(|m| {
                crate::provider::http::endpoint_origin(
                    m["baseUrl"]
                        .as_str()
                        .ok_or_else(|| DomainError::InvalidInput("baseUrl".into()))?,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let scheduler = Scheduler::default();
        let endpoint_leases = authority_registry.endpoint_leases(&origins)?;
        if !config.fixture && !endpoint_leases.is_empty() && config.runtime.is_none() {
            return Err(DomainError::Unavailable("Local runtime ownership has not been transferred; an endpoint probe is not an ownership grant".into()));
        }
        for origin in &origins {
            let url = reqwest::Url::parse(origin)
                .map_err(|_| DomainError::InvalidInput("endpoint origin".into()))?;
            let local = config.fixture
                || url.host_str().is_some_and(|host| {
                    host == "localhost"
                        || host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|ip| ip.is_loopback())
                })
                || config.models.iter().any(|m| {
                    m.get("runtime").is_some_and(|v| !v.is_null())
                        && crate::provider::http::endpoint_origin(
                            m["baseUrl"].as_str().unwrap_or(""),
                        )
                        .as_ref()
                            == Ok(origin)
                });
            scheduler.register(
                origin,
                if local {
                    EndpointKind::Local
                } else {
                    EndpointKind::Remote
                },
            )?;
        }
        let local_runtime = match &config.runtime {
            Some(runtime) => Some(runtime.open(&config, &state, authority_registry).await?),
            None => injected,
        };
        let imported_settings = imported::Settings::load(&config, &state)?;
        let store = Arc::new(Store::open(&state.join("store"))?);
        let previous = store.runtime_selection(workspace.id().as_str())?;
        let session = previous
            .as_ref()
            .map(|p| p.selection.session.clone())
            .unwrap_or_else(|| imported_session.clone().unwrap_or_else(|| "initial".into()));
        if previous.is_none() && imported_session.is_none() {
            let key = nonce()?;
            store.bootstrap_session(SessionBootstrap {command:"client-bootstrap",session:&session,origin:&session,cursor_key:key.as_bytes(),seed:SessionSeed::Fresh {header:&serde_json::to_vec(&json!({"type":"session","version":3,"id":session,"cwd":workspace.root(),"timestamp":timestamp()})).map_err(json_error)?,records:&[]}})?;
        }
        let clock: Arc<dyn Clock> = Arc::new(WallClock);
        let provider = Arc::new(Resolver {
            models: config.models.clone(),
            credentials: credentials::HostCredentials {
                providers: config.credentials.clone(),
                fallback: config.credential_env.clone(),
                vault: imported::Vault::load(&config, &state).await?,
            },
        });
        let ui = Arc::new(extensions::Ui::default());
        let project = Arc::new(Mutex::new(projects::load(
            &workspace,
            &state,
            config.tooling.as_ref(),
        )?));
        let mut http = ProviderHttp::new(&origins, Duration::from_secs(10))?;
        if let Some(runtime) = &local_runtime {
            http = http.with_runtime(runtime.clone());
        }
        let services = Arc::new(Services {
            imported_settings: imported_settings.clone(),
            scheduler: scheduler.clone(),
            http: http.clone(),
            ui: ui.clone(),
            project: project.clone(),
            workspace: workspace.clone(),
            store: store.clone(),
            config: config.clone(),
            provider,
            clock: clock.clone(),
            state: state.clone(),
            templates: Mutex::new(Default::default()),
        });
        let factory = Arc::new(factory::ClientFactory {
            inner: StoredRuntimeFactory {
                store: store.clone(),
                pool: Arc::new(BlockingPool::new(4, 128)?),
                scheduler,
                http,
                services: services.clone(),
            },
            services,
        });
        let epoch = previous.as_ref().map_or(1, |p| p.selection.epoch + 1);
        let prepared = factory
            .prepare(
                &Target {
                    workspace: workspace.id().as_str().into(),
                    session: SessionId::new(session)?,
                },
                epoch,
            )
            .await?;
        let registry = Arc::new(RuntimeRegistry::durable(
            prepared,
            previous,
            format!("client-open-{}", nonce()?),
        )?);
        registry.reconcile_selection().await?;
        let commands = RuntimeCommands::new(
            registry.clone(),
            factory,
            nonce()?.try_into().map_err(DomainError::InvalidInput)?,
            clock,
            RunSettings {
                max_generations: config.max_generations,
                models: config.models.clone(),
                compaction: imported_settings.compaction,
                retry: imported_settings.retry,
                steering_mode: imported_settings.steering,
                follow_up_mode: imported_settings.follow_up,
                ..Default::default()
            },
            tokio::runtime::Handle::current(),
        )?;
        commands.initialize().await?;
        let host = Arc::new(Self {
            local_runtime,
            endpoint_leases: Mutex::new(endpoint_leases),
            authority: Mutex::new(authority),
            ui,
            uploads: Mutex::new(Default::default()),
            project,
            commands,
            registry,
            store,
            workspace,
            config,
            state,
            observations: Mutex::new(Default::default()),
            terminal: Mutex::new(None),
            progress: Mutex::new(String::new()),
            failure: Mutex::new(None),
            controls: tokio::sync::Mutex::new(()),
            last_view: Mutex::new(None),
            wake: tokio::sync::Notify::new(),
            driver: tokio::sync::Mutex::new(None),
            stopping: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&host);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let Some(owner) = weak.upgrade() else {
                    break;
                };
                if owner.stopping.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(_control) = owner.controls.try_lock()
                    && let Ok(mut slot) = owner.project.try_lock()
                    && let Some(runtime) = slot.as_mut()
                {
                    let _ = projects::poll(
                        runtime,
                        &crate::resource_fs::ScanControl {
                            cancel: &AtomicBool::new(false),
                            deadline: std::time::Instant::now() + Duration::from_millis(80),
                        },
                    );
                }
            }
        });
        let owner = host.clone();
        *host.driver.lock().await = Some(tokio::spawn(async move {
            owner.drive().await;
        }));
        let weak = Arc::downgrade(&host);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                let Some(owner) = weak.upgrade() else { break };
                if owner.stopping.load(Ordering::Acquire) {
                    break;
                }
                let _ = owner.poll_terminal();
            }
        });
        Ok(host)
    }
    async fn drive(self: Arc<Self>) {
        loop {
            self.wake.notified().await;
            if self.stopping.load(Ordering::Acquire) {
                break;
            }
            loop {
                if self.commands.client_transition_pending().unwrap_or(false) {
                    if self.commands.recovery_required().unwrap_or(true) {
                        *self.failure.lock().unwrap() =
                            Some("Session selection requires recovery".into());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                let started = WallClock.now_ms();
                let scope = self.commands.snapshot().map(|s| s.scope);
                let observed = AtomicBool::new(false);
                let listener = |event: &SessionEvent| {
                    let mut observations = self.observations.lock().unwrap();
                    if !observed.swap(true, Ordering::AcqRel)
                        && let Ok(scope) = &scope
                    {
                        observations.begin(scope.clone(), started);
                    }
                    if let Some(text) = observations.event(event) {
                        let mut p = self.progress.lock().unwrap();
                        p.push_str(&text);
                        while p.len() > 16384 {
                            let end = p.char_indices().nth(1024).map_or(p.len(), |(i, _)| i);
                            p.drain(..end);
                        }
                    }
                };
                match self.commands.run_next(&listener).await {
                    Ok(Some(_)) => {
                        self.progress.lock().unwrap().clear();
                    }
                    Ok(None) => break,
                    Err(error) => {
                        *self.failure.lock().unwrap() = Some(format!("{error:?}"));
                        break;
                    }
                }
            }
        }
    }
    pub async fn dispatch(
        self: &Arc<Self>,
        mut envelope: w::CommandEnvelope,
    ) -> Result<w::BridgeFrame> {
        self.authority
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .admit()?;
        envelope
            .version
            .negotiate()
            .map_err(|_| DomainError::InvalidInput("protocol version".into()))?;
        w::encode(&w::WireMessage::Command(envelope.clone()))
            .map_err(|_| DomainError::LimitExceeded("command envelope".into()))?;
        if let w::Command::Runtime { action, args } = &envelope.command
            && action == "client_action"
        {
            envelope.command = self.resource_action(
                args["text"]
                    .as_str()
                    .ok_or_else(|| DomainError::InvalidInput("client action text".into()))?,
            )?;
        }
        if let w::Command::Runtime { action, args } = &envelope.command
            && action == "workflow"
        {
            envelope.command = self.workflow(args)?;
        }
        let _guard = self.controls.lock().await;
        self.authority
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .admit()?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(DomainError::Unavailable("Client host is stopping".into()));
        }
        let snapshot = self.commands.snapshot()?;
        if envelope.scope != snapshot.scope {
            return Err(DomainError::Conflict);
        }
        if matches!(&envelope.command,w::Command::Files {action,..} if action=="save")
            && envelope.expected_revision != snapshot.revision
        {
            return Err(DomainError::Conflict);
        }
        if let w::Command::Runtime { action, args } = &envelope.command {
            if action == "recover_acknowledgement" {
                if self.commands.client_transition_pending()? {
                    self.commands.reconcile().await?;
                } else {
                    self.commands
                        .reconcile_terminal(
                            args["command"]
                                .as_str()
                                .ok_or(DomainError::NotFound)?
                                .to_owned()
                                .try_into()
                                .map_err(DomainError::InvalidInput)?,
                        )
                        .await?;
                }
                *self.failure.lock().map_err(|_| DomainError::Conflict)? = None;
                self.wake.notify_one();
                return Ok(w::BridgeFrame::Reply {
                    id: envelope.id,
                    scope: snapshot.scope,
                    revision: snapshot.revision,
                    durable: true,
                    status: "reconciled".into(),
                    data: json!({"reconciled":true,"replayed":false}),
                });
            }
            if action == "inspect_recovery" {
                let inspection = self
                    .registry
                    .selected()?
                    .services
                    .runtime()
                    .inspect_restart(
                        args["after_tool"].as_str().map(str::to_owned),
                        args["after_command"].as_str().map(str::to_owned),
                        10,
                    )
                    .await?;
                let data = serde_json::to_value(inspection).map_err(json_error)?;
                if data.to_string().len() > 48000 {
                    return Err(DomainError::LimitExceeded(
                        "Recovery page; inspect retained artifacts individually".into(),
                    ));
                }
                return Ok(w::BridgeFrame::Reply {
                    id: envelope.id,
                    scope: snapshot.scope,
                    revision: snapshot.revision,
                    durable: false,
                    status: "query".into(),
                    data,
                });
            }
        }
        if effects::is_effect(&envelope.command) {
            let owner = self.clone();
            return tokio::task::spawn_blocking(move || {
                if owner.commands.client_transition_pending()? {
                    return Err(DomainError::Conflict);
                }
                owner.client_effect(&envelope)
            })
            .await
            .map_err(|_| DomainError::DurabilityUnknown)?;
        }
        let owner = self.clone();
        let query = envelope.command.clone();
        let data = tokio::task::spawn_blocking(move || {
            if owner.commands.client_transition_pending()? {
                return Err(DomainError::Conflict);
            }
            owner.query(&query)
        })
        .await
        .map_err(|_| DomainError::DurabilityUnknown)??;
        if let Some(data) = data {
            if serde_json::to_vec(&data).map_err(json_error)?.len() > 48000 {
                return Err(DomainError::LimitExceeded(
                    "Client response; use a bounded detail or artifact page".into(),
                ));
            }
            return Ok(w::BridgeFrame::Reply {
                id: envelope.id,
                scope: snapshot.scope,
                revision: snapshot.revision,
                durable: false,
                status: "query".into(),
                data,
            });
        }
        let reply = self.commands.dispatch(envelope.clone()).await;
        let status = if matches!(reply.result, w::CommandResult::Rejected { .. }) {
            "rejected"
        } else {
            "accepted"
        };
        if status == "accepted" {
            self.wake.notify_one();
        }
        // Publication can advance the registry before this acknowledgement is
        // serialized. Preserve the accepted result and its original scope.
        let revision = self
            .commands
            .snapshot()
            .ok()
            .filter(|next| next.scope == reply.scope)
            .map(|next| next.revision)
            .unwrap_or(snapshot.revision);
        Ok(w::BridgeFrame::Reply {
            id: reply.id,
            scope: reply.scope,
            revision,
            durable: status == "accepted",
            status: status.into(),
            data: serde_json::to_value(reply.result).map_err(json_error)?,
        })
    }
    pub async fn reply(self: &Arc<Self>, envelope: w::CommandEnvelope) -> w::BridgeFrame {
        match self.dispatch(envelope.clone()).await {
            Ok(reply) => reply,
            Err(error) => w::BridgeFrame::Reply {
                id: envelope.id,
                scope: envelope.scope,
                revision: envelope.expected_revision,
                durable: false,
                status: "rejected".into(),
                data: json!({"error":format!("{error:?}")}),
            },
        }
    }
    pub fn check_authority(&self) -> Result<()> {
        self.authority
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .admit()
    }
    pub async fn drain_for_cutover(&self) -> Result<()> {
        {
            let _guard = self.controls.lock().await;
            self.authority
                .lock()
                .map_err(|_| DomainError::DurabilityUnknown)?
                .begin_drain()?;
        }
        self.shutdown().await
    }
    pub async fn shutdown(&self) -> Result<()> {
        let _guard = self.controls.lock().await;
        self.terminal_command("stop", &json!({}))?;
        let selected = self.registry.selected()?;
        selected.services.runtime().cancel()?;
        for operation in self.commands.snapshot()?.queued {
            let snapshot = self.commands.snapshot()?;
            self.commands
                .dispatch(w::CommandEnvelope {
                    version: Default::default(),
                    id: format!("quit-{}", nonce()?)
                        .try_into()
                        .map_err(DomainError::InvalidInput)?,
                    scope: snapshot.scope,
                    expected_revision: snapshot.revision,
                    command: w::Command::Cancel { operation },
                    extension: None,
                })
                .await;
        }
        self.wake.notify_one();
        let until = tokio::time::Instant::now() + Duration::from_secs(10);
        while !self.commands.snapshot()?.queued.is_empty()
            || !selected.services.runtime().settled()?
        {
            if tokio::time::Instant::now() >= until {
                return Err(DomainError::TimedOut);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let runtime = selected.services.runtime();
        let snapshot = self.commands.snapshot()?;
        let scope = ag1_core::tools::registry::Scope {
            workspace: snapshot.scope.workspace.as_str().into(),
            session: snapshot.scope.session.as_str().into(),
            epoch: runtime.context_epoch()?,
            revision: runtime.journal_revision()?,
        };
        let tools = selected.services.tools().clone();
        let operation = format!("shutdown-{}", nonce()?);
        tokio::task::spawn_blocking(move || tools.quiesce(&scope, &operation))
            .await
            .map_err(|_| DomainError::DurabilityUnknown)??;
        if let Some(runtime) = self.project.lock().unwrap().as_mut() {
            runtime.stop()?;
        }
        self.stopping.store(true, Ordering::Release);
        self.wake.notify_one();
        if let Some(driver) = self.driver.lock().await.take() {
            driver.await.map_err(|_| DomainError::DurabilityUnknown)?;
        }
        if let Some(runtime) = &self.local_runtime {
            runtime.shutdown().await?;
        }
        self.authority
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .quiesced()?;
        self.endpoint_leases
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .clear();
        Ok(())
    }
}
