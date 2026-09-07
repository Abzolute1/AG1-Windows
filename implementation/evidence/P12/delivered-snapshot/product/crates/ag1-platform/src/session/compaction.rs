//! Durable same-model compaction at the native session boundary. Summaries are
//! isolated scheduled requests, and original custody precedes their projections.
use super::recovery::RecoveryLease;
use super::{ActiveTurn, Listener, NEXT_TURN, NativeSession, ProviderInput, ProviderResolver};
use crate::provider::{
    generation::request_isolated_generation,
    http::{CancelToken, cancellation},
};
use ag1_core::runtime::recovery::Owner;
use ag1_core::{
    DomainError, OperationId,
    context::{
        ContextArtifactPort,
        compaction::{self, Settings},
        compiler::Compilation,
        features::Features,
        retry,
        summary_request::{self, SummaryRequest},
    },
    runtime::session::CompactionCapture,
};
use ag1_storage::context::ContextStore;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Manual,
    Threshold,
    Overflow,
}
pub struct CompactInput {
    pub command: String,
    pub timestamp: String,
    pub settings: Settings,
    pub focus: Option<String>,
    pub reason: Reason,
    /// The reference's summary-specific bound defaults to 900000ms. This never
    /// changes the parent model's reasoning watchdog configuration.
    pub llm_timeout_ms: u64,
}
pub struct CompactResult {
    pub result: Value,
    pub context_epoch: u64,
    pub journal_revision: u64,
    pub(crate) stamp: ag1_core::runtime::boundary::Stamp,
}
pub(super) struct CompactLease<'a> {
    pub(super) session: &'a NativeSession,
    pub(super) token: ag1_core::runtime::boundary::Compaction,
}
impl Drop for CompactLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.session.compacting.lock()
            && let Some(work) = active.take()
        {
            work.cancelled.store(true, Ordering::Release);
            work.source.cancel();
        }
        // Preserve an accepted journal commit's token until its acknowledgement.
        // Unsent summaries release only their reservation; workers, transport,
        // parent generations and tool effects retain independent barriers.
        if let Ok(mut state) = self.session.state.lock()
            && !state.pending_commit()
        {
            let _ = state.abandon_compaction(&self.token);
        }
        self.session.changed.notify_waiters();
    }
}
struct SummaryControl<'a> {
    cancelled: &'a AtomicBool,
    cancel: &'a mut CancelToken,
    deadline: Instant,
    operation: &'a str,
    outputs: Vec<ag1_core::context::ContextArtifact>,
}
struct SummaryPart<'a> {
    capture: &'a CompactionCapture,
    input: &'a CompactInput,
    preparation: &'a Value,
    split: bool,
}
impl NativeSession {
    /// Manual compaction reserves before interrupting the parent stream. If the
    /// endpoint cannot prove termination, the shared scheduler keeps the summary
    /// queued until the host supervisor reconciles the original ticket.
    pub async fn compact(
        &self,
        input: CompactInput,
        resolver: &dyn ProviderResolver,
        listener: &dyn Listener,
    ) -> Result<Option<CompactResult>, DomainError> {
        self.compact_inner(input, resolver, None, None, listener)
            .await
    }
    /// Compose the client's admitted extension runner with manual or automatic
    /// compaction. The host supplies the same capabilities used for its turns.
    pub async fn compact_with_callbacks(
        &self,
        input: CompactInput,
        resolver: &dyn ProviderResolver,
        tools: Arc<dyn super::ToolRunner>,
        listener: &dyn Listener,
    ) -> Result<Option<CompactResult>, DomainError> {
        self.compact_inner(input, resolver, Some(tools), None, listener)
            .await
    }
    pub(super) async fn compact_for_command(
        &self,
        input: CompactInput,
        resolver: &dyn ProviderResolver,
        tools: Arc<dyn super::ToolRunner>,
        cancel: Arc<AtomicBool>,
        listener: &dyn Listener,
    ) -> Result<Option<CompactResult>, DomainError> {
        self.compact_inner(input, resolver, Some(tools), Some(cancel), listener)
            .await
    }
    async fn compact_inner(
        &self,
        input: CompactInput,
        resolver: &dyn ProviderResolver,
        tools: Option<Arc<dyn super::ToolRunner>>,
        command_cancel: Option<Arc<AtomicBool>>,
        listener: &dyn Listener,
    ) -> Result<Option<CompactResult>, DomainError> {
        let _activity = self.enter()?;
        OperationId::new(&input.command)?;
        if input.llm_timeout_ms == 0
            || input.llm_timeout_ms > 86_400_000
            || input
                .focus
                .as_ref()
                .is_some_and(|focus| focus.len() > 1024 * 1024)
        {
            return Err(DomainError::InvalidInput("compaction options".into()));
        }
        let (source, mut cancel) = cancellation();
        let cancelled = command_cancel.unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let parent_idle = self
            .active
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_none();
        let token = {
            let mut compacting = self.compacting.lock().map_err(|_| DomainError::Conflict)?;
            if compacting.is_some() {
                return Err(DomainError::Conflict);
            }
            let mut state = self.state()?;
            if parent_idle && state.settled() {
                state.next_turn()?;
            }
            let token = state.reserve_compaction()?;
            let mut recovery = self.recovery.lock().map_err(|_| DomainError::Conflict)?;
            if input.reason == Reason::Manual {
                recovery.clear(Some(Owner::Retry));
            }
            if !recovery.try_arm(Owner::Compaction) {
                state.abandon_compaction(&token)?;
                return Err(DomainError::Conflict);
            }
            *compacting = Some(ActiveTurn {
                cancelled: cancelled.clone(),
                source: Arc::new(source),
            });
            token
        };
        let _lease = CompactLease {
            session: self,
            token: token.clone(),
        };
        let _recovery = RecoveryLease {
            session: self,
            owner: Owner::Compaction,
        };
        if input.reason == Reason::Manual
            && let Some(parent) = self
                .active
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .as_ref()
        {
            // Stop the parent loop as well as its transport. Its accepted work
            // still drains, using the separate summary cancellation source.
            parent.cancelled.store(true, Ordering::Release);
            parent.source.cancel();
            self.changed.notify_waiters();
        }
        let serial = NEXT_TURN
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| DomainError::Conflict)?;
        let operation = format!("compact-{}-{serial}", self.incarnation);
        let work = async {
            let capture = loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if cancelled.load(Ordering::Acquire) {
                    return Err(DomainError::Cancelled);
                }
                let captured = self.state()?.capture_compaction(&token);
                match captured {
                    Ok(capture) => break capture,
                    Err(DomainError::Unavailable(_)) => changed.await,
                    Err(error) => return Err(error),
                }
            };
            let branch = capture.branch().to_vec();
            let messages = compaction::context_messages(&branch);
            let store = self.store.clone();
            let session = self.state()?.stamp().session().clone();
            let origin = self.origin.clone();
            let op = operation.clone();
            let timestamp = input.timestamp.clone();
            let revision = capture.journal_revision();
            let settings = input.settings.clone();
            let activity=self.retain_work()?;
            let (preparation, original) = self
                .pool
                .submit(move || {
                    let _activity=activity;
                    let mut context = ContextStore::new(
                        &store,
                        session,
                        &origin,
                        &op,
                        &timestamp,
                        Features::default(),
                    )?;
                    let original = context.project(&messages, revision, |messages, _| {
                        Ok(Compilation {
                            messages: messages.to_vec(),
                            stats: json!({}),
                        })
                    })?;
                    let prepared = compaction::prepare_full(&branch, &settings)?.map(|preparation| {
                        let envelope = json!({"version":1,"kind":"compaction-trajectory","createdAt":timestamp,
                            "tokensBefore":preparation["tokensBefore"],"firstKeptEntryId":preparation["firstKeptEntryId"],
                            "isSplitTurn":preparation["isSplitTurn"],"previousSummaryIncluded":preparation["previousSummary"].as_str().is_some_and(|s| !s.is_empty()),
                            "messagesToSummarize":preparation["messagesToSummarize"],"turnPrefixMessages":preparation["turnPrefixMessages"]});
                        let trajectory = context.put_evidence(&format!("{envelope}\n"), "compaction-trajectory",
                            &json!({"type":"session","name":"semantic-compaction"}), &[], None)?;
                        Ok::<_, DomainError>((preparation, trajectory))
                    }).transpose()?;
                    Ok((prepared, original))
                })
                .await?;
            let Some((preparation, trajectory)) = preparation else {
                self.state()?.abandon_compaction(&token)?;
                return Ok(None);
            };
            let mut override_result=None;
            if let Some(tools)=&tools && tools.handles_session_event("session_before_compact") {
                let mut event=json!({"type":"session_before_compact","preparation":preparation,"branchEntries":capture.branch()});
                if let Some(focus)=&input.focus {event["customInstructions"]=json!(focus);}
                let result=self.request_callback(tools.clone(),event,cancelled.clone(),&format!("{operation}-before"),&input.timestamp).await?;
                if let Some(result)=result {
                    if result["cancel"]==true {
                        self.state()?.abandon_compaction(&token)?;
                        return if input.reason==Reason::Manual {Err(DomainError::Cancelled)} else {Ok(None)};
                    }
                    override_result=result.get("compaction").cloned().filter(|value|!value.is_null());
                }
            }
            let from_extension=override_result.is_some();
            let deadline = Instant::now() + Duration::from_millis(input.llm_timeout_ms);
            let mut control = SummaryControl {
                cancelled: &cancelled,
                cancel: &mut cancel,
                deadline,
                operation: &operation,
                outputs: vec![],
            };
            let semantic = async {
            if let Some(result)=override_result {return Ok(result);}
            let split = preparation["isSplitTurn"] == true
                && preparation["turnPrefixMessages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty());
            let history = if split
                && preparation["messagesToSummarize"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            {
                "No prior history.".into()
            } else {
                self
                    .summary_part(
                        SummaryPart {
                            capture: &capture,
                            input: &input,
                            preparation: &preparation,
                            split: false,
                        },
                        resolver,
                        &mut control,
                    )
                    .await?
            };
            let prefix = if split {
                let text = self
                    .summary_part(
                        SummaryPart {
                            capture: &capture,
                            input: &input,
                            preparation: &preparation,
                            split: true,
                        },
                        resolver,
                        &mut control,
                    )
                    .await?;
                Some(text)
            } else {
                None
            };
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            compaction::finish_summary(
                &preparation,
                &history,
                prefix.as_deref(),
                None,
            )
            }.await;
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            let mut result = match semantic {
                Ok(result) => result,
                Err(error @ (DomainError::TimedOut | DomainError::Unavailable(_))) if input.reason != Reason::Manual => {
                    self.state()?.check_compaction_capture(&capture)?;
                    compaction::emergency_checkpoint(&preparation, &format!("{error:?}"))?
                }
                Err(error) => return Err(error),
            };
            compaction::attach_provenance(&mut result, &trajectory)?;
            if input.reason != Reason::Manual && result["details"]["method"] != "persistent_emergency_checkpoint" {
                compaction::guard_summary(&mut result, capture.configuration().context_limit())?;
            }
            result["details"]["originalJournal"] = json!(original.original_journal);
            result["details"]["originalRequest"] = json!(original.original_request);
            result["details"]["summaryOutputs"] = json!(control.outputs);
            let append = self.state()?.append_compaction(
                &capture,
                &input.command,
                &operation,
                &input.timestamp,
                result.clone(),
            )?;
            let revision = append.write().expected.checked_add(1).ok_or(DomainError::Conflict)?;
            let stamp = self.commit_stamped(append, listener).await?;
            if let Some(tools)=&tools && tools.handles_session_event("session_compact") {
                let notification=async {
                    let entry=self.state()?.branch().iter().find(|entry|entry["id"]==operation).cloned().ok_or(DomainError::Conflict)?;
                    let event=json!({"type":"session_compact","compactionEntry":entry,"fromExtension":from_extension});
                    self.request_callback(tools.clone(),event,cancelled.clone(),&format!("{operation}-after"),&input.timestamp).await
                }.await;
                if let Err(error)=notification {
                    // A durable compaction remains successful if a notification
                    // fails. Never trigger emergency compaction or replay it.
                    NativeSession::emit(listener,super::SessionEvent::Recovery(json!({"type":"extension_notification_error","event":"session_compact","error":format!("{error:?}")})));
                }
            }
            Ok(Some(CompactResult {
                result,
                context_epoch: stamp.epoch(),
                journal_revision: revision,
                stamp,
            }))
        }
        .await;
        if work.is_err() {
            // A failed/lost persistence receipt may keep other barriers closed;
            // in that case retain the reservation for explicit reconciliation.
            let _ = self.state()?.abandon_compaction(&token);
        }
        self.changed.notify_waiters();
        work
    }

