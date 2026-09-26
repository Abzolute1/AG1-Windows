#[path = "support/compiler_fixture.rs"]
mod fixture;
use ag1_core::{
    DomainError,
    context::{
        ContextArtifactPort,
        compiler::{CompileOptions, compile_continuation},
        retry::complete_pairs,
    },
};
use fixture::FixtureStore;
use serde_json::{Value, json};

fn options() -> CompileOptions {
    CompileOptions {
        context_window: 32_768,
        max_working_tokens: Some(8192),
        fixed_tokens: Some(512),
        ..Default::default()
    }
}
fn append(messages: &mut Vec<Value>, id: usize) {
    messages.extend([
        json!({"role":"user","content":format!("Trial {id}: {}", "Use current source and retained evidence. ".repeat(8)),"timestamp":id}),
        json!({"role":"assistant","content":[{"type":"toolCall","id":format!("read-{id}"),"name":"read","arguments":{"path":format!("src/{id}.rs")}}],"timestamp":id}),
        json!({"role":"toolResult","toolCallId":format!("read-{id}"),"toolName":"read","content":[{"type":"text","text":format!("CURRENT_{id}: exact source β\\n")}],"timestamp":id}),
        json!({"role":"assistant","content":[{"type":"text","text":format!("Trial {id}: {}", "The scoped inspection is complete. ".repeat(8))}],"timestamp":id}),
    ]);
}
fn history() -> Vec<Value> {
    let mut rows = vec![];
    for id in 0..120 {
        append(&mut rows, id);
    }
    rows
}

#[test]
fn small_completed_turns_are_bounded_with_exact_retrievable_custody() {
    let original = history();
    let mut store = FixtureStore::default();
    let result = compile_continuation(&original, original.len(), &options(), &mut store).unwrap();
    assert_eq!(result.stats["overTargetTokens"], 0, "{}", result.stats);
    assert!(complete_pairs(&result.messages));
    assert_eq!(
        &result.messages[result.messages.len() - 8..],
        &original[original.len() - 8..]
    );
    let mut recovered: Vec<Value> = result
        .messages
        .iter()
        .filter(|m| m["customType"] != "ag1-history-custody")
        .cloned()
        .collect();
    let archives: Vec<_> = store
        .records
        .iter()
        .filter(|r| r["kind"] == "context-history-segment")
        .collect();
    assert!(!archives.is_empty());
    for archive in archives {
        let bytes = &store.texts[archive["id"].as_str().unwrap()];
        let value: Value = serde_json::from_str(bytes).unwrap();
        recovered.extend(value["messages"].as_array().unwrap().iter().cloned());
        assert_eq!(archive["producer"]["type"], "session");
        assert!(result.messages.iter().any(|m| {
            m["content"]
                .as_str()
                .is_some_and(|s| s.contains(archive["id"].as_str().unwrap()))
        }));
    }
    let sorted = |rows: &[Value]| {
        let mut rows: Vec<_> = rows.iter().map(Value::to_string).collect();
        rows.sort();
        rows
    };
    assert_eq!(
        sorted(&recovered),
        sorted(&original),
        "no historical bytes may disappear"
    );
    assert_eq!(original, history(), "caller history is immutable");
    let repeated = compile_continuation(
        &result.messages,
        result.messages.len(),
        &options(),
        &mut store,
    )
    .unwrap();
    assert_eq!(
        repeated.messages, result.messages,
        "no-pressure continuation is stable"
    );
}

#[test]
fn archive_failure_never_publishes_a_reduced_projection() {
    let original = history();
    let mut store = FixtureStore {
        fail: true,
        ..Default::default()
    };
    assert_eq!(
        compile_continuation(&original, original.len(), &options(), &mut store).unwrap_err(),
        DomainError::DurabilityUnknown
    );
    assert_eq!(original, history());
}

#[test]
fn historical_leases_and_unsettled_calls_survive_small_message_pressure() {
    let mut original = history();
    let mut store = FixtureStore::default();
    let leased = store
        .put_text(
            "Decisive leased source β",
            "test-output",
            "check",
            "old-check",
            &[],
        )
        .unwrap();
    store.leases.insert(leased.id.clone());
    original[2]["details"] = json!({"id":leased.id});
    // The following old call has no terminal result and cannot be archived.
    original.remove(6);
    let result = compile_continuation(&original, original.len(), &options(), &mut store).unwrap();
    assert_eq!(result.stats["overTargetTokens"], 0, "{}", result.stats);
    for row in [&original[1], &original[2], &original[5]] {
        assert!(result.messages.contains(row));
    }
    assert!(complete_pairs(&result.messages));
    assert_eq!(store.texts[&leased.id], "Decisive leased source β");
}

#[test]
fn successive_small_turns_keep_a_bounded_history_reference_chain() {
    let mut messages = vec![];
    let mut store = FixtureStore::default();
    for id in 0..180 {
        let prefix = messages.len();
        append(&mut messages, id);
        let result = compile_continuation(&messages, prefix, &options(), &mut store).unwrap();
        assert_eq!(
            result.stats["overTargetTokens"], 0,
            "trial {id}: {}",
            result.stats
        );
        assert!(complete_pairs(&result.messages));
        assert!(
            result
                .messages
                .iter()
                .filter(|m| m["customType"] == "ag1-history-custody")
                .count()
                <= 1
        );
        messages = result.messages;
    }
    assert!(
        store
            .records
            .iter()
            .filter(|r| r["kind"] == "context-history-segment")
            .count()
            > 2
    );
}
