//! One admitted generation owns its permit through HTTP and domain settlement.
//! An interrupted stream returns its partial result and quarantines via permit Drop.
use super::{
    WireDecoder,
    framing::{AwsEventDecoder, SseDecoder},
    http::{CancelToken, ErrorRedactor, HttpRequest, ProviderHttp},
};
use ag1_core::{DomainError, inference::GenerationPermit, provider::*};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::time::Instant;

pub struct GenerationLimits {
    pub max_output_bytes: usize,
    pub prefill_ms: u64,
    pub idle_ms: u64,
    /// Zero disables the overall watchdog, matching disabled long-reasoning
    /// limits. Cancellation, byte limits and any nonzero watchdog still apply.
    pub overall_ms: u64,
}
pub struct GenerationRequest {
    pub request_id: String,
    pub tool_names: Vec<String>,
    pub family: ApiFamily,
    pub request: HttpRequest,
    pub limits: GenerationLimits,
}
#[derive(Debug)]
pub struct GenerationFailure {
    pub error: DomainError,
    pub partial: Option<ProviderSnapshot>,
}
fn failure(error: DomainError, reducer: Option<&StreamReducer>) -> GenerationFailure {
    GenerationFailure {
        error,
        partial: reducer.map(StreamReducer::snapshot),
    }
}
enum Frames {
    Sse(SseDecoder),
    Aws(AwsEventDecoder),
}
impl Frames {
    fn finish(&self) -> Result<(), DomainError> {
        match self {
            Self::Sse(v) => v.finish(),
            Self::Aws(v) => v.finish(),
        }
    }
}
pub async fn request_generation(
    http: &ProviderHttp,
    generation: GenerationRequest,
    permit: GenerationPermit,
    cancel: &mut CancelToken,
    observer: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
) -> Result<ProviderSnapshot, GenerationFailure> {
    generation_with_admission(
        http,
        generation,
        Admission::Owned(Some(permit)),
        cancel,
        observer,
    )
    .await
}

