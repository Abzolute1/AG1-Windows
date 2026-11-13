use ag1_core::runtime::{context, messages};
use serde_json::{Value, json};

fn tool_round(id: &str) -> [Value; 2] {
    [
        json!({"role":"assistant","content":[{"type":"toolCall","id":id,"name":"read","arguments":{"path":"file.rs"}}]}),
        json!({"role":"toolResult","toolCallId":id,"toolName":"read","content":[{"type":"text","text":"source"}],"isError":false}),
    ]
}

#[test]
fn changing_working_state_preserves_all_earlier_messages_across_tool_rounds() {
    let mut journal =
        vec![json!({"role":"user","content":[{"type":"text","text":"Review this file"}]})];
    journal.extend(tool_round("read-1"));
    let first_journal = journal.clone();
    let first = context::append_working_state(&journal, Some("working-state: current")).unwrap();
    let first_provider = messages::to_provider(&first).unwrap();
    assert_eq!(journal, first_journal);
    assert_eq!(
        &first_provider[..journal.len() - 1],
        &journal[..journal.len() - 1]
    );
    assert_eq!(first_provider.len(), journal.len());
    assert_eq!(first_provider.last().unwrap()["role"], "toolResult");
    assert_eq!(first_provider.last().unwrap()["toolCallId"], "read-1");
    assert_eq!(
        first_provider.last().unwrap()["content"][0],
        journal.last().unwrap()["content"][0]
    );

    journal.extend(tool_round("read-2"));
    let second_journal = journal.clone();
    let second = context::append_working_state(&journal, Some("working-state: stale")).unwrap();
    assert_eq!(
        context::latest_query(&second).as_deref(),
        Some("Review this file")
    );
    let second_provider = messages::to_provider(&second).unwrap();
    assert_eq!(journal, second_journal);
    assert_eq!(
        &second_provider[..first_journal.len() - 1],
        &first_provider[..first_journal.len() - 1]
    );
    assert_eq!(
        &second_provider[..journal.len() - 1],
        &journal[..journal.len() - 1]
    );
    assert_eq!(second_provider.len(), journal.len());
    assert_eq!(
        second_provider
            .iter()
            .filter(|m| m["role"] == "user")
            .count(),
        1
    );
    assert_eq!(second_provider.last().unwrap()["toolCallId"], "read-2");
    assert!(ag1_core::context::retry::complete_pairs(&second_provider));
    for at in [1, 3] {
        assert_eq!(second_provider[at]["role"], "assistant");
        assert_eq!(second_provider[at + 1]["role"], "toolResult");
    }
    let wire_text = serde_json::to_string(&second_provider).unwrap();
    assert_eq!(wire_text.matches("working-state: stale").count(), 1);
    assert!(!wire_text.contains("working-state: current"));
}

#[test]
fn refreshing_a_transient_tail_replaces_it_without_duplicating_or_changing_history() {
    let original = vec![json!({"role":"user","content":"hello"})];
    let first = context::append_working_state(&original, Some("old state")).unwrap();
    let refreshed = context::append_working_state(&first, Some("new state")).unwrap();
    assert_eq!(&refreshed[..original.len()], original.as_slice());
    assert_eq!(refreshed.len(), original.len() + 1);
    assert_eq!(refreshed.last().unwrap()["content"], "new state");
    assert_eq!(
        context::append_working_state(&original, None).unwrap(),
        original
    );
    assert_eq!(
        context::append_working_state(&refreshed, None).unwrap(),
        refreshed
    );
}
