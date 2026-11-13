#[path = "runtime_policy/mod.rs"]
mod policy;
#[path = "runtime_subagent/mod.rs"]
mod subagent;
#[path = "runtime_tree/mod.rs"]
mod tree;

#[test]
fn plan_selection_and_transitions_preserve_source_rules_and_discard_does_not_resurrect_execution() {
    use ag1_core::runtime::planning::{self, Plan, Status};
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/plan-state-oracle.json"
    ))
    .unwrap();
    for row in oracle["transitions"].as_array().unwrap() {
        let from: Status = serde_json::from_value(row["from"].clone()).unwrap();
        let to: Status = serde_json::from_value(row["to"].clone()).unwrap();
        assert_eq!(from.allows(to), row["allowed"].as_bool().unwrap(), "{row}");
    }
    let draft: Plan = serde_json::from_value(oracle["draft"].clone()).unwrap();
    let timestamp = draft.updated_at.clone();
    let accepted = draft.transition(Status::Accepted, &timestamp).unwrap();
    assert_eq!(serde_json::to_value(&accepted).unwrap(), oracle["accepted"]);
    let executing = accepted.begin_step(0, &timestamp).unwrap();
    assert_eq!(
        serde_json::to_value(&executing).unwrap(),
        oracle["executing"]
    );
    assert!(draft.begin_step(0, &timestamp).is_err());
    assert!(accepted.begin_step(2, &timestamp).is_err());
    let clarifying: Plan = serde_json::from_value(oracle["clarifying"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(clarifying.answer(0, " Web ", &timestamp).unwrap()).unwrap(),
        oracle["answered"]
    );
    assert!(clarifying.answer(1, "Out of range", &timestamp).is_err());
    assert!(clarifying.transition(Status::Accepted, &timestamp).is_err());
    // B06: source clear keeps execution fields, rejects its own new record on
    // load and resurrects the older executing state. Native clears those fields.
    assert_eq!(oracle["observedAfterDiscard"], "executing");
    let malformed: Plan = serde_json::from_value(oracle["discarded"].clone()).unwrap();
    assert!(malformed.validate().is_err());
    let discarded = executing.transition(Status::Discarded, &timestamp).unwrap();
    let record = |plan: &Plan| json!({"type":"custom","customType":planning::ARTIFACT,"data":plan});
    let branch = vec![
        record(&accepted),
        record(&clarifying),
        record(&executing),
        record(&discarded),
        json!({"type":"custom","customType":planning::SELECTION,"data":oracle["selection"]}),
    ];
    let selection = planning::selection(&branch).unwrap();
    assert_eq!(planning::plans(&branch).len(), 2);
    assert_eq!(
        planning::resolve(&branch, &selection.accepted.unwrap())
            .unwrap()
            .status,
        Status::Discarded
    );
    assert_eq!(
        planning::resolve(&branch, &selection.planning.unwrap())
            .unwrap()
            .status,
        Status::Clarifying
    );
    assert!(discarded.transition(Status::Accepted, &timestamp).is_err());
}

#[test]
fn structured_plans_and_clarifying_questions_match_the_pinned_tool() {
    let oracle: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/submit-plan-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 9);
    for row in oracle["rows"].as_array().unwrap() {
        assert_eq!(
            ag1_core::tools::submit_plan::execute(&row["args"]).unwrap(),
            row["result"],
            "{}",
            row["scenario"]
        );
    }
}
use ag1_core::{DomainError, SessionId, runtime::boundary::Boundary};
fn boundary() -> Boundary {
    Boundary::new(SessionId::new("fixture").unwrap(), 1).unwrap()
}

#[test]
fn xml_fallback_candidates_and_recovery_guidance_match_the_pinned_sdk() {
    use ag1_core::runtime::xml;
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/xml-tool-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 26);
    assert_eq!(oracle["analyses"].as_array().unwrap().len(), 13);
    for row in oracle["rows"].as_array().unwrap() {
        let names: Vec<String> = serde_json::from_value(row["knownTools"].clone()).unwrap();
        assert_eq!(
            json!(xml::parse(row["text"].as_str().unwrap(), &names).unwrap()),
            row["calls"],
            "{}",
            row["id"]
        );
    }
    for row in oracle["analyses"].as_array().unwrap() {
        let input = &row["input"];
        let analysis = xml::analyze(
            input["callCount"].as_u64().unwrap() as usize,
            input["assistantTextLength"].as_u64().unwrap() as usize,
            input["assistantStopReason"].as_str(),
            input["iteration"].as_u64().unwrap() as usize,
        );
        assert_eq!(json!(analysis), row["analysis"]);
        let names = serde_json::from_value::<Vec<String>>(row["names"].clone()).unwrap();
        assert_eq!(json!(xml::guidance(&analysis, &names)), row["guidance"]);
    }
    let text = "<read><path>x</path></read>".repeat(129);
    assert!(matches!(
        xml::parse(&text, &["read".into()]),
        Err(DomainError::LimitExceeded(_))
    ));
}