    async fn summary_part(
        &self,
        part: SummaryPart<'_>,
        resolver: &dyn ProviderResolver,
        control: &mut SummaryControl<'_>,
    ) -> Result<String, DomainError> {
        let SummaryPart {
            capture,
            input,
            preparation,
            split,
        } = part;
        let key = if split {
            "turnPrefixMessages"
        } else {
            "messagesToSummarize"
        };
        let mut messages = preparation[key]
            .as_array()
            .ok_or_else(|| DomainError::InvalidInput("summary messages".into()))?
            .clone();
        for attempt in 0..=retry::MAX_RETRIES {
            if control.cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            let operation = format!(
                "{}-{}-{attempt}",
                control.operation,
                if split { "prefix" } else { "history" }
            );
            let session = self.state()?.stamp().session().clone();
            let request = summary_request::request(SummaryRequest {
                messages: &messages,
                reserve_tokens: input.settings.reserve_tokens,
                previous: preparation["previousSummary"].as_str(),
                focus: input.focus.as_deref(),
                memory: Some(&preparation["workingMemory"]),
                split_prefix: split,
                root_session: Some(session.as_str()),
                model_max: capture.configuration().model["maxTokens"]
                    .as_u64()
                    .unwrap_or(0),
            })?;
            let mut configuration = capture.configuration().clone();
            configuration.tools.clear();
            configuration.thinking = "off".into();
            configuration.system_prompt = request.context["systemPrompt"].as_str().unwrap().into();
            let projection = Compilation {
                messages: request.context["messages"].as_array().unwrap().clone(),
                stats: json!({}),
            };
            let options = json!({"maxTokens":request.max_tokens,"sessionId":request.session_id,"cacheRetention":"none"});
            let mut resolved = tokio::time::timeout_at(
                control.deadline,
                resolver.resolve(
                    ProviderInput {
                        configuration: &configuration,
                        projection: &projection,
                        operation: &operation,
                        options: Some(&options),
                    },
                    control.cancelled,
                ),
            )
            .await
            .map_err(|_| DomainError::TimedOut)??;
            if resolved.model != capture.configuration().model {
                return Err(DomainError::Conflict);
            }
            let (ticket, permit) = tokio::time::timeout_at(
                control.deadline,
                self.permit(&resolved.endpoint, &operation, control.cancelled),
            )
            .await
            .map_err(|_| DomainError::TimedOut)??;
            if let Err(error) = self.state()?.check_compaction_capture(capture) {
                permit.terminal()?;
                return Err(error);
            }
            let remaining = control
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
            let outcome = request_isolated_generation(
                &self.http,
                resolved.generation,
                permit,
                control.cancel,
                true,
            )
            .await;
            let snapshot = match &outcome {
                Ok((_, snapshot)) => Some(snapshot),
                Err(failure) => failure.partial.as_ref(),
            };
            if let Some(snapshot) = snapshot {
                let snapshot = snapshot.clone();
                let store = self.store.clone();
                let origin = self.origin.clone();
                let timestamp = input.timestamp.clone();
                let activity = self.retain_work()?;
                let artifact = self
                    .pool
                    .submit(move || {
                        let _activity = activity;
                        let mut context = ContextStore::new(
                            &store,
                            session,
                            &origin,
                            &operation,
                            &timestamp,
                            Features::default(),
                        )?;
                        context.put_evidence(
                            &json!(snapshot).to_string(),
                            "compaction-summary-output",
                            &json!({"type":"system","name":"compaction-summary"}),
                            &[],
                            None,
                        )
                    })
                    .await?;
                control.outputs.push(artifact);
            }
            match outcome {
                Ok((text, _)) => return Ok(text),
                Err(failure) => {
                    let diagnostic = failure
                        .partial
                        .as_ref()
                        .and_then(|snapshot| snapshot.error.clone())
                        .unwrap_or_else(|| format!("{:?}", failure.error));
                    if attempt == retry::MAX_RETRIES
                        || !retry::is_prompt_too_long(&diagnostic)
                        || self.pending_transport()?.is_some()
                    {
                        return Err(failure.error);
                    }
                    let Some(reduced) = retry::head(&messages, &diagnostic, resolved.timestamp_ms)?
                    else {
                        return Err(failure.error);
                    };
                    messages = reduced["messages"].as_array().unwrap().clone();
                }
            }
        }
        unreachable!("bounded summary retry loop always returns")
    }
}
