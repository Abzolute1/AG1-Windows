//! Concrete preserved runtime adapter and supervised pinned TS provider probe.
use super::*;
use crate::{
    cutover::{Registry, configuration::ConfigurationCapture},
    executor::BlockingPool,
    llama::{
        LaunchPlan, Material,
        custody::RuntimeBundle,
        lifecycle::{Observation, RuntimeHost},
        native::{MaterialVerifier, NativeRuntimeHost},
    },
    process::{ProcessSpec, ProcessSupervisor},
    provider::{
        generation::{GenerationLimits, GenerationRequest, request_generation},
        http::{GenerationRuntime, ProviderHttp, cancellation},
        request_http::{HttpCredential, generation_http_request},
        requests::RequestContext,
    },
};
use ag1_core::{
    OperationId, SecretBytes,
    inference::{EndpointKind, Scheduler, Ticket},
    provider::{ApiFamily, ProviderEvent, assistant_message},
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    pub node: Material,
    pub probe: Material,
    pub source: PathBuf,
    pub materials: Vec<Material>,
}
impl Reference {
    fn verify(&self, verifier: &mut MaterialVerifier) -> Result<()> {
        let required = self
            .source
            .join("packages/ai/dist/providers/openai-completions.js");
        if !self
            .materials
            .iter()
            .any(|m| Path::new(&m.path) == required)
        {
            return Err(DomainError::PermissionDenied);
        }
        verifier.verify(&[self.node.clone(), self.probe.clone()])?;
        verifier.verify(&self.materials)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveRequest {
    pub capture: ConfigurationCapture,
    pub models: Vec<Value>,
    pub process_host: Material,
    pub reference: Reference,
    pub destination: PathBuf,
}
pub struct TransportDriver<H: RuntimeHost> {
    runtime: Arc<RuntimeBundle<H>>,
    http: ProviderHttp,
    scheduler: Scheduler,
    pending: Vec<Ticket>,
    reference: Reference,
    helper: Material,
    directory: PathBuf,
    pool: BlockingPool,
    serial: u64,
    verifier: Arc<Mutex<MaterialVerifier>>,
}
impl<H: RuntimeHost + Send + 'static> TransportDriver<H>
where
    H::Handle: Send,
{
    /// Trusted adapter composition for fixture rehearsals. It cannot mint live
    /// acceptance; the concrete native constructor below owns that boundary.
    pub fn new(
        runtime: Arc<RuntimeBundle<H>>,
        reference: Reference,
        helper: Material,
        directory: &Path,
    ) -> Result<Self> {
        let mut verifier = MaterialVerifier::default();
        reference.verify(&mut verifier)?;
        verifier.verify(std::slice::from_ref(&helper))?;
        crate::cutover::private_dir(directory)?;
        let scheduler = Scheduler::default();
        scheduler.register(runtime.origin(), EndpointKind::Local)?;
        // execute() prepares custody once under the held permit, before either
        // adapter timer starts. Ordinary native hosts still compose their guard
        // through ProviderHttp::with_runtime at the generation boundary.
        let http = ProviderHttp::new(&[runtime.origin().into()], Duration::from_secs(5))?;
        Ok(Self {
            runtime,
            http,
            scheduler,
            reference,
            helper,
            directory: directory.into(),
            pool: BlockingPool::new(1, 8)?,
            serial: 0,
            pending: vec![],
            verifier: Arc::new(Mutex::new(verifier)),
        })
    }
    /// Opt-in diagnostic capture for the Rust transport only. The pinned TS
    /// adapter and request/model settings remain unchanged.
    pub fn with_response_observer(
        mut self,
        observer: crate::provider::http::ResponseObserver,
    ) -> Self {
        self.http = self.http.with_response_observer(observer);
        self
    }
    async fn execute(&mut self, runner: Runner, request: &Request) -> Result<Answer> {
        self.serial = self.serial.checked_add(1).ok_or(DomainError::QueueFull)?;
        let id = format!("p13-canary-{}", self.serial);
        let ticket = self
            .scheduler
            .submit(self.runtime.origin(), OperationId::new(&id)?)?;
        let permit = self
            .scheduler
            .start(&ticket)?
            .ok_or(DomainError::Conflict)?;
        self.pending.push(ticket);
        let expected = payload(request)?;
        let key = SecretBytes::new(b"p13-local-no-credential".to_vec());
        let input = RequestContext {
            model: &request.model,
            context: &request.context,
            options: &request.options,
            anthropic_oauth: false,
            google_project: None,
            request_id: &id,
        };
        let http_request =
            generation_http_request(&input, HttpCredential::ApiKey(&key), 4 * 1024 * 1024)?;
        // The recorded comparator payload must describe the actual Rust body,
        // not merely a second independently assembled expectation.
        if serde_json::from_slice::<Value>(&http_request.body).map_err(crate::cutover::json)?
            != expected
        {
            return Err(DomainError::Conflict);
        }
        // Loading is excluded from provider TTFT for both implementations.
        self.runtime.prepare(&http_request).await?;
        let runtime_before = self.runtime.resources().await?;
        let mut answer = match runner {
            Runner::Rust => {
                let started = Instant::now();
                let (source, mut cancel) = cancellation();
                let mut first = None;
                let outcome = request_generation(
                    &self.http,
                    GenerationRequest {
                        request_id: id,
                        tool_names: vec!["p13_echo".into()],
                        family: ApiFamily::Completions,
                        request: http_request,
                        limits: GenerationLimits {
                            max_output_bytes: 1024 * 1024,
                            prefill_ms: 900000,
                            idle_ms: 180000,
                            overall_ms: 900000,
                        },
                    },
                    permit,
                    &mut cancel,
                    |event| {
                        if let ProviderEvent::Delta { text, .. } = event
                            && !text.is_empty()
                        {
                            first.get_or_insert(started.elapsed().as_millis() as u64);
                            if request.cancel_after_first_content {
                                source.cancel();
                            }
                        }
                        Ok(())
                    },
                )
                .await;
                let (snapshot, cancelled) = match outcome {
                    Ok(value) => (value, false),
                    Err(error)
                        if error.error == DomainError::Cancelled
                            && request.cancel_after_first_content
                            && first.is_some() =>
                    {
                        (error.partial.ok_or(DomainError::Conflict)?, true)
                    }
                    Err(error) => return Err(error.error),
                };
                let mut message = assistant_message(&snapshot, &request.model, 0)?;
                if cancelled {
                    message["stopReason"] = json!("aborted");
                }
                Answer {
                    message,
                    payload: expected,
                    first_content_ms: first,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    cancelled,
                    settled: !cancelled,
                    runtime_before: None,
                    runtime_after: None,
                }
            }
            Runner::Typescript => {
                let reference = self.reference.clone();
                let helper = self.helper.clone();
                let directory = self.directory.clone();
                let runtime = self.runtime.clone();
                let request = request.clone();
                let verifier = self.verifier.clone();
                self.pool.submit(move || {
                    // The child job retains runtime custody and the generation
                    // permit even if its caller drops the asynchronous future.
                    let _runtime = runtime;
                    {
                        let mut verifier = verifier.lock().map_err(|_| DomainError::DurabilityUnknown)?;
                        reference.verify(&mut verifier)?;
                        verifier.verify(std::slice::from_ref(&helper))?;
                    }
                    let input_path = directory.join(format!("{id}-input.json"));
                    let output_path = directory.join(format!("{id}-output.json"));
                    let data = json!({"request":request,"expected_payload":expected,"origin":_runtime.origin()});
                    crate::cutover::atomic(&input_path, &serde_json::to_vec(&data).map_err(crate::cutover::json)?)?;
                    if output_path.exists() { return Err(DomainError::Conflict); }
                    let supervisor = ProcessSupervisor::new(Path::new(&helper.path))?;
                    let mut child = supervisor.spawn(ProcessSpec {
                        executable: reference.node.path.into(), argv: vec![reference.probe.path,
                            reference.source.display().to_string(), input_path.display().to_string(), output_path.display().to_string()],
                        cwd: directory.clone(), env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
                        timeout_ms: 910000, grace_ms: 1000, resident: false,
                    }, &directory.join(format!("{id}-reference.log")))?;
                    let status = child.wait(Duration::from_secs(915))?;
                    if !status.success() { return Err(DomainError::Unavailable("Pinned TS canary probe failed; private log retained".into())); }
                    // Stop/reap the entire owned helper tree before inspecting output.
                    child.stop()?;
                    let mut answer: Answer = serde_json::from_slice(&crate::cutover::bounded(&output_path, 4*1024*1024)?).map_err(crate::cutover::json)?;
                    if !answer.cancelled { permit.terminal()?; answer.settled = true; }
                    // Cancellation drops/quarantines its permit until owned exit.
                    Ok(answer)
                }).await?
            }
        };
        if answer.cancelled {
            self.reset().await?;
            answer.settled = true;
        } else {
            answer.runtime_after = self.runtime.resources().await?;
        }
        answer.runtime_before = runtime_before;
        Ok(answer)
    }
}
impl<H: RuntimeHost + Send + 'static> Driver for TransportDriver<H>
where
    H::Handle: Send,
{
    fn reset(&mut self) -> PortFuture<'_, ()> {
        Box::pin(async {
            // Barrier: a dropped TS call must finish its owned child first.
            self.pool.submit(|| Ok(())).await?;
            self.runtime.unload_owned().await?;
            for ticket in self.pending.drain(..) {
                match self.scheduler.reconcile(&ticket) {
                    Ok(()) | Err(DomainError::NotFound) => (),
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        })
    }
    fn run<'a>(&'a mut self, runner: Runner, request: &'a Request) -> PortFuture<'a, Answer> {
        Box::pin(self.execute(runner, request))
    }
}
type Observer = Box<dyn FnMut(&LaunchPlan) -> Result<Observation> + Send>;
type Bundle = RuntimeBundle<NativeRuntimeHost<Observer>>;
/// Non-serializable proof: only an executed concrete native canary matrix can
/// create this. It retains the global runtime lease until transfer completes.
pub struct Acceptance {
    pub(super) capture: ConfigurationCapture,
    pub(super) models: Vec<Value>,
    process_host: Material,
    report: Report,
    _runtime: Arc<Bundle>,
}
impl Acceptance {
    pub fn report(&self) -> &Report {
        &self.report
    }
    pub(in crate::cutover) fn models(&self) -> &[Value] {
        &self.models
    }
    pub(in crate::cutover) fn verify_configuration(
        &self,
        config: &crate::client::Config,
    ) -> Result<()> {
        crate::llama::window::verify(None)?;
        let runtime = config
            .runtime
            .as_ref()
            .ok_or(DomainError::PermissionDenied)?;
        if config.fixture
            || self.report.fixture
            || self.report.status != "passed"
            || !self.report.runtime_settled
            || self.report.rows.len() != 36
            || runtime.capture.sha256 != self.capture.sha256
            || runtime.capture.plan_sha256 != self.capture.plan_sha256
            || runtime.process_host != self.process_host
        {
            return Err(DomainError::PermissionDenied);
        }
        for model in &self.models {
            if !config.models.contains(model) {
                return Err(DomainError::Conflict);
            }
        }
        self.capture.verify()?;
        Ok(())
    }
}
pub async fn run_live(input: LiveRequest) -> Result<Acceptance> {
    let registry = Registry::for_user()?;
    let lease = registry.runtime_lease()?;
    // Reject the current external service before any endpoint access or model
    // action, including when called directly outside the Python gate.
    crate::llama::window::verify(None)?;
    let plans = input.capture.verify()?;
    if plans.len() != 3 || input.models.len() != 3 {
        return Err(DomainError::Conflict);
    }
    for plan in &plans {
        let model = input
            .models
            .iter()
            .find(|m| m["id"] == plan.model_id)
            .ok_or(DomainError::Conflict)?;
        if model["provider"] != "llamacpp"
            || model["api"] != "openai-completions"
            || model["contextWindow"].as_u64() != Some(plan.requested_context)
            || model["maxTokens"].as_u64() != Some(plan.max_tokens)
            || crate::provider::http::endpoint_origin(
                model["baseUrl"].as_str().ok_or(DomainError::Conflict)?,
            )? != crate::llama::observation::origin(plan)?
        {
            return Err(DomainError::Conflict);
        }
    }
    MaterialVerifier::default().verify(std::slice::from_ref(&input.process_host))?;
    let destination = crate::cutover::migration::new_path(&input.destination)?;
    std::fs::create_dir(&destination).map_err(crate::io_error)?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o700))
        .map_err(crate::io_error)?;
    let handle = tokio::runtime::Handle::current();
    let observer: Observer = Box::new(move |plan| {
        let origin = crate::llama::observation::origin(plan)?;
        handle.block_on(async {
            let http = ProviderHttp::new(std::slice::from_ref(&origin), Duration::from_secs(2))?;
            crate::llama::observation::observe(&http, &origin).await
        })
    });
    let host = NativeRuntimeHost::new(Path::new(&input.process_host.path), &destination, observer)?
        .require_exclusive_gpu();
    let runtime = Arc::new(RuntimeBundle::new(
        host,
        plans,
        lease,
        Duration::from_secs(900),
    )?);
    let mut driver = TransportDriver::new(
        runtime.clone(),
        input.reference,
        input.process_host.clone(),
        &destination,
    )?;
    let mut report = super::matrix(&mut driver, &input.models, false).await;
    if report.status == "passed" && super::verify_recording(&report, &input.models).is_err() {
        report.status = "failed".into();
        report.failures.push(
            "Recorded canary request, response, timing or ownership verification failed".into(),
        );
    }
    let events = runtime.events()?;
    let bytes = serde_json::to_vec(
        &json!({"report":report,"models":input.models,"runtime_events":events,"configuration":input.capture}),
    )
    .map_err(crate::cutover::json)?;
    crate::cutover::atomic(&destination.join("canaries.json"), &bytes)?;
    if report.status != "passed" {
        return Err(DomainError::Unavailable(
            "P13 live canaries failed; evidence retained".into(),
        ));
    }
    input.capture.verify()?;
    Ok(Acceptance {
        capture: input.capture,
        models: input.models,
        process_host: input.process_host,
        report,
        _runtime: runtime,
    })
}