#[test]
fn retry_classification_overflow_and_backoff_match_the_pinned_sdk() {
    use ag1_core::runtime::retry;
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/retry-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 94);
    for row in rows {
        let message = &row["message"];
        let window = row["window"].as_u64().unwrap();
        assert_eq!(json!(retry::classify(message)), row["kind"], "{message}");
        assert_eq!(
            json!(retry::overflow(message, window)),
            row["overflow"],
            "{message}"
        );
        assert_eq!(
            json!(retry::retryable(message, window)),
            row["retryable"],
            "{message}"
        );
        for delay in row["backoff"].as_array().unwrap() {
            assert_eq!(
                json!(
                    retry::backoff_ms(
                        message["errorMessage"].as_str().unwrap(),
                        delay["attempt"].as_u64().unwrap() as u32,
                        delay["base"].as_u64().unwrap()
                    )
                    .unwrap()
                ),
                delay["delay"],
                "{message}"
            );
        }
    }
}

#[test]
fn emergency_checkpoint_and_source_provenance_match_the_pinned_coordinator() {
    use ag1_core::context::{ContextArtifact, compaction};
    use serde_json::Value;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/compaction-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    for row in rows {
        let mut result = if row["kind"] == "fallback" {
            compaction::emergency_checkpoint(&row["preparation"], row["cause"].as_str().unwrap())
                .unwrap()
        } else {
            row["result"].clone()
        };
        let artifact: ContextArtifact = serde_json::from_value(row["artifact"].clone()).unwrap();
        compaction::attach_provenance(&mut result, &artifact).unwrap();
        assert_eq!(result, row["output"]);
    }
}

#[test]
fn provider_message_projection_matches_pinned_sdk_without_changing_the_journal_values() {
    use serde_json::Value;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/messages-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 10);
    for row in rows {
        let messages = row["messages"].as_array().unwrap();
        let original = messages.clone();
        let projection = ag1_core::runtime::messages::to_provider(messages).unwrap();
        assert_eq!(
            serde_json::json!(projection),
            row["output"],
            "{}",
            row["id"]
        );
        assert_eq!(*messages, original);
    }
}

#[test]
fn recovery_state_transitions_and_continue_fences_match_immutable_sdk_captures() {
    use ag1_core::runtime::recovery::{Decision, Owner, Recovery};
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/recovery-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 6);
    let owner = |value: &Value| serde_json::from_value::<Owner>(value.clone()).unwrap();
    for row in oracle["rows"].as_array().unwrap() {
        let mut state = boundary();
        let mut recovery = Recovery::default();
        let mut now = 0;
        let mut events = vec![];
        let mut errors = vec![];
        let mut runs = 0;
        for (step, expected) in row["steps"]
            .as_array()
            .unwrap()
            .iter()
            .zip(row["states"].as_array().unwrap())
        {
            let mut returned = Value::Null;
            match step["op"].as_str().unwrap() {
                "arm" => returned = json!(recovery.try_arm(owner(&step["owner"]))),
                "clear" | "cancel" => recovery.clear(Some(owner(&step["owner"]))),
                "change" => state.changed().unwrap(),
                "schedule" => {
                    let who = owner(&step["owner"]);
                    let accepted = recovery
                        .schedule(who, state.stamp(), now, step["delay"].as_u64().unwrap())
                        .unwrap();
                    returned = json!(accepted);
                    if !accepted {
                        events.push(json!({"type":"recovery_replay_skipped","owner":who,"reason":"already_scheduled","message":"blocked"}));
                    }
                }
                "advance" => {
                    now += step["ms"].as_u64().unwrap();
                    match recovery.poll(now,state.stamp(),false) {
                        Some(Decision::Run(_))=>{runs+=1;if row["failRun"]==true {errors.push("fixture run error");}},
                        Some(Decision::StateChanged(who))=>events.push(json!({"type":"recovery_replay_skipped","owner":who,"reason":"state_changed","message":"stale"})),
                        None=>{},
                    }
                }
                _ => panic!("unknown captured step"),
            }
            assert_eq!(
                json!({"returned":returned,"owner":recovery.owner(),"scheduled":recovery.is_scheduled(),"runs":runs,"events":events,"errors":errors}),
                *expected,
                "{} {step}",
                row["id"]
            );
        }
    }
    assert_eq!(oracle["replay"].as_array().unwrap().len(), 8);
    for row in oracle["replay"].as_array().unwrap() {
        let mut state = boundary();
        let mut recovery = Recovery::default();
        recovery
            .schedule(Owner::Retry, state.stamp(), 0, 0)
            .unwrap();
        let change = row["change"].as_str().unwrap();
        if !["none", "empty", "streaming"].contains(&change) {
            state.changed().unwrap();
        }
        assert_eq!(
            json!(
                recovery.poll(0, state.stamp(), change == "streaming")
                    == Some(Decision::Run(Owner::Retry))
            ),
            row["accepted"],
            "{row}"
        );
    }
}

