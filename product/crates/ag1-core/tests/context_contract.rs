#[path = "support/compiler_fixture.rs"]
mod fixture;
use ag1_core::{
    DomainError,
    context::{compiler::compile_request, salient},
};
use fixture::{FixtureStore, recipe_input};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[test]
fn compiler_cold_and_warm_projections_match_sixteen_unchanged_sdk_captures() {
    let recipes: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/compiler-inputs.json"
    ))
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/compiler-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["groups"].as_array().unwrap().len(), 16);
    for (recipe, expected) in recipes["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["groups"].as_array().unwrap())
    {
        let id = recipe["id"].as_str().unwrap();
        assert_eq!(id, expected["id"]);
        let mut store = FixtureStore::default();
        let (messages, options) = recipe_input(recipe, &mut store);
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&messages).unwrap())
            ),
            expected["inputSha256"],
            "{id} input"
        );
        let original = messages.clone();
        let cold = compile_request(&messages, &options, &mut store).unwrap();
        assert_eq!(messages, original, "{id} changed input");
        assert_eq!(
            cold.stats, expected["cold"]["stats"],
            "{id} cold statistics"
        );
        assert_eq!(
            json!(cold.messages),
            expected["cold"]["messages"],
            "{id} cold projection"
        );
        assert_eq!(
            store.records.len(),
            expected["coldArtifacts"].as_u64().unwrap() as usize
        );
        let warm = compile_request(&cold.messages, &options, &mut store).unwrap();
        assert_eq!(
            warm.stats, expected["warm"]["stats"],
            "{id} warm statistics"
        );
        assert_eq!(
            json!(warm.messages),
            expected["warm"]["messages"],
            "{id} warm projection"
        );
        assert_eq!(
            json!(store.records),
            expected["artifacts"],
            "{id} immutable records"
        );
        for r in &store.records {
            let text = &store.texts[r["id"].as_str().unwrap()];
            assert_eq!(
                format!("{:x}", Sha256::digest(text.as_bytes())),
                r["sha256"]
            );
        }
    }
}
#[test]
fn durability_failure_never_returns_a_shrunken_projection() {
    let recipe = json!({"rounds":5,"textBytes":3000,"thinkingBytes":100,"window":32768,"finalUser":true,"recent":1});
    let mut store = FixtureStore::default();
    let (messages, options) = recipe_input(&recipe, &mut store);
    let original = messages.clone();
    store.fail = true;
    assert!(matches!(
        compile_request(&messages, &options, &mut store),
        Err(DomainError::DurabilityUnknown)
    ));
    assert_eq!(messages, original);
}
#[test]
fn salient_lines_keep_exact_conflicts_and_exclude_negated_filler() {
    let text = "/tmp/REQUIREMENT\nno actionable evidence\nCURRENT_CONTRACT: keep source current\nSTALE_CONTRACT: old behavior\nwarning ordinary\nCURRENT_CONTRACT: keep source current";
    assert_eq!(
        salient::select(text),
        vec![
            "CURRENT_CONTRACT: keep source current",
            "STALE_CONTRACT: old behavior"
        ]
    );
    assert_eq!(salient::score("without relevant contract"), 0);
}

