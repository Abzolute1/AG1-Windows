use super::*;
use crate::provider::{generation::request_isolated_generation, http::CancelToken};
use ag1_core::{context::ContextArtifact, isolated::IsolatedContext};
use tokio::time::Instant;
pub(super) struct Request<'a> {
    pub capture: &'a CompactionCapture,
    pub input: &'a TreeInput,
    pub operation: &'a str,
    pub request: IsolatedContext,
    pub cancelled: &'a AtomicBool,
    pub cancel: &'a mut CancelToken,
}
impl NativeSession {
    pub(super) async fn tree_summary(
        &self,
        request: Request<'_>,
        resolver: &dyn ProviderResolver,
    ) -> Result<(String, Option<ContextArtifact>), DomainError> {
        let Request {
            capture,
            input,
            operation,
            request,
            cancelled,
            cancel,
        } = request;
        let deadline = Instant::now() + Duration::from_millis(input.summary_timeout_ms);
        let operation = format!("{operation}-summary");
        let mut configuration = capture.configuration().clone();
        configuration.tools.clear();
        configuration.thinking = "off".into();
        configuration.system_prompt = request.context["systemPrompt"].as_str().unwrap().into();
        let projection = Compilation {
            messages: request.context["messages"].as_array().unwrap().clone(),
            stats: json!({}),
        };
        let estimate = configuration.fixed_tokens()
            + projection
                .messages
                .iter()
                .map(ag1_core::context::estimate_tokens)
                .sum::<u64>();
        if estimate.saturating_mul(11).div_ceil(10)
            > configuration.model["contextWindow"].as_u64().unwrap_or(0)
        {
            return Err(DomainError::LimitExceeded(
                "branch summary context admission margin".into(),
            ));
        }
        let options = json!({"maxTokens":request.max_tokens,"sessionId":request.session_id,"cacheRetention":"none"});
        let mut resolved = tokio::time::timeout_at(
            deadline,
            resolver.resolve(
                ProviderInput {
                    configuration: &configuration,
                    projection: &projection,
                    operation: &operation,
                    options: Some(&options),
                },
                cancelled,
            ),
        )
        .await
        .map_err(|_| DomainError::TimedOut)??;
        if resolved.model != capture.configuration().model {
            return Err(DomainError::Conflict);
        }
        let (ticket, permit) = tokio::time::timeout_at(
            deadline,
            self.permit(&resolved.endpoint, &operation, cancelled),
        )
        .await
        .map_err(|_| DomainError::TimedOut)??;
        if let Err(error) = self.check_tree_capture(capture) {
            permit.terminal()?;
            return Err(error);
        }
        let remaining = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if remaining == 0 || cancelled.load(Ordering::Acquire) {
            permit.terminal()?;
            return Err(if remaining == 0 {
                DomainError::TimedOut
            } else {
                DomainError::Cancelled
            });
        }
        resolved.generation.limits.overall_ms = if resolved.generation.limits.overall_ms == 0 {
            remaining
        } else {
            remaining.min(resolved.generation.limits.overall_ms)
        };
        *self.transport.lock().map_err(|_| DomainError::Conflict)? = Some(ticket);
        let outcome =
            request_isolated_generation(&self.http, resolved.generation, permit, cancel, true)
                .await;
        let snapshot = match &outcome {
            Ok((_, snapshot)) => Some(snapshot.clone()),
            Err(failure) => failure.partial.clone(),
        };
        let output = if let Some(snapshot) = snapshot {
            let activity = self.retain_work()?;
            let store = self.store.clone();
            let session = self.state()?.stamp().session().clone();
            let origin = self.origin.clone();
            let timestamp = input.timestamp.clone();
            Some(
                self.pool
                    .submit(move || {
                        let _activity = activity;
                        let mut artifacts = ContextStore::new(
                            &store,
                            session,
                            &origin,
                            &operation,
                            &timestamp,
                            Default::default(),
                        )?;
                        artifacts.put_evidence(
                            &json!(snapshot).to_string(),
                            "runtime-branch-summary-output",
                            &json!({"type":"session","name":"branch-summary"}),
                            &[],
                            None,
                        )
                    })
                    .await?,
            )
        } else {
            None
        };
        outcome
            .map(|(text, _)| (text, output))
            .map_err(|failure| failure.error)
    }
}