#[test]
fn recovery_owners_coalesce_and_stale_session_model_tool_or_thinking_state_never_replays() {
    use ag1_core::runtime::recovery::{Decision, Owner, Recovery};
    let state = boundary();
    let mut recovery = Recovery::default();
    assert!(recovery.try_arm(Owner::Retry));
    assert!(recovery.try_arm(Owner::Retry));
    assert_eq!(recovery.owner(), Some(Owner::Retry));
    assert!(!recovery.try_arm(Owner::Compaction));
    recovery.clear(Some(Owner::Compaction));
    assert_eq!(recovery.owner(), Some(Owner::Retry));
    recovery.clear(Some(Owner::Retry));
    assert!(recovery.try_arm(Owner::Compaction));
    assert!(
        !recovery
            .schedule(Owner::Retry, state.stamp(), 0, 100)
            .unwrap()
    );
    recovery.clear(None);
    assert!(
        recovery
            .schedule(Owner::Retry, state.stamp(), 0, 100)
            .unwrap()
    );
    assert!(
        recovery
            .schedule(Owner::Retry, state.stamp(), 50, 100)
            .unwrap()
    );
    assert_eq!(recovery.poll(100, state.stamp(), false), None);
    assert_eq!(
        recovery.poll(150, state.stamp(), false),
        Some(Decision::Run(Owner::Retry))
    );
    assert_eq!(recovery.poll(150, state.stamp(), false), None);
    assert_eq!(recovery.owner(), None);
    for _changed in ["transcript", "model", "thinking", "tool", "epoch"] {
        let mut state = boundary();
        recovery
            .schedule(Owner::Compaction, state.stamp(), 0, 0)
            .unwrap();
        state.changed().unwrap();
        assert_eq!(
            recovery.poll(0, state.stamp(), false),
            Some(Decision::StateChanged(Owner::Compaction))
        );
        assert_eq!(recovery.owner(), None);
    }
    recovery
        .schedule(Owner::Retry, state.stamp(), 0, 0)
        .unwrap();
    assert_eq!(
        recovery.poll(0, boundary().stamp(), false),
        Some(Decision::StateChanged(Owner::Retry))
    );
    recovery
        .schedule(Owner::Retry, state.stamp(), 0, 0)
        .unwrap();
    assert_eq!(
        recovery.poll(0, state.stamp(), true),
        Some(Decision::StateChanged(Owner::Retry))
    );
    recovery
        .schedule(Owner::Retry, state.stamp(), 0, 0)
        .unwrap();
    recovery.clear(None);
    assert_eq!(recovery.poll(0, state.stamp(), false), None);
    assert!(!recovery.is_scheduled());
    assert!(
        recovery
            .schedule(Owner::Retry, state.stamp(), u64::MAX, 1)
            .is_err()
    );
    assert!(
        recovery
            .schedule(Owner::Retry, state.stamp(), 0, 86_400_001)
            .is_err()
    );
    assert_eq!(recovery.owner(), None);
}

#[test]
fn request_admission_rechecks_cancellation_and_every_state_revision_after_preparation() {
    // The native runner performs these asynchronous stages between prepare/admit.
    for _stage in ["context", "transform", "credentials"] {
        let mut state = boundary();
        let prepared = state.prepare().unwrap();
        state.cancel();
        assert!(state.admit(prepared).is_err());
        assert!(state.settled());
        state.next_turn().unwrap();
        let prepared = state.prepare().unwrap();
        let generation = state.admit(prepared).unwrap();
        assert!(!state.settled());
        state.generation_terminal(&generation).unwrap();
        assert!(state.settled());
        assert!(matches!(
            state.generation_terminal(&generation),
            Err(DomainError::Conflict)
        ));
    }
    for _changed in [
        "journal", "leaf", "model", "tools", "thinking", "task", "context",
    ] {
        let mut state = boundary();
        let prepared = state.prepare().unwrap();
        state.changed().unwrap();
        assert!(matches!(state.admit(prepared), Err(DomainError::Conflict)));
        let current = state.prepare().unwrap();
        assert!(state.admit(current).is_ok());
    }
}

