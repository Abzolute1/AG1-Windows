//! Explicit host trust and client UI callbacks for the admitted helper.
use super::*;
use crate::{
    extension_host::{Bridge, ExtensionService, NativeRequest},
    resource_fs::{ResourceFs, ScanControl},
    session::extensions::NativeExtensions,
};
use std::collections::VecDeque;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extensions {
    pub node: PathBuf,
    pub host_script: PathBuf,
    pub process_host: PathBuf,
    pub trusted: bool,
    #[serde(default)]
    pub allow_inference: bool,
}
#[derive(Default)]
pub struct Ui {
    state: Mutex<UiState>,
    changed: std::sync::Condvar,
}
#[derive(Default)]
struct UiState {
    pending: Option<Value>,
    answer: Option<Value>,
    events: VecDeque<Value>,
}
impl Ui {
    pub fn view(&self) -> Result<Value> {
        let s = self.state.lock().map_err(|_| DomainError::Conflict)?;
        Ok(json!({"pending":s.pending,"events":s.events}))
    }
    pub fn answer(&self, scope: &w::Scope, args: &Value) -> Result<Value> {
        let mut s = self.state.lock().map_err(|_| DomainError::Conflict)?;
        let p = s.pending.as_ref().ok_or(DomainError::NotFound)?;
        if p["id"] != args["id"]
            || p["session"] != scope.session.as_str()
            || p["workspace"] != scope.workspace.as_str()
        {
            return Err(DomainError::Conflict);
        }
        let value = &args["value"];
        match p["method"].as_str() {
            Some("ui.confirm") if !value.is_boolean() => {
                return Err(DomainError::InvalidInput("confirmation boolean".into()));
            }
            Some("ui.select")
                if !value.is_null()
                    && !p["arguments"]["args"][1]
                        .as_array()
                        .is_some_and(|choices| choices.contains(value)) =>
            {
                return Err(DomainError::InvalidInput("selection option".into()));
            }
            Some("ui.input" | "ui.editor")
                if !value.is_null() && !value.as_str().is_some_and(|s| s.len() <= 16384) =>
            {
                return Err(DomainError::InvalidInput("UI text".into()));
            }
            _ => {}
        }
        if s.answer.is_some() {
            return Err(DomainError::Conflict);
        }
        s.answer = Some(value.clone());
        self.changed.notify_all();
        Ok(json!({"answered":true}))
    }
}
struct UiBridge(Arc<Ui>);
impl Bridge for UiBridge {
    fn request(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>> {
        control.check()?;
        if request.arguments.to_string().len() > 16384 {
            return Err(DomainError::LimitExceeded("extension UI".into()));
        }
        let mut s = self.0.state.lock().map_err(|_| DomainError::Conflict)?;
        if matches!(
            request.method,
            "ui.select" | "ui.confirm" | "ui.input" | "ui.editor"
        ) {
            if s.pending.is_some() {
                return Err(DomainError::Conflict);
            }
            s.pending = Some(
                json!({"id":nonce()?,"workspace":request.scope.workspace,"session":request.scope.session,"epoch":request.scope.epoch,"revision":request.scope.revision,"method":request.method,"extension":request.extension,"arguments":request.arguments}),
            );
            let result = loop {
                if let Err(error) = control.check() {
                    break Err(error);
                }
                if let Some(answer) = s.answer.take() {
                    break Ok(Some(answer));
                }
                s = self
                    .0
                    .changed
                    .wait_timeout(s, Duration::from_millis(20))
                    .map_err(|_| DomainError::Conflict)?
                    .0;
            };
            s.pending = None;
            s.answer = None;
            result
        } else if matches!(
            request.method,
            "ui.notify"
                | "ui.setStatus"
                | "ui.setWorkingMessage"
                | "ui.setHiddenThinkingLabel"
                | "ui.setTitle"
                | "ui.pasteToEditor"
                | "ui.setEditorText"
                | "ui.setToolsExpanded"
                | "ui.setWidget"
        ) {
            s.events.push_back(json!({"id":nonce()?,"session":request.scope.session,"method":request.method,"extension":request.extension,"arguments":request.arguments}));
            while s.events.len() > 16
                || s.events.iter().map(|e| e.to_string().len()).sum::<usize>() > 24000
            {
                s.events.pop_front();
            }
            Ok(Some(Value::Null))
        } else {
            Err(DomainError::Unavailable(format!(
                "Explicit extension command callback {} is not admitted by this host",
                request.method
            )))
        }
    }
}
// Explicit host-owned services; none are looked up through ambient globals.
#[allow(clippy::too_many_arguments)]
pub(super) fn load(
    workspace: Arc<Workspace>,
    state: &Path,
    session: &str,
    epoch: u64,
    revision: u64,
    config: &Config,
    resources: &[ag1_core::resources::catalog::ExtensionResource],
    ui: Arc<Ui>,
    scheduler: Scheduler,
    http: ProviderHttp,
    store: Arc<Store>,
) -> Result<Option<Arc<NativeExtensions>>> {
    let Some(options) = &config.extensions else {
        return Ok(None);
    };
    if !options.trusted || resources.is_empty() {
        return Ok(None);
    }
    let spools = state.join("extensions").join(nonce()?);
    fs::create_dir_all(&spools).map_err(crate::io_error)?;
    let service = ExtensionService::new(
        workspace.clone(),
        ResourceFs::new(vec![
            workspace.clone(),
            WorkspaceService::new()?.register(&state.join("home"))?,
        ])?,
        Arc::new(crate::process::ProcessSupervisor::new(
            &options.process_host,
        )?),
        crate::extension_host::Config {
            node: options.node.clone(),
            host_script: options.host_script.clone(),
            spools,
            env: Default::default(),
        },
    )?;
    let scope = ag1_core::tools::registry::Scope {
        workspace: workspace.id().as_str().into(),
        session: session.into(),
        epoch,
        revision,
    };
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: std::time::Instant::now() + Duration::from_secs(20),
    };
    let project = workspace.root().join(".ag1");
    let project = project.to_str().ok_or(DomainError::PermissionDenied)?;
    let prepared = service.prepare(
        &scope,
        resources,
        ag1_core::resources::trust::Policy {
            project_config_dir: project,
            canonical_project_dir: project,
            explicit: Some(true),
            environment: None,
            user_trusted_projects: &[],
        },
        &control,
    )?;
    let allow_inference = options.allow_inference;
    let models = config.models.clone();
    let credentials = credentials::HostCredentials {
        providers: config.credentials.clone(),
        fallback: config.credential_env.clone(),
    };
    let owners = resources
        .iter()
        .map(|e| e.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let factory = Arc::new(
        move |scope: &ag1_core::tools::registry::Scope| -> Result<Box<dyn Bridge>> {
            if !allow_inference {
                return Ok(Box::new(UiBridge(ui.clone())));
            }
            let models = models
                .iter()
                .filter(|m| {
                    !m.get("runtimeConflict")
                        .is_some_and(|v| v != false && !v.is_null())
                })
                .map(|m| {
                    Ok(crate::extension_models::ModelConfig {
                        model: m.clone(),
                        endpoint: crate::provider::http::endpoint_origin(
                            m["baseUrl"].as_str().ok_or(DomainError::NotFound)?,
                        )?,
                        options: json!({"maxTokens":m["maxTokens"]}),
                        allowed_options: ["maxTokens", "temperature", "reasoning", "thinking"]
                            .into_iter()
                            .map(str::to_owned)
                            .collect(),
                        headers: None,
                        limits: GenerationLimits {
                            max_output_bytes: 512 * 1024,
                            prefill_ms: 0,
                            idle_ms: 0,
                            overall_ms: 300000,
                        },
                        max_response_bytes: 4 * 1024 * 1024,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let resolver = crate::extension_models::ConfiguredModels::new(
                scope.clone(),
                owners.clone(),
                models,
                credentials.clone(),
                Arc::new(WallClock),
            )?;
            let bridge = crate::extension_inference::InferenceBridge::new(
                scheduler.clone(),
                http.clone(),
                resolver,
                UiBridge(ui.clone()),
            )?;
            Ok(Box::new(CaptureBridge {
                inner: bridge,
                store: store.clone(),
            }))
        },
    );
    Ok(Some(Arc::new(
        NativeExtensions::bind(service.start(prepared, &scope, &control)?, &control)?
            .with_client_bridge_factory(factory),
    )))
}

struct CaptureBridge<B> {
    inner: B,
    store: Arc<Store>,
}
impl<B: Bridge> CaptureBridge<B> {
    fn retain(
        &self,
        scope: &ag1_core::tools::registry::Scope,
        operation: &str,
        method: &str,
        result: &Result<Option<Value>>,
    ) -> Result<()> {
        if !method.starts_with("inference.") {
            return Ok(());
        }
        use ag1_core::context::ContextArtifactPort;
        let timestamp = timestamp();
        let mut artifacts = ag1_storage::context::ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            &scope.session,
            operation,
            &timestamp,
            Default::default(),
        )?;
        let value = match result {
            Ok(value) => json!({"method":method,"output":value}),
            Err(error) => json!({"method":method,"error":format!("{error:?}")}),
        };
        artifacts.put_evidence(
            &value.to_string(),
            "runtime-extension-output",
            &json!({"type":"system","name":method}),
            &[],
            None,
        )?;
        Ok(())
    }
}
impl<B: Bridge> Bridge for CaptureBridge<B> {
    fn request(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>> {
        let (scope, operation, method) = (request.scope, request.operation, request.method);
        let result = self.inner.request(request, control);
        self.retain(scope, operation, method, &result)?;
        result
    }
    fn request_stream(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn crate::extension_host::NativeStream,
    ) -> Result<Option<Value>> {
        let (scope, operation, method) = (request.scope, request.operation, request.method);
        let result = self.inner.request_stream(request, control, stream);
        self.retain(scope, operation, method, &result)?;
        result
    }
    fn event(
        &mut self,
        scope: &ag1_core::tools::registry::Scope,
        operation: &str,
        event: &Value,
    ) -> Result<()> {
        self.inner.event(scope, operation, event)
    }
}
