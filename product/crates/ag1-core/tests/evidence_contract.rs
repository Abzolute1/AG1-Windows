use ag1_core::{
    evidence::{Freshness, InspectionEvidence, RuntimeObservation, SourceAccess, SourceReference},
    memory::MemoryStore,
    task::{TaskState, canonical_task, passing_check},
    todo,
};
use serde_json::{Value, json};
const NOW: &str = "2026-09-05T00:00:00.000Z";
fn oracle() -> Value {
    serde_json::from_str(include_str!(
        "../../../../fixtures/evidence/state-oracle-v2.json"
    ))
    .unwrap()
}
#[test]
fn frozen_task_memory_todo_observation_cases_match_unchanged_sdk() {
    let fixtures = oracle();
    assert_eq!(fixtures["rows"].as_array().unwrap().len(), 94);
    for (index, row) in fixtures["rows"].as_array().unwrap().iter().enumerate() {
        let i = &row["input"];
        let result: Result<Value, ag1_core::DomainError> = (|| {
            Ok(match row["kind"].as_str().unwrap() {
                "task_set" => {
                    let state = if i["state"].is_null() {
                        TaskState::empty(NOW)
                    } else {
                        TaskState::parse(&i["state"])?
                    };
                    state.set(&i["patch"], NOW)?.value().clone()
                }
                "canonical_task" => json!(canonical_task(i["text"].as_str().unwrap())),
                "task_update" => TaskState::parse(&i["state"])?
                    .update(i["id"].as_str().unwrap(), &i["patch"], NOW)?
                    .value()
                    .clone(),
                "passing_check" => json!(passing_check(
                    i["sources"].as_array().unwrap(),
                    i["journal"].as_array().unwrap()
                )),
                "memory_add" => {
                    let r = MemoryStore::parse(&i["store"])?.add(
                        i["kind"].as_str().unwrap(),
                        i["text"].as_str().unwrap(),
                        i["now"].as_i64().unwrap(),
                        i["priority"].as_str().unwrap(),
                        &i["metadata"],
                    )?;
                    let mut v = json!({"store":r.store.value(),"entry":r.entry});
                    if r.duplicate {
                        v["duplicate"] = json!(true);
                    }
                    if r.near_limit {
                        v["nearLimit"] = json!(true);
                    }
                    if !r.archived.is_empty() {
                        v["archived"] = json!(r.archived);
                    }
                    v
                }
                "memory_update" => {
                    let r = MemoryStore::parse(&i["store"])?.update(
                        i["id"].as_str().unwrap(),
                        &i["patch"],
                        i["now"].as_i64().unwrap(),
                    )?;
                    json!({"store":r.store.value(),"entry":r.entry})
                }
                "memory_select" => {
                    let status: Freshness = serde_json::from_value(i["status"].clone()).unwrap();
                    MemoryStore::parse(&i["store"])?
                        .select(i["query"].as_str().unwrap(), 1600, 16, |_| status)
                        .value()
                }
                "todo_validate" => match todo::validate_items(&i["items"]) {
                    Ok(items) => json!({"ok":true,"items":items}),
                    Err(_) => json!({"ok":false}),
                },
                "runtime_parse" => RuntimeObservation::parse(&i["value"])
                    .map(|v| v.value().clone())
                    .unwrap_or(Value::Null),
                "inspection_parse" => InspectionEvidence::parse(&i["value"])
                    .map(|v| v.value().clone())
                    .unwrap_or(Value::Null),
                other => panic!("missing oracle handler {other}"),
            })
        })();
        let expected = &row["output"];
        if expected["ok"] == false || expected["value"].get("error").is_some() {
            assert!(result.is_err(), "case {index}: expected rejection");
        } else {
            assert_eq!(
                result.unwrap(),
                expected["value"],
                "case {index} {}",
                row["kind"]
            );
        }
    }
}
#[test]
fn steering_resume_and_skill_expansion_preserve_original_objective() {
    let initial = TaskState::empty(NOW)
        .capture_instruction(
            "<skill name=\"x\">reference</skill> fix settings",
            &["build desktop".into()],
            NOW,
        )
        .unwrap();
    assert_eq!(initial.value()["objective"], "build desktop");
    let resumed = TaskState::parse(initial.value())
        .unwrap()
        .capture_instruction("continue", &[], NOW)
        .unwrap();
    assert_eq!(resumed.value()["objective"], "build desktop");
    assert_eq!(resumed.value()["latestInstruction"], "continue");
    assert_eq!(canonical_task("<skill x>unclosed"), None);
}
#[test]
fn claims_require_sources_and_links_cannot_promote_hypotheses() {
    let state = TaskState::reset("build", NOW).unwrap();
    for kind in ["FACT", "OBSERVATION", "CONSTRAINT", "RESULT", "FAILURE"] {
        assert!(
            state
                .record(
                    &json!({"kind":kind,"statement":"claimed"}),
                    "ts_00000001",
                    NOW
                )
                .is_err()
        );
    }
    let (state, id) = state
        .record(
            &json!({"kind":"HYPOTHESIS","statement":"maybe works"}),
            "ts_00000001",
            NOW,
        )
        .unwrap();
    assert!(state.set(&json!({"checkpointId":id}), NOW).is_err());
    assert!(state.update(&id, &json!({"kind":"RESULT"}), NOW).is_err());
}
#[test]
fn active_blocker_must_resolve_and_pinned_entries_survive_bounded_eviction() {
    let (state, id) = TaskState::reset("build", NOW)
        .unwrap()
        .record(
            &json!({"kind":"FAILURE","statement":"compile fails","sources":["tool-call:check1"]}),
            "ts_00000001",
            NOW,
        )
        .unwrap();
    let mut state = state
        .set(&json!({"status":"blocked","blockerId":id}), NOW)
        .unwrap();
    assert!(state.set(&json!({"status":"completed"}), NOW).is_err());
    assert!(state.remove(&id, NOW).is_err());
    for n in 2..=100 {
        state = state
            .record(
                &json!({"kind":"HYPOTHESIS","statement":format!("idea {n}")}),
                &format!("ts_{n:08x}"),
                NOW,
            )
            .unwrap()
            .0;
    }
    assert_eq!(state.value()["entries"].as_array().unwrap().len(), 50);
    assert!(
        state.value()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == id)
    );
    let state = state
        .update(&id, &json!({"status":"resolved"}), NOW)
        .unwrap();
    assert!(state.value().get("blockerId").is_none());
    assert_eq!(state.value()["status"], "active");
}
#[test]
fn compilation_checkpoint_is_invalidated_by_mutation_and_never_runtime_proof() {
    let (state, id) = TaskState::empty(NOW)
        .record(
            &json!({"kind":"RESULT","statement":"compile passed","sources":["tool-call:compile"]}),
            "ts_00000001",
            NOW,
        )
        .unwrap();
    let check = json!({"type":"message","id":"e1","message":{"role":"toolResult","toolName":"check","toolCallId":"compile","details":{"passed":true}}});
    assert!(state.checkpoint_verified(&id, std::slice::from_ref(&check)));
    for name in ["write", "edit", "bash"] {
        assert!(!state.checkpoint_verified(
            &id,
            &[
                check.clone(),
                json!({"type":"message","message":{"role":"toolResult","toolName":name}})
            ]
        ));
    }
    let v = json!({"version":1,"id":"obs_012345abcdef","feature":"UI","result":"pass","environment":"linux","revision":"target1","reporter":"agent","reproduction":"opened","notes":"looks fine","sources":["tool-call:compile"],"observedAt":NOW});
    let observation = RuntimeObservation::parse(&v).unwrap();
    assert!(!observation.verified_for("UI", "linux", "target1", |_| Freshness::Current));
    let mut v = v;
    v["reporter"] = json!("user");
    let observation = RuntimeObservation::parse(&v).unwrap();
    assert!(observation.verified_for("UI", "linux", "target1", |_| Freshness::Current));
    assert!(!observation.verified_for("UI", "linux", "target2", |_| Freshness::Current));
    assert!(!observation.verified_for("UI", "linux", "target1", |_| Freshness::Stale));
}
#[test]
fn memory_archive_dedup_core_cap_and_provenance_are_preserved() {
    let mut store = MemoryStore::default();
    for n in 0..45 {
        store = store
            .add("fact", &format!("fact {n}"), n, "normal", &json!({}))
            .unwrap()
            .store;
    }
    assert_eq!(store.value()["entries"].as_array().unwrap().len(), 40);
    assert_eq!(store.value()["archived"].as_array().unwrap().len(), 5);
    let duplicate = store
        .add("decision", "fact 0", 999, "core", &json!({}))
        .unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.store, store);
    assert!(
        store
            .add(
                "fact",
                "new fact",
                1000,
                "normal",
                &json!({"component":"core"})
            )
            .is_err()
    );
    let mut core = MemoryStore::default();
    for n in 0..40 {
        core = core
            .add("style", &format!("core {n}"), n, "core", &json!({}))
            .unwrap()
            .store;
    }
    assert!(
        core.add("fact", "extra", 100, "normal", &json!({}))
            .is_err()
    );
    assert!(
        core.select("", 1600, 16, |_| Freshness::Current)
            .entries
            .len()
            <= 16
    );
}
#[test]
fn historical_memory_is_labelled_and_excluded_from_unrelated_recent_filler() {
    let store=MemoryStore::default().add("fact","source compiler",0,"normal",&json!({"sources":["file:a@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]})).unwrap().store;
    assert!(
        store
            .select("unrelated", 1600, 16, |_| Freshness::Stale)
            .entries
            .is_empty()
    );
    let selection = store.select("compiler", 1600, 16, |_| Freshness::Stale);
    assert_eq!(selection.entries.len(), 1);
    assert!(
        ag1_core::memory::context_line(&selection.entries[0], Freshness::Stale)
            .contains("HISTORICAL STALE")
    );
    assert!(MemoryStore::parse(&json!({"version":99,"entries":[]})).is_err());
    assert!(MemoryStore::parse(&json!({"version":3,"entries":[{"bad":true}]})).is_err());
}
#[test]
fn prospective_todo_lifecycle_does_not_infer_completion() {
    let first = todo::replace(
        None,
        &json!([{"title":"build","status":"in_progress"},{"title":"inspect"}]),
        0,
        Some("plan1"),
        Some("compile"),
    )
    .unwrap();
    assert!(todo::replace(Some(&first), &json!([]), 1, None, None).is_err());
    assert!(
        todo::replace(
            None,
            &json!([{"title":"already done","status":"completed"}]),
            1,
            None,
            None
        )
        .is_err()
    );
    let second=todo::replace(Some(&first),&json!([{"id":"t1","title":"build","status":"completed"},{"id":"t2","title":"inspect","status":"in_progress"}]),2,None,None).unwrap();
    assert_eq!(second["planId"], "plan1");
    assert_eq!(todo::archive(&second, 3, true).unwrap()["snapshot"], second);
    assert_eq!(
        todo::calls_since_update(&[
            json!({"type":"message","message":{"role":"user"}}),
            json!({"type":"message","message":{"role":"toolResult","toolName":"read"}}),
            json!({"type":"message","message":{"role":"toolResult","toolName":"bash","isError":true}})
        ]),
        1
    );
}
#[test]
fn source_id_freshness_and_access_history_never_grant_current_implicitly() {
    let reference = SourceReference::new("src/a.rs", &"a".repeat(64)).unwrap();
    assert_eq!(
        SourceReference::parse(&reference.reference()).unwrap(),
        reference
    );
    for path in [
        "../outside",
        "/absolute",
        "a//b",
        "a\nb",
        "a/./b",
        "C:/outside",
    ] {
        assert!(SourceReference::new(path, &"a".repeat(64)).is_err());
    }
    let mut access = SourceAccess::default();
    for n in 0..300 {
        access
            .record(SourceReference::new(&format!("a{n}"), &"a".repeat(64)).unwrap())
            .unwrap();
    }
    let rows = access.recent(1000, |_| Freshness::Unverified);
    assert_eq!(rows.len(), 256);
    assert_eq!(rows[0].0.path(), "a299");
    assert!(
        rows.iter()
            .all(|(_, status)| *status == Freshness::Unverified)
    );
}
