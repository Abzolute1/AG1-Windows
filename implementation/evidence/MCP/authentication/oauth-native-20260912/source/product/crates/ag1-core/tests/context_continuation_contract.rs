#[path = "support/compiler_fixture.rs"]
mod fixture;

use ag1_core::{
    DomainError,
    context::{
        ContextArtifactPort,
        compiler::{CompileOptions, compile_continuation, compile_request},
        retry::complete_pairs,
    },
};
use fixture::FixtureStore;
use serde_json::{Value, json};

fn options() -> CompileOptions {
    CompileOptions {
        context_window: 90_000,
        ..Default::default()
    }
}
fn round(messages: &mut Vec<Value>, id: usize, bytes: usize) {
    messages.push(json!({"role":"assistant","content":[{"type":"toolCall","id":format!("write-{id}"),"name":"write","arguments":{"path":format!("src/{id}.rs"),"content":"β\r\n".repeat(bytes / 4)}}],"timestamp":1}));
    messages.push(json!({"role":"toolResult","toolCallId":format!("write-{id}"),"toolName":"write","content":[{"type":"text","text":"Wrote file successfully"}],"isError":false,"timestamp":1}));
}
fn history(rounds: usize, bytes: usize) -> Vec<Value> {
    let mut messages = vec![
        json!({"role":"user","content":"Implement the project; retain exact historical evidence","timestamp":1}),
    ];
    for id in 0..rounds {
        round(&mut messages, id, bytes);
    }
    messages
}

#[test]
fn steering_and_fixed_tail_changes_do_not_rewrite_admitted_history_below_pressure() {
    let mut store = FixtureStore::default();
    let raw = history(6, 6000);
    let first = compile_request(&raw, &options(), &mut store).unwrap();
    assert_eq!(first.messages, raw);
    let mut messages = first.messages;
    for id in 6..12 {
        let prefix = messages.clone();
        messages.push(json!({"role":"user","content":format!("Steer round {id}: verify the current contract"),"timestamp":2}));
        round(&mut messages, id, 300);
        let mut opts = options();
        opts.fixed_tokens = Some(id as u64 * 100); // changing memory/working-state budget
        let compiled = compile_continuation(&messages, prefix.len(), &opts, &mut store).unwrap();
        assert!(compiled.messages.starts_with(&prefix));
        assert_eq!(compiled.stats["externalizedToolCallPayloads"], 0);
        let repeated = compile_continuation(
            &compiled.messages,
            compiled.messages.len(),
            &opts,
            &mut store,
        )
        .unwrap();
        assert_eq!(repeated.messages, compiled.messages);
        messages = compiled.messages;
    }
    assert!(store.records.is_empty());
}

#[test]
fn repeated_pressure_reduces_newer_payloads_in_batches_and_retains_exact_custody() {
    let mut store = FixtureStore::default();
    let opts = options();
    let mut messages = history(8, 12000);
    let mut reductions = 0;
    let mut quiet_rounds = 0;
    for id in 8..48 {
        let prefix = messages.clone();
        round(&mut messages, id, 12000);
        let compiled = compile_continuation(&messages, prefix.len(), &opts, &mut store).unwrap();
        assert!(complete_pairs(&compiled.messages));
        assert_eq!(
            &compiled.messages[..3],
            &prefix[..3],
            "oldest evidence should remain stable when newer candidates suffice"
        );
        assert_eq!(
            &compiled.messages[compiled.messages.len() - 6..],
            &messages[messages.len() - 6..],
            "recent three rounds remain exact"
        );
        if compiled.stats["tokensBefore"].as_u64().unwrap() > 48000 {
            reductions += 1;
            assert!(
                compiled.stats["tokensAfter"].as_u64().unwrap()
                    <= compiled.stats["compactionTargetTokens"].as_u64().unwrap(),
                "{}",
                compiled.stats
            );
        } else {
            quiet_rounds += 1;
            assert!(compiled.messages.starts_with(&prefix));
        }
        let repeat = compile_continuation(
            &compiled.messages,
            compiled.messages.len(),
            &opts,
            &mut store,
        )
        .unwrap();
        assert_eq!(
            repeat.messages, compiled.messages,
            "no-change request must be idempotent"
        );
        messages = compiled.messages;
    }
    assert!(reductions >= 2);
    assert!(
        quiet_rounds > reductions,
        "batch headroom should span multiple ordinary rounds"
    );
    for record in &store.records {
        assert_eq!(record["kind"], "tool-input");
        let payload: Value =
            serde_json::from_str(&store.texts[record["id"].as_str().unwrap()]).unwrap();
        let id = record["producer"]["callId"]
            .as_str()
            .unwrap()
            .strip_prefix("write-")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let mut exact = vec![];
        round(&mut exact, id, 12000);
        assert_eq!(payload, exact[0]["content"][0]["arguments"]);
    }
}