#[test]
fn persistence_acknowledgement_and_tool_results_hold_admission_until_durably_drained() {
    let mut state = boundary();
    let prepared = state.prepare().unwrap();
    let generation = state.admit(prepared).unwrap();
    state.register_tools(&["one".into(), "two".into()]).unwrap();
    state.generation_terminal(&generation).unwrap();
    assert!(!state.settled());
    assert!(state.prepare().is_err());
    let first = state.begin_append().unwrap();
    let second = state.begin_append().unwrap();
    assert!(matches!(
        state.acknowledge_tool(&second, "two"),
        Err(DomainError::Conflict)
    ));
    state.acknowledge_tool(&first, "one").unwrap();
    assert!(state.prepare().is_err());
    assert!(matches!(
        state.acknowledge_tool(&first, "one"),
        Err(DomainError::Conflict)
    ));
    state.acknowledge_tool(&second, "two").unwrap();
    assert!(state.settled());
    let prepared = state.prepare().unwrap();
    let pending = state.begin_append().unwrap();
    assert!(state.admit(prepared).is_err());
    state.acknowledge(&pending).unwrap();
    assert!(state.prepare().is_ok());
}

#[test]
fn failed_persistence_remains_sticky_after_later_successful_events() {
    let mut state = boundary();
    let failed = state.begin_append().unwrap();
    let later = state.begin_append().unwrap();
    state.append_failed(&failed).unwrap();
    assert!(!state.persistence_healthy());
    state.acknowledge(&later).unwrap();
    assert!(state.settled());
    assert!(matches!(
        state.prepare(),
        Err(DomainError::DurabilityUnknown)
    ));
    state.cancel();
    assert!(matches!(
        state.next_turn(),
        Err(DomainError::DurabilityUnknown)
    ));
}

#[test]
fn compaction_reserves_before_draining_and_changes_epoch_only_after_a_commit() {
    let mut state = boundary();
    let prepared = state.prepare().unwrap();
    let compaction = state.reserve_compaction().unwrap();
    assert!(state.admit(prepared).is_err());
    assert!(state.prepare().is_err());
    assert!(state.reserve_compaction().is_err());
    assert!(state.compaction_ready());
    state.finish_compaction(&compaction, false).unwrap();
    assert_eq!(state.stamp().epoch(), 1);
    let prepared = state.prepare().unwrap();
    let generation = state.admit(prepared).unwrap();
    state.register_tools(&["call".into()]).unwrap();
    let compaction = state.reserve_compaction().unwrap();
    assert!(!state.compaction_ready());
    assert!(state.finish_compaction(&compaction, true).is_err());
    state.generation_terminal(&generation).unwrap();
    assert!(!state.compaction_ready());
    let append = state.begin_append().unwrap();
    state.acknowledge_tool(&append, "call").unwrap();
    assert!(state.compaction_ready());
    state.finish_compaction(&compaction, true).unwrap();
    assert_eq!(state.stamp().epoch(), 2);
    assert!(state.prepare().is_ok());
}

#[test]
fn cancellation_waits_for_compaction_and_retains_a_commit_that_won_the_race() {
    for committed in [false, true] {
        let mut state = boundary();
        let compaction = state.reserve_compaction().unwrap();
        let append = committed.then(|| state.begin_append().unwrap());
        state.cancel();
        assert!(!state.settled());
        assert!(!state.compaction_ready());
        assert_eq!(state.next_turn(), Err(DomainError::Conflict));
        if let Some(append) = append {
            assert!(state.finish_compaction(&compaction, true).is_err());
            state.acknowledge(&append).unwrap();
        }
        state.finish_compaction(&compaction, committed).unwrap();
        assert_eq!(state.stamp().epoch(), if committed { 2 } else { 1 });
        assert!(state.settled());
        assert!(matches!(state.prepare(), Err(DomainError::Cancelled)));
        assert_eq!(
            state.finish_compaction(&compaction, committed),
            Err(DomainError::Conflict)
        );
        state.next_turn().unwrap();
    }
}

#[test]
fn abandoning_a_summary_does_not_acknowledge_other_running_work() {
    let mut state = boundary();
    let preparation = state.prepare().unwrap();
    let generation = state.admit(preparation).unwrap();
    let compaction = state.reserve_compaction().unwrap();
    let append = state.begin_append().unwrap();
    state.cancel();
    state.finish_compaction(&compaction, false).unwrap();
    assert!(!state.settled());
    state.generation_terminal(&generation).unwrap();
    assert!(!state.settled());
    state.acknowledge(&append).unwrap();
    assert!(state.settled());
}

