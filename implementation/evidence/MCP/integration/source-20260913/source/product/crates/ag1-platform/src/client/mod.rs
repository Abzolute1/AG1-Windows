//! GUI-independent isolated client composition. No TypeScript parent session.
use crate::{
    file_tools::FileService,
    executor::BlockingPool,
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
pub mod approval_policy;
mod conversations;
pub mod credentials;
mod details;
mod effects;
pub mod export;
pub mod extensions;
mod factory;
mod folders;
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
mod changes;
mod diff_review;
mod change_undo;
mod local_git;
mod uploads;
pub mod workflow;

type Result<T> = std::result::Result<T, DomainError>;
fn json_error(_: serde_json::Error) -> DomainError {
    DomainError::InvalidInput("client JSON".into())
}
const DEFAULT_GENERATION_BYTES: usize = 4 * 1024 * 1024;
const LOCAL_LLAMA_GENERATION_BYTES: usize = 32 * 1024 * 1024;

/// Local llama sessions may legitimately stream long reasoning traces. Keep
/// remote and extension provider budgets conservative, while aligning the
/// owned runtime with the reducer's existing 32 MiB safety ceiling.
fn generation_byte_budget(model: &Value) -> usize {
    if matches!(
        model["provider"].as_str(),
        Some("llamacpp" | "llama.cpp")
    ) {
        LOCAL_LLAMA_GENERATION_BYTES
    } else {
        DEFAULT_GENERATION_BYTES
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_append: Option<String>,
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
            let options = crate::provider::requests::session_options(
                model,
                &input.configuration.thinking,
                input.options,
            )?;
            let generation_bytes = generation_byte_budget(model);
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
                generation_bytes,
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
                        max_output_bytes: generation_bytes,
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
#[cfg(target_os = "linux")]
pub use crate::mcp::service::Context as McpServices;
#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
pub struct McpServices;
struct Services {
    mcp: Option<McpServices>,
    github: Option<Arc<crate::github::service::Service>>,
    working_projects: Mutex<std::collections::BTreeMap<String, Arc<Mutex<Option<crate::projects::Runtime>>>>>,
    folder_access: Mutex<std::collections::BTreeMap<String, Arc<crate::folder_access::FolderAccess>>>,
    approval_policy: Arc<approval_policy::ApprovalPolicy>,
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
            let access = {
                let mut cached = self.folder_access.lock().map_err(|_|DomainError::Conflict)?;
                if !cached.contains_key(input.target.session.as_str()) {
                    cached.insert(input.target.session.as_str().into(), crate::folder_access::FolderAccess::open(self.workspace.clone(), &self.state, input.target.session.as_str())?);
                }
                cached.get(input.target.session.as_str()).cloned().ok_or(DomainError::Conflict)?
            };
            let folders = Arc::new(access.snapshot()?);
            let workspace = folders.primary();
            let project = if workspace.root() == self.workspace.root() { self.project.clone() } else {
                let key = ag1_storage::digest(format!("{}:{}",input.target.session.as_str(),workspace.root().display()).as_bytes());
                let mut projects = self.working_projects.lock().map_err(|_|DomainError::Conflict)?;
                if !projects.contains_key(&key) {
                    let state = self.state.join("chat-projects").join(&key);
                    fs::create_dir_all(&state).map_err(crate::io_error)?;
                    projects.insert(key.clone(), Arc::new(Mutex::new(projects::load(&workspace,&state,self.config.tooling.as_ref())?)));
                }
                projects.get(&key).cloned().ok_or(DomainError::Conflict)?
            };
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
            if self.github.is_some() { crate::github::tool::register(&mut registry)?; }
            self.store.ensure_output_origin(
                input.target.session.as_str(),
                input.target.session.as_str(),
            )?;
            let mut adapters = tooling::adapters(
                workspace.clone(),
                self.store.clone(),
                &self.state,
                input.target.session.as_str(),
                input.epoch,
                self.clock.clone(),
                self.config.tooling.as_ref(),
                Some(&folders),
            )?;
            if self.github.is_some() && let Some(t)=&self.config.tooling {
                adapters.local_git=folders.roots.iter().filter_map(|root| {
                    let build=||->Result<Arc<crate::github::local::LocalGit>> {
                    let writable=folders.resolve(root.root().to_str().ok_or(DomainError::PermissionDenied)?,true).is_ok();
                    crate::github::local::LocalGit::new(root.clone(),&t.process_host,&t.git,&t.bubblewrap,&self.state.join("local-git"))
                        .map(|g|Arc::new(g.with_grant(access.clone(),folders.selection.revision,writable).with_github(self.github.as_ref().unwrap().clone())))
                    };build().ok()
                }).collect();
            }
            adapters.folder_authority = Some(access);
            adapters.github = self.github.clone();
            adapters.folders = Some(folders.clone());
            if project
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .is_some()
            {
                let check = crate::projects::SharedCheck {
                    runtime: project.clone(),
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
            let mut resources = resources::load(
                &workspace,
                &self.state,
                &self.config.system_prompt,
                self.config.prompt_append.as_deref(),
                self.config.packages.as_ref(),
            )?;
            resources.append_system.push(folders.prompt());
            let (ws, root, session, epoch, revision, config, ui, discovered) = (
                workspace.clone(),
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
            #[cfg(target_os = "linux")]
            if let Some(mcp)=&self.mcp && let Some(catalog_scope)=mcp.catalog_scope(&self.workspace,&folders)? {
                let scope=ag1_core::tools::registry::Scope {workspace:workspace.id().as_str().into(),session:input.target.session.as_str().into(),epoch:input.epoch,revision:0};
                let project=matches!(catalog_scope,crate::mcp::catalog::CatalogScope::Workspace(_)).then(||folders.primary());
                let session=mcp.service.session_with_project(catalog_scope,scope,Some(self.approval_policy.clone()),project)?;
                session.discovery().register(&mut registry)?;
                adapters.mcp_discovery=Some(session.discovery().clone());
                adapters.mcp_management=Some(session);
            }
            let mut names = adapters.available_names();
            names.push("task".into());
            names.push("explore".into());
            registry.activate(&names, &names.iter().cloned().collect())?;
            let mut tools: Vec<_> = registry
                .native_tools()
                .into_iter()
                .filter(|t| names.iter().any(|n| t["name"] == *n))
                .collect();
            if let Some(entry) = branch
                .iter()
                .rev()
                .find(|v| v["type"] == "custom" && v["customType"] == "ag1_active_tools")
            {
                let active = entry["data"].as_array().ok_or(DomainError::Conflict)?;
                if active.iter().any(|name| !names.iter().any(|n| name == n)) {
                    return Err(DomainError::Unavailable(
                        "A restored tool is not configured on this host".into(),
                    ));
                }
                // Tool order is part of the saved configuration and provider
                // prompt. Reconstruct it from the branch rather than retaining
                // the host catalog's order, which would fail strict resume.
                tools = active
                    .iter()
                    .map(|name| {
                        tools
                            .iter()
                            .find(|tool| &tool["name"] == name)
                            .cloned()
                            .ok_or(DomainError::Conflict)
                    })
                    .collect::<Result<Vec<_>>>()?;
            } else {
                tools = ag1_core::tools::surface::preferred(tools);
            }
            let prompt = crate::session::NativeSession::client_prompt(
                &workspace,
                &timestamp(),
                &resources,
                registry.clone(),
            )?;
            let approvals = Arc::new(PendingApprovals::new(self.clock.clone()));
            adapters.approval_policy = Some(self.approval_policy.clone());
            adapters.resources = Some(Arc::new(
                resource_actions::Actions::new(
                    workspace.clone(),
                    self.store.clone(),
                    &self.state,
                    self.config.packages.as_ref(),
                    approvals.clone(),
                    self.clock.clone(),
                    &self.config.policy_mode,
                )?
                .with_policy(self.approval_policy.clone()),
            ));
            let runner = Arc::new(FileRunner {
                files: folders.roots.iter().position(|r| Arc::ptr_eq(r,&workspace)).and_then(|i|adapters.folder_files.get(i)).cloned().unwrap_or_else(||Arc::new(FileService::new(workspace.clone()).with_reviews(self.store.clone()))),
                adapters,
                registry: Arc::new(registry),
                policy: serde_json::from_value(
                    json!({"mode":self.config.policy_mode,"cwd":workspace.root()}),
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
                workspace: workspace.clone(),
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
    live_changed: tokio::sync::Notify,
    live_revision: std::sync::atomic::AtomicU64,
    approval_policy: Arc<approval_policy::ApprovalPolicy>,
    local_runtime: Option<Arc<dyn crate::provider::http::GenerationRuntime>>,
    authority: Arc<Mutex<crate::cutover::Lease>>,
    owns_resources: bool,
    owns_runtime: bool,
    closing: AtomicBool,
    workspace_closing: Arc<AtomicBool>,
    runtime_closing: Arc<AtomicBool>,
    shared_hosts: Arc<tokio::sync::Mutex<Vec<std::sync::Weak<Host>>>>,
    services: Arc<Services>,
    conversation_owners:
        Arc<tokio::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<Host>>>>,
    pool: Arc<BlockingPool>,
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
    diff_review: Mutex<Option<diff_review::DiffReview>>,
    local_git_plans:Mutex<std::collections::BTreeMap<String,local_git::Pending>>,
    pub(crate) project: Arc<Mutex<Option<crate::projects::Runtime>>>,
    pub(crate) controls: tokio::sync::Mutex<()>,
    workspace_controls: Arc<tokio::sync::Mutex<()>>,
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
        Self::open_composed(
            workspace,
            state,
            config,
            authority_registry,
            None,
            None,
            None,
            None, None,
        )
        .await
    }
    /// Host-only evaluation observation. This preserves normal authority and
    /// runtime admission; no observer configuration is exposed over client RPC.
    pub async fn open_with_request_observer(
        workspace: &Path,
        state: &Path,
        config: Config,
        authority_registry: &crate::cutover::Registry,
        observer: crate::provider::http::RequestObserver,
    ) -> Result<Arc<Self>> {
        Self::open_composed(
            workspace,
            state,
            config,
            authority_registry,
            None,
            Some(observer),
            None,
            None, None,
        )
        .await
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
        Self::open_composed(
            workspace,
            state,
            config,
            authority_registry,
            Some(runtime),
            None,
            None,
            None, None,
        )
        .await
    }
    /// Reuse an already admitted runtime only for the exact same configured catalog/capture.
    pub async fn open_shared(
        workspace: &Path,
        state: &Path,
        config: Config,
        parent: &Host,
    ) -> Result<Arc<Self>> {
        if config.models != parent.config.models
            || serde_json::to_value(&config.runtime).map_err(json_error)?
                != serde_json::to_value(&parent.config.runtime).map_err(json_error)?
        {
            return Self::open_composed(workspace,state,config,&crate::cutover::Registry::for_user()?,None,None,None,parent.services.github.clone(),None).await;
        }
        parent.check_authority()?;
        Self::open_composed(
            workspace,
            state,
            config,
            &crate::cutover::Registry::for_user()?,
            None,
            None,
            Some(parent),
            parent.services.github.clone(), None,
        )
        .await
    }
    /// Application-owned account capability, never a path supplied by model
    /// arguments or imported project configuration.
    pub async fn open_integrated(workspace:&Path,state:&Path,config:Config,github:Arc<crate::github::service::Service>) -> Result<Arc<Self>> {
        Self::open_composed(workspace,state,config,&crate::cutover::Registry::for_user()?,None,None,None,Some(github),None).await
    }
    /// Desktop supplies explicit account/project MCP context without changing
    /// model configuration or inferring availability from imported metadata.
    #[cfg(target_os = "linux")]
    pub async fn open_with_mcp(workspace:&Path,state:&Path,config:Config,github:Arc<crate::github::service::Service>,mcp:McpServices,parent:Option<&Host>) -> Result<Arc<Self>> {
        let shared=match parent {
            Some(parent) if config.models==parent.config.models && serde_json::to_value(&config.runtime).map_err(json_error)?==serde_json::to_value(&parent.config.runtime).map_err(json_error)? => Some(parent),
            _=>None,
        };
        if let Some(parent)=shared {parent.check_authority()?;}
        Self::open_composed(workspace,state,config,&crate::cutover::Registry::for_user()?,None,None,shared,Some(github),Some(mcp)).await
    }
    /// Owned offline acceptance uses the same composition with an isolated
    /// authority registry. It cannot inject a live model runtime or credentials.
    #[cfg(target_os = "linux")]
    pub async fn open_mcp_rehearsal(
        workspace: &Path, state: &Path, config: Config,
        authority_registry: &crate::cutover::Registry, mcp: McpServices,
    ) -> Result<Arc<Self>> {
        if !config.fixture || config.runtime.is_some() || config.credential_env.is_some() {
            return Err(DomainError::PermissionDenied);
        }
        Self::open_composed(workspace,state,config,authority_registry,None,None,None,None,Some(mcp)).await
    }
    async fn open_composed(
        workspace: &Path,
        state: &Path,
        config: Config,
        authority_registry: &crate::cutover::Registry,
        injected: Option<Arc<dyn crate::provider::http::GenerationRuntime>>,
        observer: Option<crate::provider::http::RequestObserver>,
        shared: Option<&Host>,
        github: Option<Arc<crate::github::service::Service>>,
        mcp: Option<McpServices>,
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
        let scheduler = shared
            .map(|h| h.services.scheduler.clone())
            .unwrap_or_default();
        let endpoint_leases = if shared.is_some() {
            Vec::new()
        } else {
            authority_registry.endpoint_leases(&origins)?
        };
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
            if shared.is_none() {
                scheduler.register(
                    origin,
                    if local {
                        EndpointKind::Local
                    } else {
                        EndpointKind::Remote
                    },
                )?;
            }
        }
        let local_runtime = if let Some(parent) = shared {
            parent.local_runtime.clone()
        } else {
            match &config.runtime {
                Some(runtime) => Some(runtime.open(&config, &state, authority_registry).await?),
                None => injected,
            }
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
        if let Some(observer) = observer {
            http = http.with_request_observer(observer);
        }
        if let Some(runtime) = &local_runtime {
            http = http.with_runtime(runtime.clone());
        }
        let approval_policy = approval_policy::ApprovalPolicy::open(&state, &config.policy_mode)?;
        let services = Arc::new(Services {
            mcp,
            github,
            working_projects: Mutex::new(Default::default()),
            folder_access: Mutex::new(Default::default()),
            approval_policy: approval_policy.clone(),
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
            services: services.clone(),
        });
        let pool = factory.inner.pool.clone();
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
            live_changed: tokio::sync::Notify::new(),
            live_revision: std::sync::atomic::AtomicU64::new(0),
            approval_policy,
            local_runtime,
            endpoint_leases: Mutex::new(endpoint_leases),
            authority: Arc::new(Mutex::new(authority)),
            conversation_owners: Arc::new(tokio::sync::Mutex::new(Default::default())),
            pool,
            closing: AtomicBool::new(false),
            workspace_closing: Arc::new(AtomicBool::new(false)),
            runtime_closing: shared
                .map(|h| h.runtime_closing.clone())
                .unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
            shared_hosts: shared
                .map(|h| h.shared_hosts.clone())
                .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(Vec::new()))),
            owns_resources: true,
            owns_runtime: shared.is_none(),
            services,
            ui,
            uploads: Mutex::new(Default::default()),
            diff_review: Mutex::new(None),
            local_git_plans:Mutex::new(Default::default()),
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
            workspace_controls: Arc::new(tokio::sync::Mutex::new(())),
            last_view: Mutex::new(None),
            wake: tokio::sync::Notify::new(),
            driver: tokio::sync::Mutex::new(None),
            stopping: AtomicBool::new(false),
        });
        host.conversation_owners.lock().await.insert(
            host.commands.snapshot()?.scope.session.as_str().into(),
            Arc::downgrade(&host),
        );
        host.shared_hosts.lock().await.push(Arc::downgrade(&host));
        host.start_folder_project_poll();
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
                    let delta = observations.event(event);
                    self.live_revision.fetch_add(1, Ordering::Release);
                    self.live_changed.notify_waiters();
                    if let Some(text) = delta {
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
    /// Serialize an explicit desktop repository change with prompt/session
    /// admission, and keep that boundary held until the caller has finished.
    pub async fn idle_client_guard(&self) -> Result<tokio::sync::MutexGuard<'_, ()>> {
        let guard = self.workspace_controls.lock().await;
        let working = self.working_folder(false)?;
        for host in self
            .shared_hosts
            .lock()
            .await
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            if !host.commands.snapshot()?.queued.is_empty()
                && host.folder_selection().map_or(true, |f|f.selection.folders.iter().any(|g|working.root().starts_with(&g.path)||g.path.starts_with(working.root())))
            {
                return Err(DomainError::Unavailable(
                    "Wait for this project’s running conversations before changing its repository"
                        .into(),
                ));
            }
        }
        self.authority
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .admit()?;
        if !self.commands.snapshot()?.queued.is_empty()
            || !self.registry.selected()?.services.runtime().settled()?
        {
            return Err(DomainError::Unavailable(
                "Stop the current response before changing the repository".into(),
            ));
        }
        Ok(guard)
    }
    /// Streaming display waits for actual events, independent of durable journal
    /// projection. The client paints at its display cadence and keeps one waiter.
    pub async fn live_display(&self, after: u64) -> Result<Value> {
        let notified = self.live_changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.live_revision.load(Ordering::Acquire) == after {
            let _ = tokio::time::timeout(Duration::from_millis(500), notified).await;
        }
        let scope = self.commands.snapshot()?.scope;
        let observations = self
            .observations
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        Ok(
            json!({"revision":self.live_revision.load(Ordering::Acquire),"scope":scope,"observations":observations.view(&scope)}),
        )
    }
    pub async fn dispatch(
        self: &Arc<Self>,
        mut envelope: w::CommandEnvelope,
    ) -> Result<w::BridgeFrame> {
        if !matches!(envelope.command, w::Command::Cancel { .. }) {
            self.authority
                .lock()
                .map_err(|_| DomainError::DurabilityUnknown)?
                .admit()?;
        }
        envelope
            .version
            .negotiate()
            .map_err(|_| DomainError::InvalidInput("protocol version".into()))?;
        w::encode(&w::WireMessage::Command(envelope.clone()))
            .map_err(|_| DomainError::LimitExceeded("command envelope".into()))?;
        if envelope.scope != self.commands.scope()? {
            return Err(DomainError::Conflict);
        }
        // Cancellation must remain available after a provider, journal, or
        // admission failure. The protocol scope check above still applies;
        // this only signals an operation owned by this host's live queue.
        if let w::Command::Cancel { operation } = &envelope.command
            && self.commands.emergency_cancel(operation)?
        {
            let snapshot = self.commands.snapshot().ok();
            return Ok(w::BridgeFrame::Reply {
                id: envelope.id,
                scope: envelope.scope.clone(),
                revision: snapshot
                    .map(|snapshot| snapshot.revision)
                    .unwrap_or(envelope.expected_revision.clone()),
                durable: false,
                status: "accepted".into(),
                data: json!({"kind":"accepted","operation":operation}),
            });
        }
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
        let _workspace_guard = self.workspace_controls.lock().await;
        let _guard = self.controls.lock().await;
        self.authority
            .lock()
            .map_err(|_| DomainError::DurabilityUnknown)?
            .admit()?;
        self.check_authority()?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(DomainError::Unavailable("Client host is stopping".into()));
        }
        if let w::Command::Session { action, args } = &envelope.command {
            if action == "resume" {
                if let Some(target) = args["session"].as_str() {
                    let owners = self.conversation_owners.lock().await;
                    if owners
                        .get(target)
                        .and_then(std::sync::Weak::upgrade)
                        .is_some_and(|host| {
                            !Arc::ptr_eq(&host, self)
                                && !host.stopping.load(Ordering::Acquire)
                        })
                    {
                        return Err(DomainError::Unavailable(
                            "This conversation is already open. Select it in the chat sidebar."
                                .into(),
                        ));
                    }
                }
            }
        }
        let snapshot = self.commands.snapshot()?;
        // A retired scope can observe its own durable command receipt, but
        // cannot submit new work or cross this host's workspace boundary.
        if envelope.scope != snapshot.scope
            && !(envelope.scope.workspace == snapshot.scope.workspace
                && matches!(envelope.command, w::Command::CommandResult { .. }))
        {
            return Err(DomainError::Conflict);
        }
        if matches!(&envelope.command,w::Command::Files {action,..} if matches!(action.as_str(), "save" | "change_undo" | "change_undo_recover" | "git_confirm"))
            && envelope.expected_revision != snapshot.revision
        {
            return Err(DomainError::Conflict);
        }
        if let w::Command::Runtime { action, args } = &envelope.command {
            if action == "folder_access" {
                if envelope.expected_revision != snapshot.revision { return Err(DomainError::Conflict); }
                let data = self.folder_access_command(args).await?;
                let snapshot = self.commands.snapshot()?;
                return Ok(w::BridgeFrame::Reply { id: envelope.id, scope: snapshot.scope, revision: snapshot.revision, durable: args.get("action").is_some(), status: "query".into(), data });
            }
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
        // A failed capability reload must not leave the old grants usable.
        if !matches!(envelope.command, w::Command::CommandResult { .. } | w::Command::Cancel { .. } | w::Command::Approve { .. } | w::Command::ArtifactRead { .. })
            && !matches!(&envelope.command,w::Command::Runtime {action,..} if action=="terminal_stop")
        {
            let folders = self.folder_selection()?;
            if !folders.primary_available() { return Err(DomainError::Unavailable("Choose an available working folder in Chat folders before continuing".into())); }
        }
        if effects::is_effect(&envelope.command) {
            if let w::Command::Files {action,args}=&envelope.command && action=="git_confirm" {self.ensure_local_git_idle(args).await?;}
            if let w::Command::Files {action,args} = &envelope.command && (action == "change_undo" || action == "change_undo_recover") { self.ensure_undo_idle(args).await?; }
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
        let data = if matches!(query, w::Command::CommandResult { .. }) {
            // Receipt inspection belongs to the command owner, including while
            // a session transition is pending. It is not a selected-view query.
            None
        } else {
            tokio::task::spawn_blocking(move || {
                if owner.commands.client_transition_pending()? {
                    return Err(DomainError::Conflict);
                }
                owner.query(&query)
            })
            .await
            .map_err(|_| DomainError::DurabilityUnknown)??
        };
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
        if self.closing.load(Ordering::Acquire)
            || self.workspace_closing.load(Ordering::Acquire)
            || self.runtime_closing.load(Ordering::Acquire)
        {
            return Err(DomainError::Unavailable("Conversation is closing".into()));
        }
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
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        self.closing.store(true, Ordering::Release);
        if self.owns_runtime {
            self.runtime_closing.store(true, Ordering::Release);
        }
        if self.owns_resources {
            self.workspace_closing.store(true, Ordering::Release);
        }
        if self.owns_resources {
            let others: Vec<_> = self
                .shared_hosts
                .lock()
                .await
                .iter()
                .filter_map(std::sync::Weak::upgrade)
                .filter(|h| {
                    !std::ptr::eq(h.as_ref(), self)
                        && (self.owns_runtime || h.workspace.root() == self.workspace.root())
                })
                .collect();
            for other in others.into_iter().rev() {
                Box::pin(other.shutdown()).await?;
            }
        }
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
        self.release_unused_working_projects(true)?;
        if self.owns_resources {
            if let Some(runtime) = self.project.lock().unwrap().as_mut() {
                runtime.stop()?;
            }
        }
        self.stopping.store(true, Ordering::Release);
        self.wake.notify_one();
        if let Some(driver) = self.driver.lock().await.take() {
            driver.await.map_err(|_| DomainError::DurabilityUnknown)?;
        }
        if !self.owns_resources {
            return Ok(());
        }
        if self.owns_runtime {
            if let Some(runtime) = &self.local_runtime {
                runtime.shutdown().await?;
            }
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