/// A sequential transport inside an admitted custom provider borrows the actual
/// permit. It cannot release its caller's slot. Incomplete transport settlement
/// latches a fault even if the provider catches the error and returns success.
pub(crate) async fn request_borrowed_generation(
    http: &ProviderHttp,
    generation: GenerationRequest,
    permit: &mut GenerationPermit,
    fault: &AtomicBool,
    cancel: &mut CancelToken,
    observer: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
) -> Result<ProviderSnapshot, GenerationFailure> {
    if fault.load(Ordering::Acquire) {
        return Err(failure(
            DomainError::Unavailable("parent generation requires settlement".into()),
            None,
        ));
    }
    generation_with_admission(
        http,
        generation,
        Admission::Borrowed {
            _permit: permit,
            fault,
            settled: false,
        },
        cancel,
        observer,
    )
    .await
}
enum Admission<'a> {
    Owned(Option<GenerationPermit>),
    Borrowed {
        _permit: &'a mut GenerationPermit,
        fault: &'a AtomicBool,
        settled: bool,
    },
}
impl Admission<'_> {
    fn terminal(mut self) -> Result<(), DomainError> {
        match &mut self {
            Self::Owned(permit) => permit.take().ok_or(DomainError::Conflict)?.terminal(),
            Self::Borrowed { settled, .. } => {
                *settled = true;
                Ok(())
            }
        }
    }
}
impl Drop for Admission<'_> {
    fn drop(&mut self) {
        if let Self::Borrowed {
            fault,
            settled: false,
            ..
        } = self
        {
            fault.store(true, Ordering::Release);
        }
    }
}
async fn generation_with_admission(
    http: &ProviderHttp,
    generation: GenerationRequest,
    permit: Admission<'_>,
    cancel: &mut CancelToken,
    mut observer: impl FnMut(&ProviderEvent) -> Result<(), DomainError>,
) -> Result<ProviderSnapshot, GenerationFailure> {
    let limits = &generation.limits;
    if limits.overall_ms > 86_400_000
        || limits.prefill_ms > 86_400_000
        || limits.idle_ms > 86_400_000
    {
        permit.terminal().map_err(|e| failure(e, None))?;
        return Err(failure(
            DomainError::InvalidInput("generation watchdog limits".into()),
            None,
        ));
    }
    let prepared = (|| {
        http.validate_request(&generation.request)?;
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        Ok((
            StreamReducer::new(generation.family, 0, limits.max_output_bytes)?,
            WireDecoder::for_request(generation.family, &generation.request_id)?
                .with_tool_names(&generation.tool_names)?,
        ))
    })();
    let (mut reducer, mut decoder) = match prepared {
        Ok(value) => value,
        Err(error) => {
            permit.terminal().map_err(|e| failure(e, None))?;
            return Err(failure(error, None));
        }
    };
    if generation.family == ApiFamily::Responses
        && let Ok(body) = serde_json::from_slice::<serde_json::Value>(&generation.request.body)
        && let Some(tier) = body["service_tier"].as_str()
        && let Err(error) = reducer.apply(ProviderEvent::ServiceTier(tier.into()), 0)
    {
        permit.terminal().map_err(|e| failure(e, None))?;
        return Err(failure(error, None));
    }
    if let Err(error) = http
        .prepare_generation(&generation.request, None, cancel)
        .await
    {
        // No provider request was dispatched. Custody cancellation drains its
        // owned load separately; this generation has no uncertain HTTP outcome.
        permit.terminal().map_err(|e| failure(e, None))?;
        return Err(failure(error, None));
    }
    let started = Instant::now();
    let hard_deadline =
        (limits.overall_ms != 0).then(|| started + Duration::from_millis(limits.overall_ms));
    let deadline = |reducer: &StreamReducer| {
        reducer
            .watchdog_deadline_ms(limits.prefill_ms, limits.idle_ms)
            .map(|ms| started + Duration::from_millis(ms))
            .into_iter()
            .chain(hard_deadline)
            .min()
    };
    let redactor = ErrorRedactor::new(&generation.request);
    let mut response = http
        .send_until(generation.request, deadline(&reducer), cancel)
        .await
        .map_err(|e| failure(e, Some(&reducer)))?;
    if !(200..300).contains(&response.status()) {
        let status = response.status();
        let mut body = vec![];
        while let Some(bytes) = response
            .chunk_until(hard_deadline, cancel)
            .await
            .map_err(|e| failure(e, Some(&reducer)))?
        {
            if body.len() < 4096 {
                body.extend_from_slice(&bytes[..bytes.len().min(4096 - body.len())]);
            }
        }
        let event = ProviderEvent::Fail(redactor.redact(format!(
            "Provider HTTP {status}: {}",
            String::from_utf8_lossy(&body)
        )));
        reducer
            .apply(event.clone(), started.elapsed().as_millis() as u64)
            .map_err(|e| failure(e, Some(&reducer)))?;
        observer(&event).map_err(|e| failure(e, Some(&reducer)))?;
        let result = reducer
            .finish(false)
            .map_err(|e| failure(e, Some(&reducer)))?;
        permit.terminal().map_err(|e| failure(e, Some(&reducer)))?;
        return Ok(result);
    }
    let mut frames = if generation.family == ApiFamily::Bedrock {
        if response
            .content_type()
            .is_none_or(|v| !v.starts_with("application/vnd.amazon.eventstream"))
        {
            return Err(failure(
                DomainError::InvalidInput("AWS stream content type".into()),
                Some(&reducer),
            ));
        }
        Frames::Aws(AwsEventDecoder::new(2 * 1024 * 1024).map_err(|e| failure(e, Some(&reducer)))?)
    } else {
        if response
            .content_type()
            .is_none_or(|v| !v.starts_with("text/event-stream"))
        {
            return Err(failure(
                DomainError::InvalidInput("provider SSE content type".into()),
                Some(&reducer),
            ));
        }
        Frames::Sse(SseDecoder::new(2 * 1024 * 1024).map_err(|e| failure(e, Some(&reducer)))?)
    };
    while let Some(bytes) = response
        .chunk_until(deadline(&reducer), cancel)
        .await
        .map_err(|e| failure(e, Some(&reducer)))?
    {
        for chunk in bytes.chunks(65536) {
            let mut events = vec![];
            match &mut frames {
                Frames::Sse(sse) => {
                    for frame in sse.feed(chunk).map_err(|e| failure(e, Some(&reducer)))? {
                        if frame.data == "[DONE]" {
                            if reducer.stop_reason().is_none()
                                && !events.iter().any(|e| {
                                    matches!(e, ProviderEvent::Stop(_) | ProviderEvent::Fail(_))
                                })
                            {
                                events.push(ProviderEvent::Stop(StopReason::Stop));
                            }
                        } else {
                            events.extend(
                                decoder
                                    .decode_json(frame.data.as_bytes())
                                    .map_err(|e| failure(e, Some(&reducer)))?,
                            );
                        }
                    }
                }
                Frames::Aws(aws) => {
                    for message in aws.feed(chunk).map_err(|e| failure(e, Some(&reducer)))? {
                        events.extend(
                            decoder
                                .decode(
                                    &message
                                        .json_event()
                                        .map_err(|e| failure(e, Some(&reducer)))?,
                                )
                                .map_err(|e| failure(e, Some(&reducer)))?,
                        );
                    }
                }
            }
            for mut event in events {
                if let ProviderEvent::Fail(message) = &mut event {
                    *message = redactor.redact(std::mem::take(message));
                }
                reducer
                    .apply(event.clone(), started.elapsed().as_millis() as u64)
                    .map_err(|e| failure(e, Some(&reducer)))?;
                observer(&event).map_err(|e| failure(e, Some(&reducer)))?;
            }
        }
    }
    frames.finish().map_err(|e| failure(e, Some(&reducer)))?;
    if reducer.stop_reason().is_none() {
        return Err(failure(
            DomainError::Unavailable("provider EOF without terminal evidence".into()),
            Some(&reducer),
        ));
    }
    let result = reducer
        .finish(false)
        .map_err(|e| failure(e, Some(&reducer)))?;
    permit.terminal().map_err(|e| failure(e, Some(&reducer)))?;
    Ok(result)
}