#[test]
fn cancellation_drains_effects_and_foreign_runtime_tokens_never_settle_a_replacement() {
    let mut old = boundary();
    let mut replacement = boundary();
    let old_preparation = old.prepare().unwrap();
    let current = replacement.prepare().unwrap();
    assert!(replacement.admit(old_preparation).is_err());
    let generation = replacement.admit(current).unwrap();
    replacement.register_tools(&["active".into()]).unwrap();
    let append = replacement.begin_append().unwrap();
    let foreign = old.begin_append().unwrap();
    assert!(replacement.acknowledge(&foreign).is_err());
    replacement.cancel();
    assert!(!replacement.settled());
    assert!(replacement.next_turn().is_err());
    replacement.generation_terminal(&generation).unwrap();
    assert!(!replacement.settled());
    replacement.acknowledge_tool(&append, "active").unwrap();
    assert!(replacement.settled());
    replacement.next_turn().unwrap();
    assert!(replacement.prepare().is_ok());
    let mut bounded = boundary();
    assert!(
        bounded
            .register_tools(&["duplicate".into(), "duplicate".into()])
            .is_err()
    );
    assert!(bounded.settled());
    assert!(bounded.register_tools(&vec!["x".into(); 129]).is_err());
    for _ in 0..256 {
        bounded.begin_append().unwrap();
    }
    assert!(matches!(
        bounded.begin_append(),
        Err(DomainError::QueueFull)
    ));
}

#[test]
fn working_state_assembly_and_user_insertion_match_the_pinned_agent_session() {
    use ag1_core::{
        context::{ContextArtifact, ContextArtifactPort},
        evidence::{SourceReference, SourceValidation},
        runtime::context,
    };
    use serde_json::{Value, json};
    struct NoArtifacts;
    impl ContextArtifactPort for NoArtifacts {
        fn put_text(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &[String],
        ) -> Result<ContextArtifact, DomainError> {
            panic!("request-local assembly must not write artifacts");
        }
        fn get(&self, _: &str) -> Result<ContextArtifact, DomainError> {
            Err(DomainError::NotFound)
        }
        fn leased_ids(&self) -> Result<Vec<String>, DomainError> {
            Ok(vec![])
        }
    }
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/context-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 21);
    for row in rows {
        let branch = row["branch"].as_array().unwrap();
        let messages = row["messages"].as_array().unwrap();
        let before = messages.clone();
        let tail = context::working_state(branch, messages, &NoArtifacts, |source| {
            let verdict = &row["verdicts"][source];
            if verdict.is_null() {
                return None;
            }
            Some(SourceValidation {
                reference: source.into(),
                status: serde_json::from_value(verdict["status"].clone()).unwrap(),
                current: verdict["currentSha256"].as_str().map(|sha| {
                    SourceReference::parse(source)
                        .ok()
                        .and_then(|reference| SourceReference::new(reference.path(), sha).ok())
                        .map(|reference| reference.reference())
                        .unwrap_or_else(|| sha.into())
                }),
            })
        });
        assert_eq!(json!(tail), row["tail"], "{} tail", row["id"]);
        let output = context::append_working_state(messages, tail.as_deref()).unwrap();
        // Intentional native correction to the retained SDK oracle: changing
        // working state belongs at the request tail, rather than rewriting an
        // earlier user message ahead of all subsequent assistant/tool rounds.
        // The legacy tail text is still checked exactly above.
        if let Some(tail) = tail {
            assert_eq!(&output[..messages.len()], messages);
            assert_eq!(output.len(), messages.len() + 1);
            assert_eq!(
                output.last().unwrap(),
                &json!({"role":"custom","customType":"ag1-working-state","content":tail,"display":false,"timestamp":0})
            );
        } else {
            assert_eq!(json!(output), row["output"], "{} output", row["id"]);
        }
        assert_eq!(*messages, before);
    }
}

#[test]
fn native_memory_actions_and_presentation_match_the_pinned_tool() {
    use ag1_core::{evidence::Freshness, memory::MemoryStore, tools::memory};
    use serde_json::Value;
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/memory-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 19);
    for row in rows {
        let store = MemoryStore::parse(&row["store"]).unwrap();
        let loaded = row["loaded"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().to_owned())
            .collect();
        let status = match row["status"].as_str() {
            Some("current") => Freshness::Current,
            Some("stale") => Freshness::Stale,
            Some("missing") => Freshness::Missing,
            _ => Freshness::Unverified,
        };
        let output = memory::prepare(
            &store,
            &row["args"],
            &loaded,
            row["now"].as_i64().unwrap(),
            |_| status,
        );
        if row.get("error").is_some() {
            assert!(output.is_err(), "{}", row["id"]);
            assert_eq!(store.value(), &row["after"], "{}", row["id"]);
        } else {
            let output = output.unwrap();
            assert_eq!(output.result, row["result"], "{}", row["id"]);
            assert_eq!(
                output.next.as_ref().unwrap_or(&store).value(),
                &row["after"],
                "{}",
                row["id"]
            );
        }
    }
}

