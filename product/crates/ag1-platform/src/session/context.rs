use super::NativeSession;
use crate::source::SourceService;
use ag1_core::{
    DomainError,
    context::{
        ContextArtifact, ContextArtifactPort,
        compiler::{Compilation, compile_request},
        features::Features,
        projections::{self, TurnAddenda},
    },
    evidence::{Freshness, SourceValidation, memory_sources},
    memory::MemoryService,
    runtime::{context, messages, session::PreparationAction},
    storage::StoragePort,
};
use ag1_storage::context::ContextStore;
use serde_json::json;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};

pub(super) struct ContextWitness {
    observes_memory: bool,
    memory_revision: Option<String>,
    sources: BTreeMap<String, SourceValidation>,
}
pub(super) struct PreparedContext {
    pub projection: Compilation,
    pub system_prompt: String,
    pub witness: ContextWitness,
    pub loaded_memory: std::collections::BTreeSet<String>,
}
impl NativeSession {
    pub(super) async fn retain_provider_output(
        &self,
        operation: &str,
        timestamp: &str,
        model: &serde_json::Value,
        snapshot: Option<&ag1_core::provider::ProviderSnapshot>,
        error: Option<&DomainError>,
    ) -> Result<ContextArtifact, DomainError> {
        let session = self.state()?.stamp().session().clone();
        let store = self.store.clone();
        let origin = self.origin.clone();
        let operation = operation.to_owned();
        let timestamp = timestamp.to_owned();
        let model = model.clone();
        let snapshot = snapshot.cloned();
        let error = error.map(|error| format!("{error:?}"));
        self.pool
            .submit(move || {
                let envelope = json!({
                    "version": 1,
                    "kind": "runtime-provider-output",
                    "operation": operation,
                    "model": model,
                    "snapshot": snapshot,
                    "error": error,
                });
                let mut context = ContextStore::new(
                    &store,
                    session,
                    &origin,
                    &operation,
                    &timestamp,
                    Features::default(),
                )?;
                context.put_evidence(
                    &envelope.to_string(),
                    "runtime-provider-output",
                    &json!({"type":"session","name":"provider-output","callId":operation}),
                    &[],
                    None,
                )
            })
            .await
    }
    pub(super) async fn prepare_context(
        &self,
        preparation: &PreparationAction,
        operation: &str,
        timestamp: &str,
        verification_command: Option<&str>,
        controlled: Option<super::controlled::Shared>,
        callback: Option<(Arc<dyn super::ToolRunner>, Arc<AtomicBool>)>,
    ) -> Result<PreparedContext, DomainError> {
        let branch = preparation.branch().to_vec();
        let mut messages = preparation.messages().clone().into_values();
        let mut configuration = preparation.configuration().clone();
        let mut options = preparation.compile_options();
        let revision = preparation.journal_revision();
        let session = preparation.stamp().session().clone();
        let store = self.store.clone();
        let root = self.root.clone();
        let memory_id = self.workspace.clone();
        let origin = self.origin.clone();
        let operation = operation.to_owned();
        let timestamp = timestamp.to_owned();
        let verification_command = verification_command.map(str::to_owned);
        let delegated = self.delegated;
        let capture = if callback.is_some() {
            let mut capture = self.capture_callback()?;
            capture.request_view(preparation)?;
            Some(capture)
        } else {
            None
        };
        let activity = self.retain_work()?;
        let prepared=self.pool
            .submit(move || {
                let _activity = activity;
                if let Some((tools, cancel)) = callback {
                    let mut artifacts = ContextStore::new(&store,session.clone(),&origin,&operation,&timestamp,Features::default())?;
                    let result=capture.as_ref().ok_or(DomainError::Conflict)?.call(tools.as_ref(),&json!({"type":"context","messages":messages}),&cancel,&mut artifacts)?;
                    if let Some(result)=result {
                        messages=result.as_array().ok_or_else(||DomainError::InvalidInput("extension context messages".into()))?.clone();
                        ag1_core::context::validate_history(&messages)?;
                    }
                }
                if delegated {
                    let mut artifacts = ContextStore::new(&store,session,&origin,&operation,&timestamp,Features::default())?;
                    artifacts.put_evidence(&json!({"version":1,"systemPrompt":configuration.system_prompt,"tools":configuration.tools,"model":configuration.model,"messages":messages}).to_string(),"runtime-request-input",&json!({"type":"session","name":"subagent-context"}),&[],None)?;
                    let profile = ag1_core::model::infer_capabilities(configuration.model["contextWindow"].as_u64(),
                        ag1_core::context::prompt::compact(configuration.model["provider"].as_str()),
                        configuration.model["input"].as_array().is_some_and(|input| input.iter().any(|v| v == "image")),
                        configuration.model["reasoning"] == true);
                    let reduced = ag1_core::context::hygiene::copy(&messages,&ag1_core::runtime::subagent::hygiene(&profile))?;
                    return Ok(PreparedContext {
                        projection: Compilation { messages:messages::to_provider(&ag1_core::context::hygiene::live(&reduced,chrono::DateTime::parse_from_rfc3339(&timestamp).map_err(|_| DomainError::InvalidInput("subagent timestamp".into()))?.timestamp_millis().max(0) as u64))?, stats:json!({"mode":"subagent-hygiene"}) },
                        system_prompt:configuration.system_prompt,
                        loaded_memory:Default::default(),
                        witness:ContextWitness { observes_memory:false,memory_revision:None,sources:Default::default() },
                    });
                }
                let validator = RefCell::new(SourceService::new(root));
                let sources = RefCell::new(BTreeMap::<String, SourceValidation>::new());
                let validate = |source: &str| {
                    if !source.starts_with("file:") {
                        return None;
                    }
                    // Every projection in this preparation uses the same source
                    // verdict. Final admission re-reads it after asynchronous work.
                    if let Some(verdict) = sources.borrow().get(source).cloned() {
                        return Some(verdict);
                    }
                    let verdict = validator.borrow_mut().validate(source);
                    sources
                        .borrow_mut()
                        .insert(source.to_owned(), verdict.clone());
                    Some(verdict)
                };
                let mut artifacts = ContextStore::new(
                    &store,
                    session,
                    &origin,
                    &operation,
                    &timestamp,
                    Features::default(),
                )?;
                let (memory, memory_revision) = MemoryService::load(store.as_ref(), &memory_id)?;
                let query = context::latest_query(&messages).unwrap_or_default();
                let selected = memory.select(&query, 1600, 16, |entry| {
                    memory_sources(entry, |source| {
                        validate(source)
                            .map(|v| v.status)
                            .unwrap_or(Freshness::Unverified)
                    })
                });
                let memory = projections::memory(&selected);
                if let Some(addendum) = projections::turn(TurnAddenda {
                    verification_command: verification_command.as_deref(),
                    memory: memory.as_deref(),
                    ..Default::default()
                }) {
                    configuration.system_prompt.push_str("\n\n");
                    configuration.system_prompt.push_str(&addendum);
                }
                if let Some(controlled) = &controlled {
                    let controller = controlled.lock().map_err(|_| DomainError::Conflict)?;
                    configuration.system_prompt.push_str(&controller.policy().format());
                    if super::controlled::serialize(&configuration.model) {
                        configuration.system_prompt.push_str("\nControlled runtime is active: emit exactly one tool call per assistant turn, then wait for its result before the next action. Emitting multiple tool calls in one turn gets the extra calls rejected.");
                    }
                    if let Some(next) = controller.guidance() {
                        configuration.system_prompt.push_str(&format!("\nControlled runtime: next required action: {next}."));
                    }
                }
                let tail = context::working_state(&branch, &messages, &artifacts, validate);
                let original = context::append_working_state(&messages, tail.as_deref())?;
                let tail_tokens = tail
                    .as_ref()
                    .map(|text| {
                        ag1_core::context::estimate_tokens(&json!({"role":"custom","content":text}))
                    })
                    .unwrap_or(0);
                options.fixed_tokens =
                    Some(configuration.fixed_tokens().saturating_add(tail_tokens));
                // Retain the complete, unreduced request input before any
                // compiler reduction, alongside P08's raw journal custody.
                let input = json!({
                    "version": 1,
                    "systemPrompt": configuration.system_prompt,
                    "tools": configuration.tools,
                    "model": configuration.model,
                    "messages": original,
                    "memoryRevision": memory_revision,
                });
                artifacts.put_evidence(
                    &input.to_string(),
                    "runtime-request-input",
                    &json!({"type":"session","name":"request-context"}),
                    &[],
                    None,
                )?;
                let projection = artifacts
                    .project(&original, revision, |_, store| {
                        let mut compiled = compile_request(&messages, &options, store)?;
                        compiled.messages = messages::to_provider(&context::append_working_state(
                            &compiled.messages,
                            tail.as_deref(),
                        )?)?;
                        Ok(compiled)
                    })?
                    .projection;
                Ok(PreparedContext {
                    projection,
                    loaded_memory: selected
                        .entries
                        .iter()
                        .filter_map(|entry| entry["id"].as_str().map(str::to_owned))
                        .collect(),
                    system_prompt: configuration.system_prompt,
                    witness: ContextWitness {
                        observes_memory: true,
                        memory_revision,
                        sources: sources.into_inner(),
                    },
                })
            })
            .await?;
        *self
            .last_context
            .lock()
            .map_err(|_| DomainError::Conflict)? = json!({
            "state":"prepared", "journal_revision":revision.to_string(),
            "stats":prepared.projection.stats,
            "sources":prepared.witness.sources.values().map(|s|json!({"reference":s.reference,"status":format!("{:?}",s.status),"current":s.current})).collect::<Vec<_>>()
        });
        Ok(prepared)
    }

