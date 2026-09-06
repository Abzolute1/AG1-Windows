//! Branch-local episodes use the current journal owner and host inference lane.
use crate::{
    file_tools::FileExecution,
    session::tool_journal::{JournalMutationError, ToolJournal},
    source::SourceService,
    workspace::Workspace,
};
use ag1_core::{
    DomainError, SessionId,
    context::{ContextArtifactPort, episode, episode_draft},
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        episode as presentation,
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Scope, ValidatedCall},
    },
};
use ag1_storage::{Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
pub struct EpisodeTools {
    workspace: Arc<Workspace>,
    store: Arc<Store>,
    origin: String,
}
pub struct PreparedEpisode {
    call: ValidatedCall,
    target: String,
    journal: Arc<ToolJournal>,
}
impl PreparedEpisode {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
    pub fn may_mutate(&self) -> bool {
        matches!(
            self.call.args()["action"].as_str(),
            Some("begin" | "close" | "draft")
        )
    }
}
fn result(result: Value, effect: Effect) -> AdapterOutcome {
    AdapterOutcome {
        result,
        effect,
        artifact_page: false,
    }
}
fn rejected(error: DomainError, effect: Effect) -> AdapterOutcome {
    result(
        json!({"content":[{"type":"text","text":format!("Episode tool failed: {error:?}")}],"isError":true}),
        effect,
    )
}
impl EpisodeTools {
    pub fn new(workspace: Arc<Workspace>, store: Arc<Store>, origin: String) -> Self {
        Self {
            workspace,
            store,
            origin,
        }
    }
    fn scope(&self, scope: &Scope, call: &ValidatedCall) -> Result<(), DomainError> {
        if scope.workspace != self.workspace.id().as_str()
            || call.name() != "episode"
            || call.origin() != Origin::BuiltIn
        {
            Err(DomainError::PermissionDenied)
        } else {
            Ok(())
        }
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        journal: Option<&Arc<ToolJournal>>,
    ) -> Result<PreparedEpisode, DomainError> {
        self.scope(scope, call)?;
        let journal = journal
            .ok_or_else(|| {
                DomainError::Unavailable("Episode tool requires the session journal owner".into())
            })?
            .clone();
        journal.snapshot(scope)?;
        Ok(PreparedEpisode {
            call: call.clone(),
            target: format!(
                "episode:{}:{}:{}:{}",
                scope.workspace, scope.session, scope.epoch, scope.revision
            ),
            journal,
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedEpisode,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<AdapterOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        let check = || {
            if cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                self.workspace.verify_root()?;
                plan.journal.snapshot(scope).map(|_| ())
            }
        };
        let prepared = (|| {
            self.scope(scope, authorization.call())?;
            check()?;
            let call = authorization.consume(
                registry,
                ExecutionFence {
                    scope,
                    policy_revision,
                    target_revision: &plan.target,
                    now: now(),
                },
            )?;
            if call.args_hash() != plan.call.args_hash()
                || call.operation() != plan.call.operation()
                || call.call_id() != plan.call.call_id()
            {
                return Err(DomainError::Conflict);
            }
            let branch = plan.journal.snapshot(scope)?;
            let artifacts = ContextStore::new(
                &self.store,
                SessionId::new(&scope.session)?,
                &self.origin,
                call.operation(),
                plan.journal.timestamp(),
                Default::default(),
            )?;
            Ok((call, branch, artifacts))
        })();
        let (call, branch, mut artifacts) = match prepared {
            Ok(p) => p,
            Err(error) => return Ok(rejected(error, Effect::None)),
        };
        let args = call.args();
        let mut source = SourceService::new(self.workspace.clone());
        let action = args["action"].as_str().unwrap_or("");
        if matches!(action, "list" | "show" | "search") {
            return Ok(
                match presentation::query(&branch, args, &artifacts, |s| source.validate(s).status)
                    .and_then(|value| {
                        check()?;
                        Ok(value)
                    }) {
                    Ok(value) => result(value, Effect::None),
                    Err(error) => rejected(error, Effect::None),
                },
            );
        }
        if action == "begin" {
            let id = format!(
                "ep_{}",
                &ag1_storage::digest(call.operation().as_bytes())[..12]
            );
            let record = match episode::begin_record(
                &branch,
                plan.journal.timestamp(),
                &id,
                args["goal"].as_str().unwrap_or(""),
                args["title"].as_str(),
            ) {
                Ok(r) => r,
                Err(e) => return Ok(rejected(e, Effect::None)),
            };
            if let Err(e) = check() {
                return Ok(rejected(e, Effect::None));
            }
            return self.publish(
                &plan,
                scope,
                &call,
                &record,
                presentation::begin_result(&record),
            );
        }
        if episode::find_open(&branch).is_none() {
            return Ok(rejected(
                DomainError::InvalidInput("no open episode".into()),
                Effect::None,
            ));
        }
        if action == "draft" {
            let journal = plan.journal.stored_snapshot(scope)?;
            let (configuration, model) = plan.journal.model(scope)?;
            let draft = episode_draft::prepare(
                &journal,
                &mut artifacts,
                episode_draft::DraftOptions {
                    scope: model.clone(),
                    root_session: Some(&scope.session),
                    max_input_chars: configuration.model["contextWindow"]
                        .as_u64()
                        .map(|n| n.saturating_mul(2).clamp(64000, 240000) as usize),
                    max_output_tokens: None,
                    model_max: configuration.model["maxTokens"].as_u64().unwrap_or(0),
                },
            )?;
            if let Err(e) = check() {
                return Ok(rejected(e, Effect::Committed));
            }
            let mut output = plan.journal.draft_generation(
                scope,
                &call,
                draft.request_for(&model)?,
                deadline,
            )?;
            // Preserve raw terminal/partial snapshots before candidate parsing or
            // epoch validation. This stays on the same bounded blocking worker.
            let (mut input, mut tokens) = (0, 0);
            if let Some(snapshot) = &output.snapshot {
                input = snapshot
                    .usage
                    .input
                    .saturating_add(snapshot.usage.cache_read)
                    .saturating_add(snapshot.usage.cache_write);
                tokens = snapshot.usage.output;
                artifacts.put_evidence(
                    &json!(snapshot).to_string(),
                    "episode-draft-output",
                    &json!({"type":"system","name":"episode-draft","callId":call.call_id()}),
                    &[],
                    None,
                )?;
            }
            output.acknowledge_custody();
            if !output.settled {
                return Err(DomainError::Unavailable(
                    "Episode draft transport requires reconciliation".into(),
                ));
            }
            let text = match output.text {
                Ok(text) => text,
                Err(e) => return Ok(rejected(e, Effect::Committed)),
            };
            if let Err(e) = check() {
                return Ok(rejected(e, Effect::Committed));
            }
            let (_, current) = plan.journal.model(scope)?;
            let candidate = match draft.finish(&mut artifacts, &current, &text, false) {
                Ok(c) => c,
                // Durable artifact failures need the same unknown-effect path as
                // every other P09 adapter. Invalid model JSON is a settled failure.
                Err(e @ DomainError::InvalidInput(_)) => return Ok(rejected(e, Effect::Committed)),
                Err(e) => return Err(e),
            };
            if let Err(e) = check() {
                return Ok(rejected(e, Effect::Committed));
            }
            return Ok(result(
                presentation::draft_result(&candidate, input, tokens),
                Effect::Committed,
            ));
        }
        if action != "close" {
            return Ok(rejected(
                DomainError::InvalidInput("episode action".into()),
                Effect::None,
            ));
        }
        if let Err(e) =
            presentation::completion_gate(&branch, args, &artifacts, |s| source.validate(s).status)
        {
            return Ok(rejected(e, Effect::None));
        }
        let journal = plan.journal.stored_snapshot(scope)?;
        let record = match episode::close_record(
            &journal,
            &mut artifacts,
            plan.journal.timestamp(),
            call.call_id(),
            args,
        ) {
            Ok(r) => r,
            Err(e @ DomainError::InvalidInput(_)) => return Ok(rejected(e, Effect::None)),
            Err(e) => return Err(e),
        };
        // Revalidate evidence after custody; a changed file cannot support a
        // completed close merely because it was current before artifact storage.
        let mut source = SourceService::new(self.workspace.clone());
        if let Err(e) = check().and_then(|()| {
            presentation::completion_gate(&branch, args, &artifacts, |s| source.validate(s).status)
        }) {
            return Ok(rejected(e, Effect::Committed));
        }
        let health = episode::health(&record, &artifacts, |s| source.validate(s).status);
        self.publish(
            &plan,
            scope,
            &call,
            &record,
            presentation::closed_result(&record, &health),
        )
    }
    fn publish(
        &self,
        plan: &PreparedEpisode,
        scope: &Scope,
        call: &ValidatedCall,
        record: &episode::EpisodeRecord,
        value: Value,
    ) -> Result<AdapterOutcome, DomainError> {
        match plan
            .journal
            .commit_state(scope, call, "context_episode", record.value().clone())
        {
            Ok(()) => Ok(result(value, Effect::Committed)),
            Err(JournalMutationError::BeforeEffect(e)) => Ok(rejected(
                e,
                if call.args()["action"] == "close" {
                    Effect::Committed
                } else {
                    Effect::None
                },
            )),
            Err(JournalMutationError::Persistence(e)) => Err(e),
        }
    }
}