#[test]
fn episode_records_retrieval_conflicts_candidates_and_features_match_sdk() {
    use ag1_core::context::{
        episode, episode_retrieval,
        features::{FeatureOverrides, Features},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let input: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/episode-inputs.json"
    ))
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/episode-oracle.json"
    ))
    .unwrap();
    let store = FixtureStore { records: input["artifacts"].as_array().unwrap().iter().map(|r| json!({"id":r["id"],"kind":r["kind"],"bytes":1,"sha256":"0".repeat(64),"lineCount":1})).collect(), ..Default::default() };
    for row in oracle["records"].as_array().unwrap() {
        let actual = episode::EpisodeRecord::parse(&row["input"])
            .ok()
            .map(|r| r.value().clone())
            .unwrap_or(Value::Null);
        assert_eq!(actual, row["output"], "episode parse {:?}", row["input"]);
    }
    let entries = input["entries"].as_array().unwrap();
    for row in oracle["searches"].as_array().unwrap() {
        let query = row["query"].as_str().unwrap();
        let validate = |source: &str| {
            serde_json::from_value(
                input["freshness"]
                    .get(source)
                    .cloned()
                    .unwrap_or(json!("invalid")),
            )
            .unwrap()
        };
        assert_eq!(
            episode_retrieval::search(entries, query, 5, &store, validate).unwrap(),
            row["result"],
            "search {query}"
        );
        let actual = episode_retrieval::addendum(entries, query, &store, validate).unwrap();
        if let Some(text) = &actual {
            assert!(text.encode_utf16().count() <= 2400);
        }
        assert_eq!(json!(actual), row["addendum"], "addendum {query}");
    }
    for row in oracle["candidates"].as_array().unwrap() {
        let allowed: BTreeSet<String> =
            serde_json::from_value(row["input"]["allowed"].clone()).unwrap();
        let actual =
            episode::parse_candidate(row["input"]["text"].as_str().unwrap(), &allowed, &store)
                .unwrap_or(Value::Null);
        assert_eq!(actual, row["output"], "candidate {:?}", row["input"]);
    }
    for row in oracle["features"].as_array().unwrap() {
        let explicit: FeatureOverrides =
            serde_json::from_value(row["input"]["explicit"].clone()).unwrap();
        let env: BTreeMap<String, String> =
            serde_json::from_value(row["input"]["env"].clone()).unwrap();
        assert_eq!(json!(Features::resolve(explicit, &env)), row["output"]);
    }
}

#[test]
fn system_prompt_resources_tool_modes_and_stable_prefixes_match_sdk() {
    use ag1_core::context::prompt::{self, PromptInput};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/prompt-oracle.json"
    ))
    .unwrap();
    for row in oracle["prompts"].as_array().unwrap() {
        let input: PromptInput = serde_json::from_value(row["input"].clone()).unwrap();
        // The native UI tools require an owned Runtime web target. The SDK
        // capture predates that boundary; keep every other prompt byte pinned.
        let expected = row["output"].as_str().unwrap().replace(
            "- verify UI you built: verify_ui (concrete claim) or screenshot_url (HTML path or localhost URL); interact_then_screenshot after clicks/fills. Do not start a server just to screenshot static HTML.",
            "- verify UI you built: verify_ui or screenshot_url require a ready AG1-owned web target started through Runtime; interact_then_screenshot after clicks/fills. These tools do not accept arbitrary websites or local file previews. Show local images (including simple vector SVG) with read; do not start a server just to show an image.",
        );
        assert_eq!(
            prompt::build(&input).unwrap(),
            expected,
            "prompt {:?}",
            row["input"]
        );
    }
    for row in oracle["prefixes"].as_array().unwrap() {
        let i = &row["input"];
        assert_eq!(
            prompt::prefix_shape(
                i["system"].as_str().unwrap(),
                i["tools"].as_array().unwrap(),
                i["rewrite"].as_u64().unwrap()
            ),
            row["output"]
        );
    }
    assert_eq!(
        oracle["prefixes"][0]["output"]["prefixHash"],
        oracle["prefixes"][1]["output"]["prefixHash"]
    );
}

#[test]
fn task_memory_observation_and_capsule_projections_match_reference() {
    use ag1_core::{
        context::{capsule::CapsuleRecord, projections},
        evidence::{InspectionEvidence, SourceValidation},
        memory::MemorySelection,
        task::TaskState,
    };
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/projection-oracle.json"
    ))
    .unwrap();
    for row in oracle["tasks"].as_array().unwrap() {
        let i = &row["input"];
        let state = TaskState::parse(&i["state"]).unwrap();
        let actual = projections::task(
            &state,
            |source| {
                i["verdicts"].get(source).map(|v| SourceValidation {
                    reference: source.into(),
                    status: serde_json::from_value(v["status"].clone()).unwrap(),
                    current: v["currentRevision"].as_str().map(str::to_string),
                })
            },
            |id| i["verified"].as_array().unwrap().iter().any(|v| v == id),
        );
        assert_eq!(json!(actual), row["output"]);
    }
    for row in oracle["memories"].as_array().unwrap() {
        let i = &row["input"];
        let selection = MemorySelection {
            entries: i["entries"].as_array().unwrap().clone(),
            stored_count: i["storedCount"].as_u64().unwrap() as usize,
            source_statuses: serde_json::from_value(
                i.get("sourceStatuses").cloned().unwrap_or(json!({})),
            )
            .unwrap(),
        };
        assert_eq!(json!(projections::memory(&selection)), row["output"]);
    }
    for row in oracle["inspections"].as_array().unwrap() {
        let i = &row["input"];
        let evidence: Vec<_> = i["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| InspectionEvidence::parse(v).unwrap())
            .collect();
        assert_eq!(
            json!(projections::inspection(&evidence, |source| {
                serde_json::from_value(i["verdicts"][source]["status"].clone()).unwrap()
            })),
            row["output"]
        );
    }
    for row in oracle["runtimes"].as_array().unwrap() {
        assert_eq!(
            json!(projections::runtime(row["input"].as_array().unwrap(), 12)),
            row["output"]
        );
    }
    for row in oracle["capsules"].as_array().unwrap() {
        assert_eq!(
            CapsuleRecord::parse(&row["input"])
                .ok()
                .map(|r| json!(r))
                .unwrap_or(Value::Null),
            row["output"]
        );
    }
}

