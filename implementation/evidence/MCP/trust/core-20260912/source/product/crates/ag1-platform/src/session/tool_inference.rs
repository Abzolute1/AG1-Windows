//! A blocking tool asks its owning async session to perform isolated inference.
//! No nested blocking-pool work: the worker retains artifacts before and after
//! the response, while the host alone owns resolver, scheduler and transport.
use super::{NativeSession, ProviderInput, ProviderResolver, tool_journal::ToolJournal};
use crate::provider::{generation::request_isolated_generation, http::cancellation};
use ag1_core::{
    DomainError,
    context::compiler::Compilation,
    isolated::IsolatedContext,
    provider::ProviderSnapshot,
    tools::registry::{Scope, ValidatedCall},
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub struct Output {
    pub text: Result<String, DomainError>,
    pub snapshot: Option<ProviderSnapshot>,
    pub settled: bool,
    custody: Option<tokio::sync::oneshot::Sender<()>>,
}
impl Output {
    pub fn acknowledge_custody(&mut self) {
        if let Some(custody) = self.custody.take() {
            let _ = custody.send(());
        }
    }
}
pub(super) struct Request {
    scope: Scope,
    call: ValidatedCall,
    context: IsolatedContext,
    deadline: Instant,
    reply: mpsc::SyncSender<Result<Output, DomainError>>,
}
impl ToolJournal {
    pub fn draft_generation(
        &self,
        scope: &Scope,
        call: &ValidatedCall,
        context: &IsolatedContext,
        deadline: Instant,
    ) -> Result<Output, DomainError> {
        self.validate_draft(scope, call)?;
        let (reply, receive) = mpsc::sync_channel(1);
        self.isolated
            .send(Request {
                scope: scope.clone(),
                call: call.clone(),
                context: context.clone(),
                deadline,
                reply,
            })
            .map_err(|_| DomainError::Cancelled)?;
        // Accepted work must drain even when cancellation or the deadline fires.
        // Dropping the host request closes the channel instead of stranding a worker.
        loop {
            self.service_controls()?;
            match receive.recv_timeout(Duration::from_millis(10)) {
                Ok(output) => return output,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(DomainError::Unavailable(
                        "isolated host dropped before settlement".into(),
                    ));
                }
            }
        }
    }
}
impl NativeSession {
    pub(super) async fn run_tool_inference(
        &self,
        journal: &Arc<ToolJournal>,
        request: Request,
        resolver: &dyn ProviderResolver,
        parent_cancel: &AtomicBool,
    ) {
        let output = self
            .tool_inference(journal, &request, resolver, parent_cancel)
            .await;
        let mut output = match output {
            Ok(output) => Ok(output),
            Err(error)
                if self
                    .pending_transport()
                    .is_ok_and(|pending| pending.is_none()) =>
            {
                Ok(Output {
                    text: Err(error),
                    snapshot: None,
                    settled: true,
                    custody: None,
                })
            }
            Err(error) => Err(error),
        };
        // A successful buffered send is not proof that the worker received or
        // durably stored the snapshot. Keep custody until explicit acknowledgement.
        let snapshot = output
            .as_ref()
            .ok()
            .and_then(|output| output.snapshot.clone());
        let (acknowledge, acknowledged) = tokio::sync::oneshot::channel();
        if let Ok(output) = &mut output {
            output.custody = Some(acknowledge);
        }
        let _ = request.reply.send(output);
        if let Some(snapshot) = snapshot
            && acknowledged.await.is_err()
        {
            // A control-commit failure or worker panic can retire the receiver
            // while transport is draining. Once it has retired, the same pool
            // can retain the returned bytes without waiting on the draft worker.
            use ag1_core::context::ContextArtifactPort;
            let store = self.store.clone();
            let origin = self.origin.clone();
            let operation = format!("{}-orphaned-draft", request.call.operation());
            let timestamp = journal.timestamp().to_owned();
            let session = request.scope.session;
            let retained = self
                .pool
                .submit(move || {
                    let mut artifacts = ag1_storage::context::ContextStore::new(
                        &store,
                        ag1_core::SessionId::new(session)?,
                        &origin,
                        &operation,
                        &timestamp,
                        Default::default(),
                    )?;
                    artifacts.put_evidence(
                        &json!(snapshot).to_string(),
                        "episode-draft-output",
                        &json!({"type":"system","name":"episode-draft-recovery"}),
                        &[],
                        None,
                    )
                })
                .await;
            if retained.is_err()
                && let Ok(mut state) = self.state()
            {
                state.fail_persistence();
            }
        }
    }
    async fn tool_inference(
        &self,
        journal: &ToolJournal,
        request: &Request,
        resolver: &dyn ProviderResolver,
        parent_cancel: &AtomicBool,
    ) -> Result<Output, DomainError> {
        let check = || {
            if parent_cancel.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if Instant::now() >= request.deadline {
                return Err(DomainError::TimedOut);
            }
            journal.validate_draft(&request.scope, &request.call)
        };
        check()?;
        if self.pending_transport()?.is_some() {
            return Err(DomainError::Conflict);
        }
        let (mut configuration, model_scope) = journal.model(&request.scope)?;
        let model = configuration.model.clone();
        configuration.tools.clear();
        configuration.thinking = "off".into();
        configuration.system_prompt = request.context.context["systemPrompt"]
            .as_str()
            .ok_or(DomainError::Conflict)?
            .into();
        let projection = Compilation {
            messages: request.context.context["messages"]
                .as_array()
                .ok_or(DomainError::Conflict)?
                .clone(),
            stats: json!({}),
        };
        let options = json!({"maxTokens":request.context.max_tokens,
            "sessionId":request.context.session_id,"cacheRetention":"none"});
        let operation = format!("{}-draft", request.call.operation());
        // Resolution and queueing have no admitted external effects and can be
        // dropped immediately when the selected configuration becomes stale.
        let preparation = async {
            let resolved = resolver
                .resolve(
                    ProviderInput {
                        configuration: &configuration,
                        projection: &projection,
                        operation: &operation,
                        options: Some(&options),
                    },
                    parent_cancel,
                )
                .await?;
            if resolved.model != model || resolved.endpoint != model_scope.endpoint {
                return Err(DomainError::Conflict);
            }
            let (ticket, permit) = self
                .permit(&resolved.endpoint, &operation, parent_cancel)
                .await?;
            Ok((resolved, ticket, permit))
        };
        tokio::pin!(preparation);
        let (mut resolved, ticket, permit) = loop {
            tokio::select! {
                output = &mut preparation => break output?,
                _ = tokio::time::sleep(Duration::from_millis(10)) => check()?,
            }
        };
        if let Err(error) = check() {
            permit.terminal()?;
            return Err(error);
        }
        let remaining = request
            .deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as u64;
        if remaining == 0 {
            permit.terminal()?;
            return Err(DomainError::TimedOut);
        }
        resolved.generation.limits.overall_ms = if resolved.generation.limits.overall_ms == 0 {
            remaining
        } else {
            remaining.min(resolved.generation.limits.overall_ms)
        };
        *self.transport.lock().map_err(|_| DomainError::Conflict)? = Some(ticket);
        let (source, mut cancel) = cancellation();
        let generation = request_isolated_generation(
            &self.http,
            resolved.generation,
            permit,
            &mut cancel,
            false,
        );
        tokio::pin!(generation);
        let mut invalid = None;
        let result = loop {
            tokio::select! {
                output = &mut generation => break output,
                _ = tokio::time::sleep(Duration::from_millis(10)), if invalid.is_none() => {
                    if let Err(error) = check() { invalid = Some(error); source.cancel(); }
                }
            }
        };
        let (text, snapshot) = match result {
            Ok((text, snapshot)) => (Ok(text), Some(snapshot)),
            Err(failure) => (Err(failure.error), failure.partial),
        };
        // Return partial output for custody even if configuration changed during
        // generation. It cannot become an accepted candidate in that case.
        let text = match invalid.or_else(|| check().err()) {
            Some(error) => Err(error),
            None => text,
        };
        Ok(Output {
            text,
            snapshot,
            settled: self.pending_transport()?.is_none(),
            custody: None,
        })
    }
}