#[test]
fn metadata_pressure_still_reduces_but_leased_evidence_and_rehydrated_pages_survive() {
    let mut store = FixtureStore::default();
    let mut messages = history(12, 0);
    let mut leased = None;
    for id in 0..12 {
        let record = store
            .put_text(
                &format!("exact evidence {id}\r\nβ"),
                "test-output",
                "check",
                &format!("write-{id}"),
                &[],
            )
            .unwrap();
        messages[id * 2 + 2]["content"] = json!([{"type":"text","text":format!("artifact: {}\n{}\n{}",record.id,ag1_core::context::compiler::EXACT,"preview ".repeat(250))}]);
        messages[id * 2 + 2]["details"] = json!({"id":record.id});
        if id == 7 {
            leased = Some(record.id);
        }
    }
    store.leases.insert(leased.unwrap());
    messages.push(json!({"role":"user","content":"New request","timestamp":2}));
    let protected = messages[15..17].to_vec();
    let opts = CompileOptions {
        max_resident_artifact_metadata_tokens: Some(2000),
        ..options()
    };
    let result = compile_continuation(&messages, messages.len(), &opts, &mut store).unwrap();
    assert!(result.stats["evictedArtifactMetadata"].as_u64().unwrap() > 0);
    assert!(result.stats["leasedEvidenceMessages"].as_u64().unwrap() > 0);
    assert_eq!(&result.messages[15..17], protected);
    let repeated =
        compile_continuation(&result.messages, result.messages.len(), &opts, &mut store).unwrap();
    assert_eq!(repeated.messages, result.messages);
    assert_eq!(
        store.records.len(),
        12,
        "metadata reduction must not re-externalize exact sources"
    );
    let id = store.records[0]["id"].as_str().unwrap().to_owned();
    let raw = store.texts[&id].clone();
    let mut next = result.messages;
    next.push(json!({"role":"assistant","content":[{"type":"toolCall","id":"rehydrate","name":"artifact","arguments":{"action":"read","id":id}}],"timestamp":3}));
    next.push(json!({"role":"toolResult","toolCallId":"rehydrate","toolName":"artifact","content":[{"type":"text","text":raw}],"details":{"action":"read","id":id},"timestamp":3}));
    let recovered = compile_continuation(&next, next.len() - 2, &opts, &mut store).unwrap();
    assert_eq!(recovered.messages.last(), next.last());
    assert!(complete_pairs(&recovered.messages));
}

#[test]
fn invalid_frontier_durability_failure_and_irreducible_pressure_remain_explicit() {
    let mut store = FixtureStore::default();
    let messages = history(20, 12000);
    assert!(matches!(
        compile_continuation(&messages, messages.len() + 1, &options(), &mut store),
        Err(DomainError::InvalidInput(_))
    ));
    store.fail = true;
    assert_eq!(
        compile_continuation(&messages, messages.len(), &options(), &mut store).unwrap_err(),
        DomainError::DurabilityUnknown
    );
    let protected = history(3, 90000);
    store.fail = false;
    let result = compile_continuation(&protected, protected.len(), &options(), &mut store).unwrap();
    assert_eq!(result.messages, protected);
    assert!(
        result.stats["overTargetTokens"].as_u64().unwrap() > 0,
        "never hide protected overflow or disable admission limits"
    );
}

#[test]
fn increased_fixed_cost_reopens_reduction_without_stripping_recent_thinking() {
    let mut store = FixtureStore::default();
    let mut messages = history(8, 0);
    for message in &mut messages {
        if message["role"] == "assistant" {
            message["content"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"thinking","thinking":"r".repeat(15000)}));
        }
    }
    let admitted = compile_continuation(&messages, messages.len(), &options(), &mut store).unwrap();
    assert_eq!(admitted.messages, messages);
    let opts = CompileOptions {
        fixed_tokens: Some(10000),
        ..options()
    };
    let reduced = compile_continuation(&messages, messages.len(), &opts, &mut store).unwrap();
    assert!(reduced.stats["strippedThinkingMessages"].as_u64().unwrap() > 0);
    assert!(
        reduced.stats["tokensAfter"].as_u64().unwrap()
            <= reduced.stats["compactionTargetTokens"].as_u64().unwrap()
    );
    assert_eq!(&reduced.messages[..3], &messages[..3]);
    assert_eq!(
        &reduced.messages[reduced.messages.len() - 6..],
        &messages[messages.len() - 6..]
    );
    assert!(complete_pairs(&reduced.messages));
    let repeated =
        compile_continuation(&reduced.messages, reduced.messages.len(), &opts, &mut store).unwrap();
    assert_eq!(repeated.messages, reduced.messages);
}