#[test]
fn summary_preparation_hygiene_horizons_and_working_memory_match_sdk() {
    use ag1_core::{
        context::{
            compaction::{self, SummaryState},
            hygiene::{self, HygieneOptions},
            working_memory::{self, MemoryInputs},
        },
        task::TaskState,
    };
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/compaction-oracle.json"
    ))
    .unwrap();
    for row in oracle["histories"].as_array().unwrap() {
        let i = &row["input"];
        let messages = i["messages"].as_array().unwrap();
        let options: HygieneOptions = serde_json::from_value(i["options"].clone()).unwrap();
        assert_eq!(
            json!(hygiene::copy(messages, &options).unwrap()),
            row["hygiene"],
            "hygiene {}",
            i["id"]
        );
        assert_eq!(
            json!(hygiene::live(messages, 1000)),
            row["live"],
            "live {}",
            i["id"]
        );
        assert_eq!(
            json!(hygiene::provider_working_set(messages, true, &options, 1000).unwrap()),
            row["cv"]
        );
        assert_eq!(
            json!(hygiene::strip_old_thinking(messages).unwrap()),
            row["stripped"]
        );
        let entries = i["entries"].as_array().unwrap();
        let keep = i["keepTokens"].as_u64().unwrap();
        assert_eq!(
            json!(compaction::cut_point(entries, 0, entries.len(), keep).unwrap()),
            row["cut"]
        );
        assert_eq!(
            compaction::prepare_full(
                entries,
                &compaction::Settings {
                    enabled: true,
                    reserve_tokens: 8192,
                    keep_recent_tokens: keep
                }
            )
            .unwrap()
            .unwrap_or(Value::Null),
            row["preparation"],
            "full summary preparation {}",
            i["id"]
        );
        let prep = compaction::prepare(entries, keep).unwrap();
        if row["preparation"].is_null() {
            assert!(prep.is_none());
        } else {
            let prep = prep.unwrap();
            assert!(!prep.pair_boundary_adjusted);
            let value = json!(prep);
            for key in [
                "firstKeptEntryId",
                "messagesToSummarize",
                "turnPrefixMessages",
                "isSplitTurn",
            ] {
                assert_eq!(
                    value[key], row["preparation"][key],
                    "preparation {key} {}",
                    i["id"]
                );
            }
        }
    }
    for row in oracle["horizons"].as_array().unwrap() {
        let i = &row["input"];
        let w = i["window"].as_u64().unwrap();
        assert_eq!(json!(compaction::reserve_tokens(w)), row["reserve"]);
        assert_eq!(
            json!(compaction::keep_recent(w, i["overhead"].as_u64().unwrap())),
            row["keep"]
        );
    }
    for row in oracle["summaries"].as_array().unwrap() {
        let input: SummaryState = serde_json::from_value(row["input"].clone()).unwrap();
        assert_eq!(compaction::deterministic_summary(&input), row["output"]);
    }
    for row in oracle["ledgers"].as_array().unwrap() {
        let i = &row["input"];
        let task = i.get("taskState").map(|t| TaskState::parse(t).unwrap());
        let memory = working_memory::extract(
            i["messages"].as_array().unwrap(),
            MemoryInputs {
                previous: i.get("previous"),
                task: task.as_ref(),
                next_todo: i["nextTodo"].as_str(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(memory, row["output"]);
        assert_eq!(working_memory::format(&memory).unwrap(), row["formatted"]);
        assert_eq!(working_memory::primary(&memory).unwrap(), row["primary"]);
    }
}

#[test]
fn selected_branch_recent_access_and_changed_line_hints_match_sdk() {
    use ag1_core::context::signals::{self, GitOutput};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/signals-oracle.json"
    ))
    .unwrap();
    for row in oracle["recent"].as_array().unwrap() {
        let i = &row["input"];
        assert_eq!(
            json!(signals::recent(
                std::path::Path::new("/ag1-context-signals-fixture"),
                i["entries"].as_array().unwrap(),
                i["exclude"].as_str(),
                |_| None
            )),
            row["output"]
        );
    }
    for row in oracle["trees"].as_array().unwrap() {
        let outputs = row["input"]["outputs"].as_array().unwrap();
        let out = |i: usize| GitOutput {
            stdout: outputs[i]["stdout"].as_str().unwrap(),
            code: outputs[i]["code"].as_i64().unwrap() as i32,
        };
        let tree = signals::working_tree(out(0), out(1), out(2), out(3)).unwrap();
        assert_eq!(
            json!({"insideRepository":tree.inside_repository,"changedFiles":tree.changed_files,"changedLines":tree.changed_lines}),
            row["output"]
        );
    }
}