#[test]
fn artifact_literal_scanning_matches_sdk_lines_context_ranges_and_unicode_casing() {
    use ag1_core::tools::artifact_scan;
    use serde_json::{Value, json};
    use std::io::Read;
    struct Chunks<'a> {
        bytes: &'a [u8],
        size: usize,
    }
    impl Read for Chunks<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let n = output.len().min(self.size).min(self.bytes.len());
            output[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(n)
        }
    }
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/artifact-scan-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 19);
    for row in rows {
        let text = row["text"].as_str().map(str::to_owned).unwrap_or_else(|| {
            format!(
                "{}{}{}",
                row["prefix"].as_str().unwrap_or_default(),
                row["repeat"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .repeat(row["repeat"]["count"].as_u64().unwrap_or(0) as usize),
                row["suffix"].as_str().unwrap_or_default()
            )
        });
        let sizes: &[usize] = if text.len() > 10000 {
            &[8191, 65536]
        } else {
            &[1, 2, 7, 65536]
        };
        for size in sizes {
            let limit = row["maxMatches"].as_u64().unwrap_or(100) as usize;
            let scan = artifact_scan::scan(
                Chunks {
                    bytes: text.as_bytes(),
                    size: *size,
                },
                row["query"].as_str().unwrap(),
                row["ignoreCase"].as_bool().unwrap_or(true),
                limit,
                || Ok(()),
            )
            .unwrap();
            let hits = scan.hits.iter().map(|hit| {
                let len = hit.context_end-hit.context_start;
                let (offset, max) = if (1..=32768).contains(&len) { (hit.context_start,len) } else { (hit.context_start.saturating_sub(256),1024) };
                json!({"line":hit.line,"text":hit.text,"byteOffset":hit.byte_offset,"context":{"offset":offset,"maxBytes":max,"anchorLine":hit.line}})
            }).collect::<Vec<_>>();
            assert_eq!(json!(hits), row["hits"], "{} chunk={size}", row["id"]);
            assert_eq!(
                json!(scan.total_matches > limit as u64),
                row["truncated"],
                "{}",
                row["id"]
            );
            assert_eq!(json!(scan.bytes), row["bytes"]);
            assert_eq!(json!(scan.sha256), row["sha256"]);
        }
    }
}
#[test]
fn artifact_literal_scan_rejects_invalid_utf8_and_honors_cancellation_between_chunks() {
    use ag1_core::tools::artifact_scan;
    for invalid in [&[0xff][..], &[0xf0, 0x9f][..]] {
        assert!(artifact_scan::scan(invalid, "needle", true, 1, || Ok(())).is_err());
    }
    let mut checks = 0;
    let long = "x".repeat(200000);
    let result = artifact_scan::scan(long.as_bytes(), "needle", true, 1, || {
        checks += 1;
        if checks == 2 {
            Err(DomainError::Cancelled)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(DomainError::Cancelled)));
    assert_eq!(checks, 2);
}

#[test]
fn task_state_tool_actions_match_twelve_unchanged_sdk_reductions_and_write_decisions() {
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/task-state-tool-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for row in rows {
        let branch = row
            .get("state")
            .map(|state| vec![json!({"type":"custom","customType":"task_state","data":state})])
            .unwrap_or_default();
        let prepared = ag1_core::tools::task_state::prepare(
            &branch,
            &row["args"],
            oracle["now"].as_str().unwrap(),
            "ts_00000002",
        );
        if row["error"] == true {
            assert!(prepared.is_err(), "{}", row["id"]);
            continue;
        }
        let mut prepared = prepared.unwrap();
        if row["args"]["action"] == "show" && row.get("state").is_some() {
            prepared.result["content"][0]["text"] =
                serde_json::from_str(prepared.result["content"][0]["text"].as_str().unwrap())
                    .unwrap();
        }
        assert_eq!(prepared.result, row["output"], "{}", row["id"]);
        assert_eq!(json!(usize::from(prepared.next.is_some())), row["writes"]);
        assert_eq!(
            prepared
                .next
                .as_ref()
                .map(|next| next.value().clone())
                .or_else(|| row.get("state").cloned())
                .unwrap_or(Value::Null),
            row["after"]
        );
    }
}

#[test]
fn todo_tool_preserves_sdk_transitions_and_exposes_current_query_ids() {
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/todo-tool-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 19);
    for row in rows {
        let branch = row
            .get("state")
            .map(|state| vec![json!({"type":"custom","customType":"todo_list","data":state})])
            .unwrap_or_default();
        let result = ag1_core::tools::todo::prepare(
            &branch,
            &row["args"],
            oracle["now"].as_i64().unwrap(),
            oracle["newPlanId"].as_str().unwrap(),
        );
        if row["error"] == true {
            assert!(result.is_err(), "{}", row["id"]);
            continue;
        }
        let plan = result.unwrap();
        if matches!(row["id"].as_str(), Some("query" | "query-phase")) {
            // Native queries intentionally expose reusable IDs; the frozen SDK
            // oracle returned only a generic acknowledgement and null todos.
            let items = row
                .get("state")
                .map(|state| state["items"].clone())
                .unwrap_or_else(|| json!([]));
            assert_eq!(
                plan.result["details"],
                json!({
                    "todos": items, "planId": row["state"]["planId"], "phase": row["state"]["phase"],
                })
            );
            assert_eq!(
                plan.result["content"],
                json!([{"type":"text","text":format!(
                    "Current todo list (reuse these IDs when updating): {items}"
                )}])
            );
        } else {
            assert_eq!(plan.result, row["output"], "{} output", row["id"]);
        }
        assert_eq!(json!(plan.records), row["records"], "{} records", row["id"]);
    }
}

