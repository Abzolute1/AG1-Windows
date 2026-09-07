//! Headless print-mode recovery using native terminal futures and acknowledged
//! history. Output rendering, full client hooks and typed patch capture live at
//! their separate boundaries; no idle timer pretends a generation has finished.
use super::{
    Listener, NativeSession, ProviderResolver, SessionEvent, ToolRunner, TurnInput, TurnResult,
};
use ag1_core::{
    DomainError,
    runtime::completion::{self, Assessment, VerificationAuthority},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct ModeLease<'a> {
    host: &'a NativeSession,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) verification_command: Option<String>,
    pub(super) controlled: Option<super::controlled::Shared>,
    pub(super) completion: super::completion_evidence::Shared,
    pub(super) planning: Option<super::planning::Turn>,
}
impl<'a> ModeLease<'a> {
    pub(super) fn begin(
        host: &'a NativeSession,
        verification_command: Option<String>,
    ) -> Result<Self, DomainError> {
        let active = host.active.lock().map_err(|_| DomainError::Conflict)?;
        let mut mode = host.mode.lock().map_err(|_| DomainError::Conflict)?;
        if active.is_some() || mode.is_some() {
            return Err(DomainError::Conflict);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        *mode = Some(cancelled.clone());
        Ok(Self {
            host,
            cancelled,
            verification_command,
            controlled: None,
            completion: super::completion_evidence::Evidence::new(None),
            planning: None,
        })
    }
}
impl Drop for ModeLease<'_> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(mut mode) = self.host.mode.lock()
            && mode
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, &self.cancelled))
        {
            *mode = None;
        }
        self.host.changed.notify_waiters();
    }
}
#[derive(Default)]
pub struct Options {
    /// Optional verifier discovered by the trusted repository/host controller.
    /// It is guidance, never an instruction to execute outside normal tool policy.
    pub recommended_verifier: Option<String>,
    /// Explicit trusted host policy; model text cannot activate or weaken it.
    pub controlled_policy: Option<ag1_core::runtime::policy::TaskPolicy>,
}
pub struct PrintResult {
    pub turn: TurnResult,
    pub assessment: Option<Assessment>,
    pub exit_code: u8,
    pub continuation_recoveries: u8,
    pub completion_recoveries: u8,
}
struct FollowUp {
    timestamp: String,
    command: String,
    maximum: Option<u64>,
    compaction: ag1_core::context::compaction::Settings,
    compaction_timeout_ms: u64,
    retry: ag1_core::runtime::retry::Settings,
}
impl FollowUp {
    fn input(
        &self,
        kind: &str,
        attempt: u8,
        content: String,
        generations: u64,
    ) -> std::result::Result<TurnInput, DomainError> {
        let remaining = self.maximum.map(|limit| limit.saturating_sub(generations));
        if remaining == Some(0) {
            return Err(DomainError::LimitExceeded(
                "caller generation budget".into(),
            ));
        }
        let timestamp = chrono::DateTime::parse_from_rfc3339(&self.timestamp)
            .map_err(|_| DomainError::InvalidInput("print timestamp".into()))?
            .timestamp_millis();
        Ok(TurnInput {
            command: format!("print-{}-{kind}-{attempt}", self.command),
            message: json!({"role":"custom","customType":format!("runtime_{kind}"),"content":content,"display":false,"details":{"kind":format!("runtime_{kind}"),"mode":"continue_now"},"timestamp":timestamp}),
            timestamp: self.timestamp.clone(),
            max_generations: remaining,
            compaction: self.compaction.clone(),
            compaction_timeout_ms: self.compaction_timeout_ms,
            retry: self.retry.clone(),
        })
    }
}
fn continuation_message(attempt: u8) -> String {
    if attempt > 1 {
        "<runtime-continuation-state>\n- Repeated planning-only continuations are not allowed.\n- Do not emit more internal reasoning or planning text.\n- Output exactly one concrete next tool call now, or the direct answer if no tool is needed.\n</runtime-continuation-state>".into()
    } else {
        "<runtime-continuation-state>\n- The previous assistant step stopped at internal reasoning only.\n- Continue immediately with either one concrete next tool call or a direct answer.\n- Do not stop again without externalizing the next action.\n</runtime-continuation-state>".into()
    }
}
fn completion_message(attempt: u8, reasons: &[String], verifier: Option<&str>) -> String {
    #[derive(serde::Serialize)]
    struct State<'a> {
        kind: &'static str,
        version: u32,
        attempt: u8,
        passed: bool,
        reasons: &'a [String],
        #[serde(skip_serializing_if = "Option::is_none")]
        recommended_verifier: Option<&'a str>,
    }
    let state = serde_json::to_string(&State {
        kind: "coding_completion_required",
        version: 1,
        attempt,
        passed: false,
        reasons,
        recommended_verifier: verifier,
    })
    .expect("bounded completion guidance");
    let verifier = verifier
        .map(|v| format!("Run the canonical repository verifier exactly as recorded: {v}"))
        .unwrap_or_else(|| {
            "Discover and run the repository's canonical verifier before finishing.".into()
        });
    format!(
        "<runtime-completion-gate>\n{state}\nThe coding task is not verified complete. Continue with a concrete repair or verification action. Do not answer as though finished while any listed reason remains.\n{verifier}\n</runtime-completion-gate>"
    )
}
async fn settled_snapshot(
    host: &NativeSession,
    cancelled: &AtomicBool,
) -> std::result::Result<(ag1_core::runtime::boundary::Stamp, Arc<Vec<Value>>), DomainError> {
    loop {
        let changed = host.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        if cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let compacting = host
            .compacting
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_some();
        {
            let state = host.state()?;
            if !state.persistence_healthy() {
                return Err(DomainError::DurabilityUnknown);
            }
            if state.settled() && !state.pending_commit() && !compacting {
                return Ok((state.stamp().clone(), state.branch_snapshot()));
            }
            if !state.pending_commit() && !compacting {
                return Err(DomainError::DurabilityUnknown);
            }
        }
        changed.await;
    }
}
async fn assess(
    host: &NativeSession,
    cancelled: &AtomicBool,
) -> std::result::Result<Assessment, DomainError> {
    loop {
        let (stamp, branch) = settled_snapshot(host, cancelled).await?;
        let assessment = host
            .pool
            .submit(move || {
                let start = branch
                    .iter()
                    .rposition(|entry| entry["message"]["role"] == "user")
                    .ok_or(DomainError::Conflict)?;
                let messages: Vec<Value> = branch[start..]
                    .iter()
                    .filter_map(|entry| entry.get("message").cloned())
                    .collect();
                completion::assess(&messages, &[], &[], VerificationAuthority::MessageHistory)
            })
            .await?;
        let (current, _) = settled_snapshot(host, cancelled).await?;
        if current == stamp {
            return Ok(assessment);
        }
    }
}
/// One print prompt and its bounded policy follow-ups. The explicit caller
/// generation budget applies across the chain; no implicit deadline is added.
pub async fn run(
    host: &NativeSession,
    input: TurnInput,
    resolver: &dyn ProviderResolver,
    tools: Arc<dyn ToolRunner>,
    listener: &dyn Listener,
    options: Options,
) -> std::result::Result<PrintResult, DomainError> {
    let _activity = host.enter()?;
    if options
        .recommended_verifier
        .as_ref()
        .is_some_and(|v| v.len() > 4096 || v.contains('\0'))
    {
        return Err(DomainError::InvalidInput("print verifier guidance".into()));
    }
    if options.controlled_policy.is_some() && !tools.supports_controlled_policy() {
        return Err(DomainError::Unavailable(
            "runner does not enforce controlled policy".into(),
        ));
    }
    let controlled = options
        .controlled_policy
        .map(super::controlled::new)
        .transpose()?;
    let prompt = ag1_core::context::hygiene::content(&input.message, "\n");
    let mut mutation = completion::expects_mutation(&prompt);
    let mut lease = ModeLease::begin(
        host,
        mutation
            .then(|| options.recommended_verifier.clone())
            .flatten(),
    )?;
    lease.controlled = controlled;
    if let Some(controlled) = &lease.controlled {
        let guard = controlled.lock().map_err(|_| DomainError::Conflict)?;
        mutation |= guard.policy().mutation_expected;
        if let Some(verifier) = &guard.policy().verification {
            if lease
                .verification_command
                .as_ref()
                .is_some_and(|existing| existing != verifier)
            {
                return Err(DomainError::InvalidInput(
                    "conflicting controlled verifier guidance".into(),
                ));
            }
            lease.verification_command = Some(verifier.clone());
        }
    }
    if mutation && lease.verification_command.is_none() {
        let root = host.root.clone();
        let cancelled = lease.cancelled.clone();
        let activity = host.retain_work()?;
        let discovered = host
            .pool
            .submit(move || {
                let _activity = activity;
                super::verification::capture(&root, &cancelled)?.verifier(&prompt)
            })
            .await;
        match discovered {
            Ok(command) => lease.verification_command = command,
            Err(DomainError::Cancelled) => return Err(DomainError::Cancelled),
            Err(error) => NativeSession::emit(
                listener,
                SessionEvent::Recovery(
                    json!({"type":"verifier_discovery_unavailable","reason":format!("{error:?}")}),
                ),
            ),
        }
    }
    // Resolve the verifier once before the first request. A host override or
    // fresh discovery must feed both request guidance and receipt assessment.
    let resolved_policy = lease
        .controlled
        .as_ref()
        .map(|controlled| {
            let controller = controlled.lock().map_err(|_| DomainError::Conflict)?;
            let mut policy = controller.policy().clone();
            policy.verification = lease.verification_command.clone();
            Ok::<_, DomainError>(policy)
        })
        .transpose()?;
    if let Some(policy) = resolved_policy {
        lease.controlled = Some(super::controlled::new(policy)?);
    }
    lease.completion =
        super::completion_evidence::Evidence::new(lease.verification_command.clone());
    let follow = FollowUp {
        timestamp: input.timestamp.clone(),
        command: ag1_storage::digest(input.command.as_bytes()),
        maximum: input.max_generations,
        compaction: input.compaction.clone(),
        compaction_timeout_ms: input.compaction_timeout_ms,
        retry: input.retry.clone(),
    };
    let mut turn = host
        .run_inner(
            input,
            resolver,
            tools.clone(),
            listener,
            super::RunOrigin::User,
            Some(&lease),
        )
        .await?;
    let mut generations = turn.generations;
    let mut continuation_recoveries = 0;
    let mut completion_recoveries = 0;
    loop {
        if lease.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let mut gate = assess(host, &lease.cancelled).await?;
        mutation |= gate.latest_mutation_index.is_some();
        if let Some(controlled) = &lease.controlled {
            let controller = controlled.lock().map_err(|_| DomainError::Conflict)?;
            mutation |= controller.policy().mutation_expected;
            if mutation
                && controller.policy().verification.is_some()
                && !controller.verified_after_mutation()
            {
                gate.passed = false;
                gate.passing_verification_count = 0;
                gate.reasons.push(
                    "the canonical verifier has not passed after an acknowledged mutation".into(),
                );
            }
        }
        {
            let evidence = lease.completion.lock().map_err(|_| DomainError::Conflict)?;
            mutation |= evidence.mutation_seen;
            if mutation && !evidence.verified() {
                if gate.passed {
                    gate.reasons.push("no successful native verifier receipt remains after the latest mutation or uncertain effect".into());
                }
                gate.passed = false;
                gate.passing_verification_count = 0;
            }
        }
        let paused = turn.assistant["nativePolicy"] == "repetition_guard";
        let follow_up = if !paused
            && completion::needs_continuation(&turn.assistant)
            && continuation_recoveries < 2
        {
            continuation_recoveries += 1;
            Some((
                "continuation_state",
                continuation_recoveries,
                continuation_message(continuation_recoveries),
            ))
        } else if !paused
            && mutation
            && !gate.passed
            && completion::looks_final(&turn.assistant)
            && completion_recoveries < 3
        {
            completion_recoveries += 1;
            Some((
                "completion_gate",
                completion_recoveries,
                completion_message(
                    completion_recoveries,
                    &gate.reasons,
                    lease.verification_command.as_deref(),
                ),
            ))
        } else {
            None
        };
        let Some((kind, attempt, content)) = follow_up else {
            turn.generations = generations;
            return Ok(PrintResult {
                exit_code: u8::from(mutation && !gate.passed),
                turn,
                assessment: mutation.then_some(gate),
                continuation_recoveries,
                completion_recoveries,
            });
        };
        let input = follow.input(kind, attempt, content, generations)?;
        NativeSession::emit(
            listener,
            SessionEvent::Recovery(
                json!({"type":"mode_recovery_preparing","kind":kind,"attempt":attempt}),
            ),
        );
        if lease.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        turn = host
            .run_inner(
                input,
                resolver,
                tools.clone(),
                listener,
                super::RunOrigin::Continuation,
                Some(&lease),
            )
            .await?;
        generations = generations
            .checked_add(turn.generations)
            .ok_or(DomainError::Conflict)?;
    }
}