    pub(super) async fn context_current(
        &self,
        witness: ContextWitness,
    ) -> Result<bool, DomainError> {
        if !witness.observes_memory {
            return Ok(true);
        }
        let store = self.store.clone();
        let root = self.root.clone();
        let memory_id = self.workspace.clone();
        self.pool
            .submit(move || {
                let memory = store.load_memory(&memory_id)?;
                if memory
                    .as_ref()
                    .is_some_and(|memory| memory.diagnostic.is_some())
                {
                    return Err(DomainError::InvalidInput(
                        "corrupt memory retained; explicit repair required".into(),
                    ));
                }
                if memory.map(|memory| memory.sha256) != witness.memory_revision {
                    return Ok(false);
                }
                let mut validator = SourceService::new(root);
                for (source, expected) in witness.sources {
                    let actual = validator.validate(&source);
                    if actual.status != expected.status || actual.current != expected.current {
                        return Ok(false);
                    }
                }
                Ok(true)
            })
            .await
    }
}

/// Match the reference inspector's virtualized context usage: successful usage
/// plus trailing messages, with fresh fixed-cost attribution after compaction
/// until the new branch has a successful nonzero usage sample.
pub(super) fn research_profile(
    state: &ag1_core::runtime::SessionRuntime,
) -> Result<ag1_core::tools::research::Profile, ag1_core::DomainError> {
    use ag1_core::context::{compaction::usage_estimate, estimate_tokens};
    let config = state.configuration();
    let window = config.context_limit() as f64;
    let messages = state.messages().values().cloned().collect::<Vec<_>>();
    let latest_compaction = state
        .branch()
        .iter()
        .rposition(|entry| entry["type"] == "compaction");
    let needs_fresh = latest_compaction.is_some_and(|index| {
        let last = state.branch()[index + 1..].iter().rev().find_map(|entry| {
            let message = &entry["message"];
            (entry["type"] == "message"
                && message["role"] == "assistant"
                && !matches!(message["stopReason"].as_str(), Some("aborted" | "error")))
            .then_some(message)
        });
        last.is_none_or(|message| {
            let usage = &message["usage"];
            usage["totalTokens"].as_u64().unwrap_or(0) == 0
                && ["input", "output", "cacheRead", "cacheWrite"]
                    .iter()
                    .all(|field| usage[field].as_u64().unwrap_or(0) == 0)
        })
    });
    let tokens = if needs_fresh {
        config.fixed_tokens() + messages.iter().map(estimate_tokens).sum::<u64>()
    } else {
        usage_estimate(&messages, false)?["tokens"]
            .as_u64()
            .unwrap_or(0)
    };
    Ok(ag1_core::tools::research::profile(
        window,
        if window > 0. {
            tokens as f64 / window * 100.
        } else {
            0.
        },
    ))
}