#[test]
fn isolated_summary_requests_match_actual_reference_provider_calls() {
    use ag1_core::context::summary_request::{self, SummaryRequest};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/summary-oracle.json"
    ))
    .unwrap();
    for row in oracle["groups"].as_array().unwrap() {
        let i = &row["input"];
        let request = summary_request::request(SummaryRequest {
            messages: i["messages"].as_array().unwrap(),
            reserve_tokens: i["reserveTokens"].as_u64().unwrap(),
            previous: i["previous"].as_str(),
            focus: i["focus"].as_str(),
            memory: i.get("memory"),
            split_prefix: i["splitPrefix"] == true,
            model_max: 8192,
            root_session: Some("fixture"),
        })
        .unwrap();
        assert_eq!(request.context, row["request"]["context"]);
        assert_eq!(json!(request.max_tokens), row["request"]["maxTokens"]);
        assert!(request.context.get("tools").is_none());
        assert_eq!(request.context["messages"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn optional_same_model_drafts_match_sdk_and_keep_originals_when_budget_rejects() {
    use ag1_core::{
        context::episode_draft::{self, DraftOptions},
        model::ModelScope,
        storage::{StoredEntry, StoredJournal},
    };
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/draft-inputs.json"
    ))
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/draft-oracle.json"
    ))
    .unwrap();
    let scope = ModelScope {
        provider: "local".into(),
        endpoint: "http://127.0.0.1:8999".into(),
        model: "preserved-model".into(),
        runtime_fingerprint: "frozen-runtime".into(),
    };
    for (input, expected) in inputs["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(oracle["groups"].as_array().unwrap())
    {
        let entries = input["entries"].as_array().unwrap();
        let journal = StoredJournal {
            revision: 1,
            committed_sequence: 1,
            header: b"{}".to_vec(),
            leaf: entries.last().unwrap()["id"].as_str().map(str::to_string),
            entries: entries
                .iter()
                .enumerate()
                .map(|(i, e)| StoredEntry {
                    id: e["id"].as_str().unwrap().into(),
                    parent: e["parentId"].as_str().map(str::to_string),
                    physical_order: i as u64,
                    raw: e.to_string().into_bytes(),
                    normalized: e.to_string().into_bytes(),
                })
                .collect(),
        };
        let mut store = FixtureStore::default();
        let draft = episode_draft::prepare(
            &journal,
            &mut store,
            DraftOptions {
                scope: scope.clone(),
                root_session: Some("fixture"),
                max_input_chars: input["options"]["maxInputChars"]
                    .as_u64()
                    .map(|n| n as usize),
                max_output_tokens: Some(3000),
                model_max: 8192,
            },
        );
        if expected["failed"] == true {
            assert!(matches!(draft, Err(DomainError::LimitExceeded(_))));
            assert!(!store.texts.is_empty());
            assert_eq!(store.records[0]["kind"], "episode-trajectory");
        } else {
            let draft = draft.unwrap();
            let request = draft.request_for(&scope).unwrap();
            assert_eq!(
                request.context["systemPrompt"],
                expected["request"]["systemPrompt"]
            );
            assert_eq!(
                request.context["messages"][0]["content"][0]["text"], expected["request"]["prompt"],
                "draft prompt {}",
                input["id"]
            );
            assert_eq!(json!(request.max_tokens), expected["request"]["maxTokens"]);
            let result = draft
                .finish(
                    &mut store,
                    &scope,
                    inputs["response"].as_str().unwrap(),
                    false,
                )
                .unwrap();
            let mut golden = expected["result"].clone();
            golden.as_object_mut().unwrap().remove("inputTokens");
            golden.as_object_mut().unwrap().remove("outputTokens");
            assert_eq!(result, golden);
            let mut changed = scope.clone();
            changed.runtime_fingerprint = "changed".into();
            assert!(draft.request_for(&changed).is_err());
            assert!(draft.finish(&mut store, &scope, "{}", true).is_err());
        }
        assert_eq!(
            json!(store.records),
            expected["records"],
            "draft artifacts {}",
            input["id"]
        );
    }
}