/// Execute an already-built isolated request with exactly one existing permit.
/// The caller uses ag1_core::isolated::context, disables cache retention and passes
/// no tools. This function neither acquires a nested permit nor persists output.
pub async fn request_isolated_generation(
    http: &ProviderHttp,
    generation: GenerationRequest,
    permit: GenerationPermit,
    cancel: &mut CancelToken,
    allow_truncated: bool,
) -> Result<(String, ProviderSnapshot), GenerationFailure> {
    let validation = (|| {
        if !generation.tool_names.is_empty() {
            return Err(DomainError::InvalidInput(
                "isolated request declares tools".into(),
            ));
        }
        let body: serde_json::Value = serde_json::from_slice(&generation.request.body)
            .map_err(|_| DomainError::InvalidInput("isolated request JSON".into()))?;
        let body = if generation.family == ApiFamily::GeminiCli {
            &body["request"]
        } else {
            &body
        };
        if body.get("tools").is_some() || body.get("toolConfig").is_some() {
            return Err(DomainError::InvalidInput(
                "isolated request tool definitions".into(),
            ));
        }
        let messages = body
            .get("messages")
            .or_else(|| body.get("input"))
            .or_else(|| body.get("contents"))
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| DomainError::InvalidInput("isolated request messages".into()))?;
        let conversation: Vec<_> = messages
            .iter()
            .filter(|m| m["role"] != "system" && m["role"] != "developer")
            .collect();
        if conversation.len() != 1 || conversation[0]["role"] != "user" {
            return Err(DomainError::InvalidInput(
                "isolated request contains conversation history".into(),
            ));
        }
        Ok(())
    })();
    if let Err(error) = validation {
        permit.terminal().map_err(|e| failure(e, None))?;
        return Err(failure(error, None));
    }
    let snapshot = request_generation(http, generation, permit, cancel, |_| Ok(())).await?;
    let text = ag1_core::isolated::accepted_text(&snapshot, allow_truncated).map_err(|error| {
        GenerationFailure {
            error,
            partial: Some(snapshot.clone()),
        }
    })?;
    Ok((text, snapshot))
}
/// NVIDIA's one-token nonstreaming preflight is inference too. It consumes one
/// permit and releases it only after the entire response body has ended.
pub async fn nvidia_preflight(
    http: &ProviderHttp,
    mut request: HttpRequest,
    model: &str,
    permit: GenerationPermit,
    cancel: &mut CancelToken,
) -> Result<u16, DomainError> {
    request.body=serde_json::to_vec(&serde_json::json!({"model":model,"messages":[{"role":"user","content":"ping"}],"max_completion_tokens":1,"stream":false})).map_err(|_|DomainError::InvalidInput("preflight JSON".into()))?;
    request.max_response_bytes = 1024 * 1024;
    if let Err(error) = http.validate_request(&request).and_then(|()| {
        if cancel.cancelled() {
            Err(DomainError::Cancelled)
        } else {
            Ok(())
        }
    }) {
        permit.terminal()?;
        return Err(error);
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    if let Err(error) = http
        .prepare_generation(&request, Some(deadline), cancel)
        .await
    {
        permit.terminal()?;
        return Err(error);
    }
    let mut response = http.send(request, deadline, cancel).await?;
    let status = response.status();
    while response.chunk(deadline, cancel).await?.is_some() {}
    permit.terminal()?;
    Ok(status)
}