#[test]
fn runtime_observation_preserves_sdk_reopening_and_labels_unverified_claims() {
    use serde_json::{Value, json};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/observation-tool-oracle.json"
    ))
    .unwrap();
    let rows = oracle["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for row in rows {
        let branch = row
            .get("state")
            .map(|state| vec![json!({"type":"custom","customType":"task_state","data":state})])
            .unwrap_or_default();
        let result = ag1_core::tools::observation::prepare(
            &branch,
            &row["args"],
            oracle["now"].as_str().unwrap(),
            ag1_core::tools::observation::ObservationIdentity {
                observation: "obs_000000000001",
                entry: "observation-entry",
                task_entry: "ts_00000001",
                call: "observation-call",
            },
        );
        if row["error"] == true {
            assert!(result.is_err(), "{}", row["id"]);
            continue;
        }
        let plan = result.unwrap();
        // Preserve the frozen source oracle while checking the intentional
        // native distinction between recording a claim and verifying it.
        let mut expected = row["output"].clone();
        expected["details"]["verification"] = json!("recorded_claim");
        expected["details"]["sourceReference"] = json!("session-entry:observation-entry");
        expected["content"][0]["text"] = json!(format!(
            "{}. Recording this claim does not independently verify its result; cite a matching check receipt for verification.",
            row["output"]["content"][0]["text"].as_str().unwrap(),
        ));
        assert_eq!(plan.result, expected, "{} result", row["id"]);
        assert_eq!(json!(plan.records), row["records"], "{} records", row["id"]);
    }
}

#[test]
fn runtime_episode_matches_sdk_branch_queries_completion_gates_and_metadata() {
    use ag1_core::{
        context::{ContextArtifact, ContextArtifactPort, episode},
        storage::{StoredEntry, StoredJournal},
        tools::episode as tool,
    };
    use serde_json::{Value, json};
    struct Artifacts(Vec<Value>);
    impl ContextArtifactPort for Artifacts {
        fn put_text(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
            _: &[String],
        ) -> Result<ContextArtifact, DomainError> {
            unreachable!()
        }
        fn put_evidence(
            &mut self,
            text: &str,
            kind: &str,
            _: &Value,
            _: &[String],
            _: Option<&str>,
        ) -> Result<ContextArtifact, DomainError> {
            self.0.push(
                json!({"id":"art_eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","bytes":text.len(),"kind":kind}),
            );
            self.get("art_eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
        }
        fn get(&self, id: &str) -> Result<ContextArtifact, DomainError> {
            let row = self
                .0
                .iter()
                .find(|a| a["id"] == id)
                .ok_or(DomainError::NotFound)?;
            Ok(ContextArtifact {
                id: id.into(),
                kind: row["kind"].as_str().unwrap_or("text").into(),
                bytes: row["bytes"].as_u64().unwrap_or(0),
                sha256: "0".repeat(64),
                line_count: Some(1),
                salient_lines: vec![],
            })
        }
        fn leased_ids(&self) -> Result<Vec<String>, DomainError> {
            Ok(vec![])
        }
    }
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/runtime/episode-tool-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 27);
    for row in oracle["rows"].as_array().unwrap() {
        let mut artifacts = Artifacts(oracle["artifacts"].as_array().unwrap().clone());
        let branch = row["branch"].as_array().unwrap();
        let args = &row["args"];
        let now = oracle["now"].as_str().unwrap();
        let validate = |source: &str| {
            serde_json::from_value(
                oracle["freshness"]
                    .get(source)
                    .cloned()
                    .unwrap_or(json!("invalid")),
            )
            .unwrap()
        };
        let mut records = vec![];
        let output = (|| match args["action"].as_str().unwrap() {
            "begin" => {
                let record = episode::begin_record(
                    branch,
                    now,
                    "ep_000000000001",
                    args["goal"].as_str().unwrap_or(""),
                    args["title"].as_str(),
                )?;
                records.push(
                    json!({"type":"custom","customType":"context_episode","data":record.value()}),
                );
                Ok(tool::begin_result(&record))
            }
            "close" => {
                tool::completion_gate(branch, args, &artifacts, validate)?;
                let journal = StoredJournal {
                    revision: branch.len() as u64,
                    committed_sequence: 1,
                    header: vec![],
                    leaf: branch
                        .last()
                        .and_then(|e| e["id"].as_str())
                        .map(str::to_owned),
                    entries: branch
                        .iter()
                        .enumerate()
                        .map(|(i, e)| StoredEntry {
                            id: e["id"].as_str().unwrap().into(),
                            parent: e["parentId"].as_str().map(str::to_owned),
                            physical_order: i as u64,
                            raw: serde_json::to_vec(e).unwrap(),
                            normalized: serde_json::to_vec(e).unwrap(),
                        })
                        .collect(),
                };
                let record =
                    episode::close_record(&journal, &mut artifacts, now, "episode-call", args)?;
                records.push(
                    json!({"type":"custom","customType":"context_episode","data":record.value()}),
                );
                let health = episode::health(&record, &artifacts, validate);
                Ok(tool::closed_result(&record, &health))
            }
            _ => tool::query(branch, args, &artifacts, validate),
        })();
        if row["error"] == true {
            assert!(output.is_err(), "{}: {output:?}", row["id"]);
        } else {
            assert_eq!(output.unwrap(), row["output"], "{} output", row["id"]);
            assert_eq!(json!(records), row["records"], "{} records", row["id"]);
        }
    }
}
#[path = "runtime_completion/mod.rs"]
mod completion;
#[path = "runtime_planning/mod.rs"]
mod planning_workflow;

