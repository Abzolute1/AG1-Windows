//! Stateless summary requests, executed sequentially through the P06 scheduler by the host.
use super::{
    blocks,
    hygiene::{self, HygieneOptions},
    slice_utf16, string, working_memory,
};
use crate::{
    DomainError,
    isolated::{self, IsolatedContext, IsolatedInput},
    task::units,
};
use serde_json::Value;
const INITIAL: &str = include_str!("prompts/summarization_prompt.txt");
const UPDATE: &str = include_str!("prompts/update_summarization_prompt.txt");
const PREFIX: &str = include_str!("prompts/turn_prefix_summarization_prompt.txt");
const SYSTEM: &str = include_str!("prompts/summarization_system_prompt.txt");
fn user_text(m: &Value) -> Option<String> {
    match string(m, "role") {
        "user" | "custom" => Some(hygiene::content(m, "")),
        "branchSummary" => Some(format!(
            "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n{}</summary>",
            string(m, "summary")
        )),
        "compactionSummary" => Some(format!(
            "The conversation history before this point was compacted into the following summary:\n\n<summary>\n{}\n</summary>",
            string(m, "summary")
        )),
        "bashExecution" if m["excludeFromContext"] != true => {
            let mut text = format!("Ran `{}`\n", string(m, "command"));
            if string(m, "output").is_empty() {
                text.push_str("(no output)");
            } else {
                text.push_str(&format!("```\n{}\n```", string(m, "output")));
            }
            if m["cancelled"] == true {
                text.push_str("\n\n(command cancelled)");
            } else if let Some(code) = m["exitCode"].as_i64().filter(|n| *n != 0) {
                text.push_str(&format!("\n\nCommand exited with code {code}"));
            }
            if m["truncated"] == true && !string(m, "fullOutputPath").is_empty() {
                text.push_str(&format!(
                    "\n\n[Output truncated. Full output: {}]",
                    string(m, "fullOutputPath")
                ));
            }
            Some(text)
        }
        _ => None,
    }
}
pub fn serialize(messages: &[Value]) -> String {
    let mut parts = vec![];
    for m in messages {
        if let Some(text) = user_text(m) {
            if !text.is_empty() {
                parts.push(format!("[User]: {text}"));
            }
        } else if m["role"] == "assistant" {
            let mut text = vec![];
            let mut calls = vec![];
            for b in blocks(&m["content"]) {
                if b["type"] == "text" {
                    text.push(string(b, "text"));
                } else if b["type"] == "toolCall" {
                    let args = b["arguments"]
                        .as_object()
                        .map(|a| {
                            a.iter()
                                .map(|(k, v)| format!("{k}={v}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    calls.push(format!("{}({args})", string(b, "name")));
                }
            }
            if !text.is_empty() {
                parts.push(format!("[Assistant]: {}", text.join("\n")));
            }
            if !calls.is_empty() {
                parts.push(format!("[Assistant tool calls]: {}", calls.join("; ")));
            }
        } else if m["role"] == "toolResult" {
            let text = hygiene::content(m, "");
            if !text.is_empty() {
                let clipped = if units(&text) <= 2000 {
                    text
                } else {
                    format!(
                        "{}\n\n[... {} more characters truncated]",
                        slice_utf16(&text, 2000),
                        units(&text) - 2000
                    )
                };
                parts.push(format!("[Tool result]: {clipped}"));
            }
        }
    }
    parts.join("\n\n")
}
#[derive(Default)]
pub struct SummaryRequest<'a> {
    pub messages: &'a [Value],
    pub reserve_tokens: u64,
    pub previous: Option<&'a str>,
    pub focus: Option<&'a str>,
    pub memory: Option<&'a Value>,
    pub split_prefix: bool,
    pub root_session: Option<&'a str>,
    pub model_max: u64,
}
pub fn request(input: SummaryRequest<'_>) -> Result<IsolatedContext, DomainError> {
    let messages = hygiene::copy(
        input.messages,
        &HygieneOptions {
            keep_turns: 0,
            ..Default::default()
        },
    )?;
    let conversation = serialize(&messages);
    let memory = if input.split_prefix {
        None
    } else {
        input.memory.map(working_memory::format).transpose()?
    };
    let mut sections = vec![];
    for (tag, value) in [
        ("working-memory", memory.as_deref()),
        ("conversation", Some(conversation.as_str())),
        (
            "previous-summary",
            if input.split_prefix {
                None
            } else {
                input.previous
            },
        ),
        (
            "additional-focus",
            if input.split_prefix {
                None
            } else {
                input.focus
            },
        ),
    ] {
        if let Some(value) = value.map(str::trim).filter(|s| !s.is_empty()) {
            sections.push(format!("<{tag}>\n{value}\n</{tag}>"));
        }
    }
    let base = if input.split_prefix {
        PREFIX
    } else if input.previous.is_some_and(|s| !s.is_empty()) {
        UPDATE
    } else {
        INITIAL
    };
    let prompt = format!("{}\n\n{base}", sections.join("\n\n"));
    let ratio = if input.split_prefix { 0.2 } else { 0.25 };
    let max = ((input.reserve_tokens as f64 * ratio).floor() as u64).clamp(2048, 8192);
    isolated::context(IsolatedInput {
        system: SYSTEM,
        prompt: &prompt,
        purpose: if input.split_prefix {
            "compaction-turn-prefix"
        } else {
            "compaction-summary"
        },
        requested_max: Some(max),
        model_max: input.model_max,
        root_session: input.root_session,
    })
}
