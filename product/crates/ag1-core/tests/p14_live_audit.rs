use ag1_core::tools::{
    approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
    policy::Policy,
    registry::{Origin, Registry, Scope},
};
use serde_json::json;

#[test]
fn late_review_gets_full_execution_lifetime_and_replay_is_rejected() {
    let mut registry = Registry::builtins().unwrap();
    let names = vec!["write".to_owned()];
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let policy: Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":"/project"})).unwrap();
    let scope = Scope {
        workspace: "workspace".into(),
        session: "session".into(),
        epoch: 1,
        revision: 1,
    };
    for (at, allowed) in [
        (70_000, true),
        (299_999, true),
        (300_000, true),
        (70_000, false),
    ] {
        let mut book = Authorizer::new("review-test").unwrap();
        let call = registry
            .prepare(
                "op",
                "call",
                "write",
                &json!({"path":"a","content":"b"}),
                true,
            )
            .unwrap();
        let Preflight::Approval(request) = book
            .preflight(
                call,
                PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: "source",
                    now: 0,
                    ttl: 300_000,
                    untrusted_extensions: true,
                },
            )
            .unwrap()
        else {
            panic!("approval")
        };
        let fence = ExecutionFence {
            scope: &scope,
            policy_revision: 1,
            target_revision: "source",
            now: at,
        };
        assert!(
            book.answer_for_execution(
                &request.binding,
                true,
                &registry,
                ExecutionFence {
                    target_revision: "changed",
                    ..fence
                },
                60_000
            )
            .is_err()
        );
        assert!(
            book.answer_for_execution(
                &request.binding,
                true,
                &registry,
                ExecutionFence {
                    policy_revision: 2,
                    ..fence
                },
                60_000
            )
            .is_err()
        );
        let result = book.answer_for_execution(&request.binding, allowed, &registry, fence, 60_000);
        if at == 300_000 {
            assert!(result.is_err());
            continue;
        }
        let authorization = result.unwrap();
        if allowed {
            let authorization = authorization.unwrap();
            assert!(
                authorization
                    .check(
                        &registry,
                        ExecutionFence {
                            now: at + 59_999,
                            ..fence
                        }
                    )
                    .is_ok()
            );
            assert!(
                authorization
                    .check(
                        &registry,
                        ExecutionFence {
                            now: at + 60_000,
                            ..fence
                        }
                    )
                    .is_err()
            );
        } else {
            assert!(authorization.is_none());
        }
        assert!(
            book.answer_for_execution(&request.binding, true, &registry, fence, 60_000)
                .is_err()
        );
    }
}

#[test]
fn preview_requires_an_explicit_string_replacement() {
    let mut registry = Registry::builtins().unwrap();
    let names = vec!["editPreview".to_owned()];
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    for args in [
        json!({"path":"a","oldText":"x"}),
        json!({"path":"a","oldText":"x","newText":null}),
        json!({"path":"a","oldText":"x","newText":42}),
    ] {
        assert!(
            registry
                .prepare("op", "call", "editPreview", &args, true)
                .is_err()
        );
    }
    for replacement in ["", "line\r\nλ\"quoted\""] {
        assert!(
            registry
                .prepare(
                    "op",
                    "call",
                    "editPreview",
                    &json!({"path":"a","oldText":"x","newText":replacement}),
                    true
                )
                .is_ok()
        );
        let preview = ag1_core::tools::preview::preview("a", Some("x"), "x", replacement).unwrap();
        assert!(preview.error.is_none());
        assert!(!preview.hunks.is_empty());
    }
}

#[test]
fn query_returns_normalized_todo_ids_without_mutating_history() {
    let initial = ag1_core::tools::todo::prepare(
        &[],
        &json!({"todos":[{"title":"Repair","status":"in_progress"}]}),
        1,
        "plan",
    )
    .unwrap();
    let query = ag1_core::tools::todo::prepare(&initial.records, &json!({}), 2, "ignored").unwrap();
    assert!(query.records.is_empty());
    assert_eq!(query.result["details"]["todos"][0]["id"], "t1");
    assert_eq!(query.result["details"]["todos"][0]["status"], "in_progress");
    assert_eq!(
        ag1_core::tools::todo::prepare(&[], &json!({}), 1, "empty")
            .unwrap()
            .result["details"]["todos"],
        json!([])
    );
}

#[test]
fn programming_years_do_not_create_current_events_intent() {
    use ag1_core::tools::research::intent;
    assert_eq!(
        intent("Rust std io ErrorKind file handling best practices 2024"),
        "code_lookup"
    );
    assert_ne!(intent("Python 2026 latest file handling"), "current_events");
    assert_eq!(intent("API documentation 2024"), "docs_reference");
    assert_eq!(intent("latest election news"), "current_events");
    let input: ag1_core::tools::research::FetchTranscript = serde_json::from_value(json!({
        "url":"https://example.invalid/docs","profile":ag1_core::tools::research::profile(32768.,0.),
        "intent":"current_events","status":"rich","authoritative":true,
        "coverage":{"kind":"docs","present":["signature"],"missing":["parameters"],"score":1}
    })).unwrap();
    let text = ag1_core::tools::research::fetch_transcript(&input);
    assert!(!text.contains("Answer from it now"));
    assert!(text.contains("does not establish completion of the active task"));
}

#[test]
fn builtin_plan_is_recognized_and_extensions_cannot_impersonate_it() {
    let mut registry = Registry::builtins().unwrap();
    let definition = registry
        .public_tools()
        .into_iter()
        .find(|t| t["name"] == "submit_plan")
        .unwrap();
    assert!(registry.register(definition, Origin::Extension).is_err());
    let policy: Policy =
        serde_json::from_value(json!({"mode":"interactive","cwd":"/project"})).unwrap();
    let decision =
        ag1_core::tools::policy::evaluate("submit_plan", &json!({}), &policy, true).unwrap();
    assert!(decision.allow);
    assert!(decision.approval_request.is_none());
}
