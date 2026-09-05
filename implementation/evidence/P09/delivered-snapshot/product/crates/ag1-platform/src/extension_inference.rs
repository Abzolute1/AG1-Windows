//! Extension inference uses the shared P06 endpoint scheduler and native HTTP
//! transport. Only host-resolved models/endpoints/credentials construct requests.
use crate::{
    extension_host::{Bridge, NativeRequest, NativeStream},
    provider::{
        generation::{GenerationRequest, request_borrowed_generation, request_generation},
        http::{ProviderHttp, cancellation},
    },
    resource_fs::ScanControl,
};
use ag1_core::{
    DomainError, OperationId,
    inference::{GenerationPermit, Scheduler},
    provider::{BlockKind, ProviderEvent, ProviderSnapshot, StopReason, StreamReducer},
    tools::registry::Scope,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
/// The selector is data. The resolver must use host configuration, never a
/// callback's model.baseUrl/apiKey/headers, to choose an endpoint and credentials.
#[derive(Clone, Copy)]
pub struct InferenceInput<'a> {
    pub scope: &'a Scope,
    pub extension: &'a str,
    pub operation: &'a str,
    pub host_operation: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub context: &'a Value,
    pub options: &'a Value,
}
pub struct ResolvedGeneration {
    pub endpoint: String,
    /// Canonical host model descriptor, used for message identity and cost rates.
    pub model: Value,
    pub generation: GenerationRequest,
    pub timestamp_ms: u64,
}
pub struct ResolvedCustomGeneration {
    pub target: crate::extension_providers::CustomTarget,
    pub endpoint: String,
    /// Native options, including credentials explicitly admitted to the hook
    /// owner. These must never enter a public catalog or evidence record.
    pub options: Value,
    pub timestamp_ms: u64,
    pub max_output_bytes: usize,
    pub overall_ms: u64,
}
pub trait ModelResolver {
    fn resolve_with_stream(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        _stream: &mut dyn NativeStream,
    ) -> Result<ResolvedGeneration, DomainError> {
        self.resolve(input, control)
    }
    /// Credential resolution may invoke an admitted OAuth hook in this same host.
    /// The resolver still owns caller/model/endpoint grants before using this port.
    fn resolve_custom_with_stream(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        _stream: &mut dyn NativeStream,
    ) -> Result<Option<ResolvedCustomGeneration>, DomainError> {
        self.resolve_custom(input, control)
    }
    fn resolve_custom(
        &mut self,
        _input: InferenceInput<'_>,
        _control: &ScanControl<'_>,
    ) -> Result<Option<ResolvedCustomGeneration>, DomainError> {
        Ok(None)
    }
    fn resolve(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
    ) -> Result<ResolvedGeneration, DomainError>;
}
pub struct InferenceBridge<R, B> {
    scheduler: Scheduler,
    http: ProviderHttp,
    resolver: R,
    fallback: B,
    runtime: tokio::runtime::Runtime,
    custom_stack: Vec<ActiveCustom>,
}
struct EndpointLease {
    permit: RefCell<Option<GenerationPermit>>,
    fault: AtomicBool,
}
struct ActiveCustom {
    scope: Scope,
    operation: String,
    endpoint: String,
    lease: Rc<EndpointLease>,
}
/// Taking the permit excludes overlapping transports, including reentrant calls.
/// Drop restores ownership to the custom root; the transport separately latches
/// uncertain settlement on the lease.
struct BorrowedSlot {
    lease: Rc<EndpointLease>,
    permit: Option<GenerationPermit>,
}
impl BorrowedSlot {
    fn take(lease: Rc<EndpointLease>) -> Result<Self, DomainError> {
        if lease.fault.load(Ordering::Acquire) {
            return Err(DomainError::Unavailable(
                "parent generation requires settlement".into(),
            ));
        }
        let permit = lease
            .permit
            .borrow_mut()
            .take()
            .ok_or(DomainError::Conflict)?;
        Ok(Self {
            lease,
            permit: Some(permit),
        })
    }
}
impl Drop for BorrowedSlot {
    fn drop(&mut self) {
        *self.lease.permit.borrow_mut() = self.permit.take();
    }
}
enum HttpAdmission {
    Owned(GenerationPermit),
    Inherited(BorrowedSlot),
}
impl HttpAdmission {
    fn before_transport_failure(self) -> Result<(), DomainError> {
        match self {
            Self::Owned(permit) => permit.terminal(),
            Self::Inherited(_) => Ok(()),
        }
    }
}
impl<R: ModelResolver, B: Bridge> InferenceBridge<R, B> {
    pub fn new(
        scheduler: Scheduler,
        http: ProviderHttp,
        resolver: R,
        fallback: B,
    ) -> Result<Self, DomainError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| DomainError::Unavailable("extension inference runtime".into()))?;
        Ok(Self {
            scheduler,
            http,
            resolver,
            fallback,
            runtime,
            custom_stack: vec![],
        })
    }
    fn inherited_lease(
        &self,
        request: &NativeRequest<'_>,
        endpoint: &str,
    ) -> Option<Rc<EndpointLease>> {
        self.custom_stack
            .iter()
            .rev()
            .find(|active| {
                active.scope == *request.scope
                    && active.operation == request.operation
                    && active.endpoint == endpoint
            })
            .map(|active| active.lease.clone())
    }
    fn infer(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        control.check()?;
        if !["tool.execute", "command", "shortcut", "event"].contains(&r.phase)
            && !(r.phase == "provider"
                && self.custom_stack.last().is_some_and(|active| {
                    active.scope == *r.scope && active.operation == r.operation
                }))
        {
            return Err(DomainError::PermissionDenied);
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(DomainError::Unavailable(
                "extension broker requires a native blocking worker".into(),
            ));
        }
        ag1_core::resources::packages::validate_settings_document(r.arguments)?;
        if r.arguments.to_string().len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "extension inference arguments".into(),
            ));
        }
        let model = &r.arguments["model"];
        let provider = model["provider"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| invalid("extension provider selector"))?;
        let id = model["id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(|| invalid("extension model selector"))?;
        let empty = json!({});
        let options = r
            .arguments
            .get("options")
            .filter(|v| !v.is_null())
            .unwrap_or(&empty);
        if !options.is_object() {
            return Err(invalid("extension provider options"));
        }
        // Callback credentials/URL overrides cannot escape host configuration. Provider
        // sampling options are passed to the native resolver for its own allowlist.
        for key in [
            "apiKey",
            "headers",
            "baseUrl",
            "fetch",
            "signal",
            "onPayload",
            "onResponse",
        ] {
            if options.get(key).is_some() {
                return Err(DomainError::PermissionDenied);
            }
        }
        let context = &r.arguments["context"];
        if !context.is_object() || !context["messages"].is_array() {
            return Err(invalid("extension inference context"));
        }
        let request_id = format!(
            "extension-inference-{}",
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let input = InferenceInput {
            scope: r.scope,
            extension: r.extension,
            operation: &request_id,
            host_operation: r.operation,
            provider,
            model: id,
            context,
            options,
        };
        if let Some(custom) = self
            .resolver
            .resolve_custom_with_stream(input, control, stream)?
        {
            return self.infer_custom(r, control, stream, &request_id, custom);
        }
        let mut resolved = self.resolver.resolve_with_stream(input, control, stream)?;
        if resolved.model["provider"] != provider || resolved.model["id"] != id {
            return Err(DomainError::Conflict);
        }
        if resolved.generation.request_id != request_id {
            return Err(DomainError::Conflict);
        }
        self.http.validate_request(&resolved.generation.request)?;
        // These limits apply only to the optional host projection, not the tuned model
        // profile. No prefill/idle watchdog is introduced by this bridge.
        let remaining = control
            .deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(u64::MAX as u128) as u64;
        if remaining == 0 {
            return Err(DomainError::TimedOut);
        }
        resolved.generation.limits.overall_ms =
            resolved.generation.limits.overall_ms.min(remaining);
        resolved.generation.limits.max_output_bytes =
            resolved.generation.limits.max_output_bytes.min(512 * 1024);
        let streaming = r.method == "inference.stream";
        let mut projection = MessageProjection::new(
            &resolved.model,
            resolved.timestamp_ms,
            resolved.generation.family,
            resolved.generation.limits.max_output_bytes,
        )?;
        let initial_message = projection.message()?;
        let inherited = self.inherited_lease(&r, &resolved.endpoint);
        let stream = RefCell::new(stream);
        let admission = if let Some(lease) = inherited {
            HttpAdmission::Inherited(BorrowedSlot::take(lease)?)
        } else {
            let ticket = self
                .scheduler
                .submit(&resolved.endpoint, OperationId::new(&request_id)?)?;
            let permit = self.runtime.block_on(async {
                loop {
                    let check = control.check().and_then(|()| {
                        stream.borrow_mut().cancelled().and_then(|yes| {
                            if yes {
                                Err(DomainError::Cancelled)
                            } else {
                                Ok(())
                            }
                        })
                    });
                    if let Err(error) = check {
                        self.scheduler.cancel(&ticket)?;
                        return Err(error);
                    }
                    if let Some(permit) = self.scheduler.start(&ticket)? {
                        return Ok(permit);
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })?;
            HttpAdmission::Owned(permit)
        };
        if streaming
            && let Err(error) = stream
                .borrow_mut()
                .emit(&json!({"type":"start","partial":initial_message}))
        {
            admission.before_transport_failure()?;
            return Err(error);
        }
        let (cancel_source, mut cancel) = cancellation();
        let result = self.runtime.block_on(async {
            let generation = async {
                let observer = |event: &ProviderEvent| {
                    let event = projection.apply(event)?;
                    if streaming && let Some(event) = event { stream.borrow_mut().emit(&event)?; }
                    Ok(())
                };
                match admission {
                    HttpAdmission::Owned(permit) => request_generation(&self.http, resolved.generation, permit, &mut cancel, observer).await,
                    HttpAdmission::Inherited(mut slot) => request_borrowed_generation(&self.http, resolved.generation, slot.permit.as_mut().expect("borrowed slot owns permit"), &slot.lease.fault, &mut cancel, observer).await,
                }
            };
            tokio::pin!(generation);
            loop {
                tokio::select! {
                    result = &mut generation => return result.map_err(|failure| failure.error),
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {
                        let status = control.check().and_then(|()| stream.borrow_mut().cancelled().and_then(|yes| if yes { Err(DomainError::Cancelled) } else { Ok(()) }));
                        if let Err(error) = status {
                            cancel_source.cancel(); let _ = generation.await; return Err(error);
                        }
                    }
                }
            }
        });
        let snapshot = result?;
        let message = message(&snapshot, &resolved.model, resolved.timestamp_ms)?;
        if streaming {
            let reason = message["stopReason"].clone();
            let event = if ["error", "aborted"].contains(&reason.as_str().unwrap_or("")) {
                json!({"type":"error","reason":reason,"error":message})
            } else {
                json!({"type":"done","reason":reason,"message":message})
            };
            stream.borrow_mut().emit(&event)?;
        }
        Ok(Some(message))
    }
    fn infer_custom(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
        request_id: &str,
        resolved: ResolvedCustomGeneration,
    ) -> Result<Option<Value>, DomainError> {
        use crate::extension_host::ProviderInvocation;
        if resolved.overall_ms == 0
            || resolved.overall_ms > 86_400_000
            || !(1024..=512 * 1024).contains(&resolved.max_output_bytes)
            || resolved.target.model()["provider"] != request.arguments["model"]["provider"]
            || resolved.target.model()["id"] != request.arguments["model"]["id"]
        {
            return Err(DomainError::PermissionDenied);
        }
        resolved.target.validate()?;
        let deadline = control
            .deadline
            .min(std::time::Instant::now() + Duration::from_millis(resolved.overall_ms));
        let control = &ScanControl {
            cancel: control.cancel,
            deadline,
        };
        let arguments = json!({"model":resolved.target.model(),"context":request.arguments["context"],"options":resolved.options});
        ag1_core::resources::packages::validate_settings_document(&arguments)?;
        if arguments.to_string().len() > 900 * 1024 {
            return Err(DomainError::LimitExceeded(
                "custom provider invocation".into(),
            ));
        }
        if self.custom_stack.len() >= 4 {
            return Err(DomainError::LimitExceeded("custom provider nesting".into()));
        }
        let inherited = self.inherited_lease(&request, &resolved.endpoint);
        let owns_lease = inherited.is_none();
        let lease = if let Some(lease) = inherited {
            if lease.fault.load(Ordering::Acquire) || lease.permit.borrow().is_none() {
                return Err(DomainError::Unavailable(
                    "parent generation requires settlement".into(),
                ));
            }
            lease
        } else {
            let ticket = self
                .scheduler
                .submit(&resolved.endpoint, OperationId::new(request_id)?)?;
            let permit = loop {
                let check = control.check().and_then(|()| {
                    stream.cancelled().and_then(|yes| {
                        if yes {
                            Err(DomainError::Cancelled)
                        } else {
                            Ok(())
                        }
                    })
                });
                if let Err(error) = check {
                    self.scheduler.cancel(&ticket)?;
                    return Err(error);
                }
                if let Some(permit) = self.scheduler.start(&ticket)? {
                    break permit;
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            Rc::new(EndpointLease {
                permit: RefCell::new(Some(permit)),
                fault: AtomicBool::new(false),
            })
        };
        if let Err(error) = resolved.target.validate() {
            if owns_lease {
                lease
                    .permit
                    .borrow_mut()
                    .take()
                    .ok_or(DomainError::Conflict)?
                    .terminal()?;
            }
            return Err(error);
        }
        let mut projection = CustomProjection {
            model: resolved.target.model().clone(),
            provider: resolved.target.binding().provider.clone(),
            terminal: None,
            events: 0,
            bytes: 0,
            limit: resolved.max_output_bytes,
        };
        self.custom_stack.push(ActiveCustom {
            scope: request.scope.clone(),
            operation: request.operation.into(),
            endpoint: resolved.endpoint,
            lease: lease.clone(),
        });
        let result = (|| {
            let result = stream.invoke_provider(
                ProviderInvocation {
                    deadline: Some(deadline),
                    name: &resolved.target.binding().provider,
                    registration: resolved.target.token(),
                    hook: "streamSimple",
                    arguments: &arguments,
                },
                &mut CustomObserver {
                    broker: self,
                    projection: &mut projection,
                    streaming: request.method == "inference.stream",
                },
            )?;
            let message = projection.terminal.ok_or_else(|| {
                DomainError::Unavailable("custom provider ended without terminal evidence".into())
            })?;
            if result.is_some_and(|result| result != message) {
                return Err(DomainError::Conflict);
            }
            if lease.fault.load(Ordering::Acquire) {
                return Err(DomainError::Unavailable(
                    "nested generation requires settlement".into(),
                ));
            }
            Ok(Some(message))
        })();
        self.custom_stack.pop();
        if result.is_err() {
            lease.fault.store(true, Ordering::Release);
        }
        if owns_lease {
            let permit = lease
                .permit
                .borrow_mut()
                .take()
                .ok_or(DomainError::Conflict)?;
            if result.is_ok() {
                permit.terminal()?;
            }
            // Failure drops the actual permit into quarantine.
        }
        result
    }
}
struct CustomProjection {
    model: Value,
    provider: String,
    terminal: Option<Value>,
    events: usize,
    bytes: usize,
    limit: usize,
}
impl CustomProjection {
    fn message(&self, message: &Value) -> Result<(), DomainError> {
        if message["role"] != "assistant"
            || message["provider"] != self.model["provider"]
            || message["model"] != self.model["id"]
            || message["api"] != self.model["api"]
            || !message["content"].is_array()
        {
            return Err(invalid("custom provider assistant message"));
        }
        if message.to_string().len() > self.limit {
            return Err(DomainError::LimitExceeded("custom provider output".into()));
        }
        for block in message["content"]
            .as_array()
            .ok_or_else(|| invalid("custom provider content"))?
        {
            let valid = match block["type"].as_str() {
                Some("text") => block["text"].is_string(),
                Some("thinking") => block["thinking"].is_string(),
                Some("toolCall") => {
                    block["id"].is_string()
                        && block["name"].is_string()
                        && block.get("arguments").is_some()
                }
                _ => false,
            };
            if !valid {
                return Err(invalid("custom provider content block"));
            }
        }
        for key in ["input", "output", "cacheRead", "cacheWrite", "totalTokens"] {
            if message["usage"][key]
                .as_f64()
                .is_none_or(|v| !v.is_finite() || v < 0.)
            {
                return Err(invalid("custom provider usage"));
            }
        }
        for key in ["input", "output", "cacheRead", "cacheWrite", "total"] {
            if message["usage"]["cost"][key]
                .as_f64()
                .is_none_or(|v| !v.is_finite())
            {
                return Err(invalid("custom provider cost"));
            }
        }
        if message["timestamp"].as_f64().is_none_or(|v| !v.is_finite()) {
            return Err(invalid("custom provider timestamp"));
        }
        Ok(())
    }
    fn event(&mut self, event: &Value) -> Result<Option<Value>, DomainError> {
        if event["type"] != "provider_event" {
            return Ok(None);
        }
        if event["provider"] != self.provider || self.terminal.is_some() {
            return Err(DomainError::Conflict);
        }
        let event = &event["event"];
        ag1_core::resources::packages::validate_settings_document(event)?;
        self.events += 1;
        self.bytes += event.to_string().len();
        if self.events > 10000 || self.bytes > 16 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("custom provider events".into()));
        }
        match event["type"].as_str() {
            Some("done" | "error") => {
                let message = if event["type"] == "done" {
                    &event["message"]
                } else {
                    &event["error"]
                };
                self.message(message)?;
                let reasons: &[&str] = if event["type"] == "done" {
                    &["stop", "length", "toolUse"]
                } else {
                    &["error", "aborted"]
                };
                if !reasons.contains(&message["stopReason"].as_str().unwrap_or(""))
                    || event["reason"] != message["stopReason"]
                {
                    return Err(invalid("custom provider terminal reason"));
                }
                self.terminal = Some(message.clone());
            }
            Some(
                "start" | "text_start" | "text_delta" | "text_end" | "thinking_start"
                | "thinking_delta" | "thinking_end" | "toolcall_start" | "toolcall_delta"
                | "toolcall_end",
            ) => {
                self.message(&event["partial"])?;
                let kind = event["type"]
                    .as_str()
                    .ok_or_else(|| invalid("custom provider event type"))?;
                if kind != "start" {
                    let index = event["contentIndex"]
                        .as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| invalid("custom provider content index"))?;
                    let block = event["partial"]["content"]
                        .get(index)
                        .ok_or_else(|| invalid("custom provider content index"))?;
                    let expected = if kind.starts_with("text_") {
                        "text"
                    } else if kind.starts_with("thinking_") {
                        "thinking"
                    } else {
                        "toolCall"
                    };
                    if block["type"] != expected
                        || (kind.ends_with("_delta") && !event["delta"].is_string())
                        || (kind.ends_with("_end")
                            && if expected == "toolCall" {
                                event["toolCall"] != *block
                            } else {
                                event["content"] != block[expected]
                            })
                    {
                        return Err(invalid("custom provider content event"));
                    }
                }
            }
            _ => return Err(invalid("custom provider stream event")),
        }
        Ok(Some(event.clone()))
    }
}
struct CustomObserver<'a, R, B> {
    broker: &'a mut InferenceBridge<R, B>,
    projection: &'a mut CustomProjection,
    streaming: bool,
}
impl<R: ModelResolver, B: Bridge> Bridge for CustomObserver<'_, R, B> {
    fn request(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        self.broker.request(r, control)
    }
    fn request_stream(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        self.broker.request_stream(r, control, stream)
    }
    fn event(&mut self, scope: &Scope, operation: &str, event: &Value) -> Result<(), DomainError> {
        self.broker.event(scope, operation, event)
    }
    fn provider_event(
        &mut self,
        scope: &Scope,
        operation: &str,
        event: &Value,
    ) -> Result<Option<Value>, DomainError> {
        let projected = self.projection.event(event)?;
        if projected.is_none() {
            self.broker.event(scope, operation, event)?;
        }
        Ok(if self.streaming { projected } else { None })
    }
}
struct NoStream;
impl NativeStream for NoStream {
    fn emit(&mut self, _: &Value) -> Result<(), DomainError> {
        Ok(())
    }
    fn cancelled(&mut self) -> Result<bool, DomainError> {
        Ok(false)
    }
}
impl<R: ModelResolver, B: Bridge> Bridge for InferenceBridge<R, B> {
    fn request(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        if r.method == "inference.complete" {
            self.infer(r, control, &mut NoStream)
        } else {
            self.fallback.request(r, control)
        }
    }
    fn request_stream(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        if ["inference.complete", "inference.stream"].contains(&r.method) {
            self.infer(r, control, stream)
        } else {
            self.fallback.request_stream(r, control, stream)
        }
    }
    fn event(&mut self, scope: &Scope, operation: &str, event: &Value) -> Result<(), DomainError> {
        self.fallback.event(scope, operation, event)
    }
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
fn stop(reason: Option<StopReason>) -> &'static str {
    match reason {
        Some(StopReason::Length) => "length",
        Some(StopReason::ToolUse) => "toolUse",
        Some(StopReason::Error) => "error",
        Some(StopReason::Aborted) => "aborted",
        _ => "stop",
    }
}
fn message(s: &ProviderSnapshot, model: &Value, timestamp: u64) -> Result<Value, DomainError> {
    let mut content = vec![];
    for block in &s.content {
        let mut value = match block.kind {
            BlockKind::Text => json!({"type":"text","text":block.text}),
            BlockKind::Thinking => json!({"type":"thinking","thinking":block.text}),
            BlockKind::Tool => {
                json!({"type":"toolCall","id":block.id,"name":block.name,"arguments":block.arguments.clone().unwrap_or_else(||json!({}))})
            }
        };
        if !block.signature.is_empty() {
            value[match block.kind {
                BlockKind::Thinking => "thinkingSignature",
                BlockKind::Tool => "thoughtSignature",
                BlockKind::Text => "textSignature",
            }] = json!(block.signature);
        }
        if block.kind == BlockKind::Tool
            && matches!(
                s.family,
                ag1_core::provider::ApiFamily::Responses
                    | ag1_core::provider::ApiFamily::AzureResponses
                    | ag1_core::provider::ApiFamily::CodexResponses
            )
        {
            value["partialJson"] = json!(block.text);
        }
        if block.redacted {
            value["redacted"] = json!(true);
        }
        if block.malformed_arguments {
            value["_argumentsTruncated"] = json!(true);
        }
        content.push(value);
    }
    let rates = ["input", "output", "cacheRead", "cacheWrite"]
        .map(|key| model["cost"][key].as_f64().unwrap_or(0.));
    let cost = s.cost(rates)?;
    let mut result = json!({"role":"assistant","content":content,"api":s.family,"provider":model["provider"],"model":model["id"],"usage":{"input":s.usage.input,"output":s.usage.output,"cacheRead":s.usage.cache_read,"cacheWrite":s.usage.cache_write,"totalTokens":s.usage.total,"cost":{"input":cost[0],"output":cost[1],"cacheRead":cost[2],"cacheWrite":cost[3],"total":cost[4]}},"stopReason":stop(s.stop_reason),"timestamp":timestamp});
    if let Some(error) = &s.error {
        result["errorMessage"] = json!(error);
    }
    if let Some(id) = &s.response_id {
        result["responseId"] = json!(id);
    }
    Ok(result)
}
struct MessageProjection {
    reducer: StreamReducer,
    model: Value,
    timestamp: u64,
    keys: BTreeMap<String, (usize, BlockKind)>,
    closed: BTreeSet<usize>,
}
impl MessageProjection {
    fn new(
        model: &Value,
        timestamp: u64,
        family: ag1_core::provider::ApiFamily,
        limit: usize,
    ) -> Result<Self, DomainError> {
        Ok(Self {
            reducer: StreamReducer::new(family, 0, limit)?,
            model: model.clone(),
            timestamp,
            keys: BTreeMap::new(),
            closed: BTreeSet::new(),
        })
    }
    fn message(&self) -> Result<Value, DomainError> {
        use ag1_core::provider::ApiFamily;
        let snapshot = self.reducer.snapshot();
        let mut result = message(&snapshot, &self.model, self.timestamp)?;
        for (index, block) in snapshot.content.iter().enumerate() {
            if block.kind != BlockKind::Tool || self.closed.contains(&index) {
                continue;
            }
            let output = &mut result["content"][index];
            if !block.text.is_empty() {
                output["arguments"] = crate::extension_partial_json::preview(&block.text)?;
            }
            let field = match snapshot.family {
                ApiFamily::Completions | ApiFamily::Mistral => Some("partialArgs"),
                ApiFamily::Anthropic
                | ApiFamily::Bedrock
                | ApiFamily::Responses
                | ApiFamily::AzureResponses
                | ApiFamily::CodexResponses => Some("partialJson"),
                _ => None,
            };
            if let Some(field) = field {
                output[field] = json!(block.text);
            }
        }
        Ok(result)
    }
    fn apply(&mut self, event: &ProviderEvent) -> Result<Option<Value>, DomainError> {
        self.reducer.apply(event.clone(), 0)?;
        let (key, stage, delta) = match event {
            ProviderEvent::Begin { key, kind, .. } => {
                let i = self.keys.len();
                self.keys.entry(key.clone()).or_insert((i, *kind));
                (key, "start", None)
            }
            ProviderEvent::Delta { key, text } => (key, "delta", Some(text.as_str())),
            ProviderEvent::End { key } => {
                self.closed
                    .insert(self.keys.get(key).ok_or(DomainError::Conflict)?.0);
                (key, "end", None)
            }
            _ => return Ok(None),
        };
        let (index, kind) = self.keys.get(key).ok_or(DomainError::Conflict)?;
        let prefix = match kind {
            BlockKind::Text => "text",
            BlockKind::Thinking => "thinking",
            BlockKind::Tool => "toolcall",
        };
        let partial = self.message()?;
        let mut result =
            json!({"type":format!("{prefix}_{stage}"),"contentIndex":index,"partial":partial});
        if let Some(delta) = delta {
            result["delta"] = json!(delta);
        }
        if stage == "end" {
            result[if *kind == BlockKind::Tool {
                "toolCall"
            } else {
                "content"
            }] = match kind {
                BlockKind::Text => partial["content"][*index]["text"].clone(),
                BlockKind::Thinking => partial["content"][*index]["thinking"].clone(),
                BlockKind::Tool => partial["content"][*index].clone(),
            };
        }
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_projection_rejects_malformed_blocks_indices_and_terminal_evidence() {
        let model = json!({"id":"model","provider":"fixture","api":"fixture-api"});
        let message = json!({"role":"assistant","provider":"fixture","model":"model","api":"fixture-api","content":[{"type":"text","text":"hello"}],"usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1});
        let projection = || CustomProjection {
            model: model.clone(),
            provider: "fixture".into(),
            terminal: None,
            events: 0,
            bytes: 0,
            limit: 65536,
        };
        let wrap = |event| json!({"type":"provider_event","provider":"fixture","event":event});
        let delta = json!({"type":"text_delta","contentIndex":0,"delta":"hello","partial":message});
        let terminal = json!({"type":"done","reason":"stop","message":message});
        let mut valid = projection();
        assert_eq!(
            valid.event(&wrap(delta.clone())).unwrap(),
            Some(delta.clone())
        );
        assert_eq!(
            valid.event(&wrap(terminal.clone())).unwrap(),
            Some(terminal.clone())
        );
        assert!(valid.event(&wrap(terminal.clone())).is_err());
        for variant in 0..9 {
            let mut event = delta.clone();
            match variant {
                0 => event["contentIndex"] = json!(2),
                1 => event["contentIndex"] = json!(-1),
                2 => event["delta"] = Value::Null,
                3 => event["partial"]["content"][0]["text"] = json!(3),
                4 => event["partial"]["usage"]["input"] = json!(-1),
                5 => event["partial"]["model"] = json!("foreign"),
                6 => {
                    event = terminal.clone();
                    event["type"] = json!("error");
                    event["error"] = message.clone();
                }
                7 => {
                    event = terminal.clone();
                    event["reason"] = json!("length");
                }
                _ => {
                    event = terminal.clone();
                    event["message"]["stopReason"] = json!("aborted");
                    event["reason"] = json!("aborted");
                }
            }
            assert!(
                projection().event(&wrap(event)).is_err(),
                "variant {variant}"
            );
        }
    }

    #[test]
    fn incomplete_tool_previews_never_replace_strict_final_arguments() {
        use ag1_core::provider::ApiFamily;
        let model = json!({"id":"model","provider":"fixture","cost":{}});
        for family in [
            ApiFamily::Completions,
            ApiFamily::Anthropic,
            ApiFamily::Responses,
            ApiFamily::Bedrock,
            ApiFamily::Mistral,
        ] {
            let mut projection = MessageProjection::new(&model, 1, family, 65536).unwrap();
            let begin = projection
                .apply(&ProviderEvent::Begin {
                    key: "tool".into(),
                    kind: BlockKind::Tool,
                    id: "call".into(),
                    name: "read".into(),
                    arguments: None,
                    redacted: false,
                })
                .unwrap()
                .unwrap();
            assert_eq!(begin["partial"]["content"][0]["arguments"], json!({}));
            let raw = "{\"path\":\"file";
            let delta = projection
                .apply(&ProviderEvent::Delta {
                    key: "tool".into(),
                    text: raw.into(),
                })
                .unwrap()
                .unwrap();
            let preview = &delta["partial"]["content"][0];
            assert_eq!(preview["arguments"], json!({"path":"file"}));
            let field = if matches!(family, ApiFamily::Completions | ApiFamily::Mistral) {
                "partialArgs"
            } else {
                "partialJson"
            };
            assert_eq!(preview[field], raw);
            // The authoritative reducer has never received a guessed argument.
            assert!(projection.reducer.snapshot().content[0].arguments.is_none());
            let end = projection
                .apply(&ProviderEvent::End { key: "tool".into() })
                .unwrap()
                .unwrap();
            assert_eq!(end["toolCall"]["arguments"], json!({}));
            assert_eq!(end["toolCall"]["_argumentsTruncated"], true);
            if family != ApiFamily::Responses {
                assert!(end["toolCall"].get(field).is_none());
            }
            assert!(projection.reducer.snapshot().content[0].malformed_arguments);
        }
    }

    #[test]
    fn assistant_projection_preserves_all_ten_pinned_provider_final_messages() {
        let wire: Value =
            serde_json::from_str(include_str!("../../../../fixtures/providers/wire.json")).unwrap();
        let oracle: Value =
            serde_json::from_str(include_str!("../../../../fixtures/providers/oracle.json"))
                .unwrap();
        for (case, expected) in wire["cases"]
            .as_array()
            .unwrap()
            .iter()
            .zip(oracle["groups"].as_array().unwrap())
        {
            let family = serde_json::from_value(case["api"].clone()).unwrap();
            let mut decoder = crate::provider::WireDecoder::new(family);
            let model = json!({"id":"fixture","provider":expected["result"]["provider"],"cost":{"input":1,"output":2,"cacheRead":0.1,"cacheWrite":1.25}});
            let mut projection = MessageProjection::new(&model, 12, family, 512 * 1024).unwrap();
            for frame in case["frames"].as_array().unwrap() {
                for event in decoder.decode(frame).unwrap() {
                    projection.apply(&event).unwrap();
                }
            }
            let snapshot = projection.reducer.finish(false).unwrap();
            let mut actual = message(&snapshot, &model, 12).unwrap();
            actual.as_object_mut().unwrap().remove("timestamp");
            let mut expected = expected["result"].clone();
            // JS/Rust float evaluation differs below 1e-15; JSON signature envelopes
            // preserve semantic object identity, with opaque signature bytes unchanged.
            for key in ["input", "output", "cacheRead", "cacheWrite", "total"] {
                assert!(
                    (actual["usage"]["cost"][key].as_f64().unwrap()
                        - expected["usage"]["cost"][key].as_f64().unwrap())
                    .abs()
                        < 1e-15
                );
                actual["usage"]["cost"][key] = expected["usage"]["cost"][key].clone();
            }
            for value in [&mut actual, &mut expected] {
                for block in value["content"].as_array_mut().unwrap() {
                    for key in ["textSignature", "thinkingSignature"] {
                        if let Some(text) = block[key].as_str()
                            && text.starts_with('{')
                        {
                            block[key] = serde_json::from_str(text).unwrap();
                        }
                    }
                }
            }
            assert_eq!(actual, expected, "{family:?}");
        }
    }
}