#[test]
fn emergency_retry_transforms_match_reference_and_pair_guard_rejects_orphan_suffix() {
    use ag1_core::context::{compaction, retry};
    let inputs: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/compaction-inputs.json"
    ))
    .unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/retry-oracle.json"
    ))
    .unwrap();
    let messages = |id: &Value| {
        inputs["histories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == *id)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
    };
    for row in oracle["groups"].as_array().unwrap() {
        assert_eq!(
            retry::head(messages(&row["id"]), row["error"].as_str().unwrap(), 1000)
                .unwrap()
                .unwrap_or(Value::Null),
            row["result"]
        );
    }
    for row in oracle["snips"].as_array().unwrap() {
        let mut expected = row.clone();
        expected.as_object_mut().unwrap().remove("id");
        assert_eq!(
            retry::snip(messages(&row["id"]), 1000, 2, 1000).unwrap(),
            expected
        );
    }
    let entries = vec![
        json!({"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"c","name":"read","arguments":{}}]}}),
        json!({"type":"custom","data":{}}),
        json!({"type":"message","message":{"role":"toolResult","toolCallId":"c","content":[{"type":"text","text":"exact evidence"}]}}),
    ];
    assert_eq!(compaction::close_pair_boundary(&entries, 2), 0);
    assert!(!retry::complete_pairs(&[entries[2]["message"].clone()]));
    assert!(retry::complete_pairs(&[
        entries[0]["message"].clone(),
        entries[2]["message"].clone()
    ]));
}

#[test]
fn checklist_reconciliation_and_per_turn_addenda_match_sdk() {
    use ag1_core::context::projections::{self, TurnAddenda};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/context/addenda-oracle.json"
    ))
    .unwrap();
    for row in oracle["plans"].as_array().unwrap() {
        let task = row["input"]
            .get("task")
            .map(ag1_core::task::TaskState::parse)
            .transpose()
            .unwrap();
        assert_eq!(
            json!(projections::task_plan(
                row["input"]["branch"].as_array().unwrap(),
                task.as_ref()
            )),
            row["output"]
        );
    }
    for row in oracle["turns"].as_array().unwrap() {
        let i = &row["input"];
        assert_eq!(
            json!(projections::turn(TurnAddenda {
                runtime_pending: i["runtimePending"].as_str(),
                verification_command: i["verificationCommand"].as_str(),
                todo: i["todoAddendum"].as_str(),
                memory: None,
                orientation: None,
                finished_jobs: None
            })),
            row["output"]
        );
    }
    assert_eq!(
        projections::turn(TurnAddenda {
            runtime_pending: Some("runtime"),
            verification_command: None,
            todo: Some("todo"),
            memory: Some("memory"),
            orientation: Some("orientation"),
            finished_jobs: Some("jobs")
        })
        .unwrap(),
        "runtime\n\ntodo\n\nmemory\n\norientation\n\njobs"
    );
}