#[test]
fn deeper_observation_explicit_task_scope_projects_once_and_rejects_changed_objective() {
    use ag1_core::{
        task::TaskState,
        tools::observation::{ObservationIdentity, prepare},
    };
    use serde_json::json;
    let task = TaskState::empty("2026-09-11T12:00:00.000Z")
        .set(
            &json!({"objective":"Investigate rendering"}),
            "2026-09-11T12:00:00.000Z",
        )
        .unwrap();
    let branch = vec![json!({"type":"custom","customType":"task_state","data":task.value()})];
    let ids = || ObservationIdentity {
        observation: "obs_000000000001",
        entry: "observation-entry",
        task_entry: "ts_00000001",
        call: "call",
    };
    let mut args = json!({"feature":"GPU surface","result":"fail","environment":"owned fixture","revision":"r1","reporter":"user","reproduction":"open window","notes":"blank","taskObjective":"Investigate rendering"});
    let plan = prepare(&branch, &args, "2026-09-11T12:00:00.000Z", ids()).unwrap();
    assert_eq!(plan.records.len(), 2);
    assert_eq!(plan.result["details"]["verification"], "recorded_claim");
    assert!(plan.result["details"]["taskStateEntryId"].is_string());
    args["updateTaskState"] = json!(false);
    assert_eq!(
        prepare(&branch, &args, "2026-09-11T12:00:00.000Z", ids())
            .unwrap()
            .records
            .len(),
        1
    );
    args["taskObjective"] = json!("A different task");
    assert!(prepare(&branch, &args, "2026-09-11T12:00:00.000Z", ids()).is_err());
}

#[test]
fn deeper_exploration_prior_marks_age_missing_provenance_and_stale_sources_without_verifying_claims()
 {
    use ag1_core::runtime::subagent::explore::prior_with_freshness;
    let board = serde_json::json!({"version":1,"findings":[{"question":"old","summary":"historical","at":10},{"question":"new","summary":"claim","at":20,"childSession":"child","sourceReferences":["file:changed@sha256:fixture"]}]});
    let text = prior_with_freshness(&board, 30, |_| "stale".into()).unwrap();
    assert!(text.contains("unknown; inspect current source"));
    assert!(text.contains("stale"));
    assert!(text.contains("not independently verified"));
    assert!(text.contains("\"ageMs\":\"10\""));
}

#[test]
fn npm_version_headers_are_not_zero_test_summaries() {
    use ag1_core::runtime::completion::{Status, classify};
    let output = "> owned-interface-probe@1.0.0 test\n> node --test verify.test.cjs\n✔ fixture value was updated\nℹ tests 1\nℹ pass 1\nℹ fail 0\n";
    assert_eq!(
        classify("npm test", output, false).unwrap().status,
        Status::Passed
    );
    for output in [
        "0 tests",
        "Collected: 0 tests",
        "running 0 tests",
        "=== 0 tests ran ===",
        "no tests ran",
        "collected 0 items",
    ] {
        assert_eq!(
            classify("npm test", output, false).unwrap().status,
            Status::NoTests,
            "{output}"
        );
    }
    assert_eq!(
        classify("npm test", "1 failed", false).unwrap().status,
        Status::Failed
    );
    assert_eq!(
        classify("npm test", output, true).unwrap().status,
        Status::Failed
    );
}
