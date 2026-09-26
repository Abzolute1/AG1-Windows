//! Request-only conversion of SDK agent messages. Durable journal values retain
//! their original roles and metadata (coding-agent/src/core/messages.ts).
use crate::{DomainError, context::validate_history};
use serde_json::{Value, json};

pub fn to_provider(messages: &[Value]) -> Result<Vec<Value>, DomainError> {
    validate_history(messages)?;
    let mut output = Vec::with_capacity(messages.len());
    for message in messages {
        let text = |key: &str| message[key].as_str().unwrap_or("");
        let content = match text("role") {
            "user" | "assistant" | "toolResult" => {
                output.push(message.clone());
                continue;
            }
            "custom" => match &message["content"] {
                Value::String(text) => json!([{"type":"text","text":text}]),
                content => content.clone(),
            },
            "branchSummary" => json!([{"type":"text","text":format!(
                "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n{}</summary>", text("summary"))}]),
            "compactionSummary" => json!([{"type":"text","text":format!(
                "The conversation history before this point was compacted into the following summary:\n\n<summary>\n{}\n</summary>", text("summary"))}]),
            "bashExecution" => {
                if message["excludeFromContext"] == true {
                    continue;
                }
                let mut body = format!("Ran `{}`\n", text("command"));
                if text("output").is_empty() {
                    body.push_str("(no output)");
                } else {
                    body.push_str(&format!("```\n{}\n```", text("output")));
                }
                if message["cancelled"] == true {
                    body.push_str("\n\n(command cancelled)");
                } else if let Some(code) = message["exitCode"].as_i64().filter(|n| *n != 0) {
                    body.push_str(&format!("\n\nCommand exited with code {code}"));
                }
                if message["truncated"] == true && !text("fullOutputPath").is_empty() {
                    body.push_str(&format!(
                        "\n\n[Output truncated. Full output: {}]",
                        text("fullOutputPath")
                    ));
                }
                json!([{"type":"text","text":body}])
            }
            _ => continue,
        };
        let mut projected = json!({"role":"user","content":content});
        if let Some(timestamp) = message.get("timestamp") {
            projected["timestamp"] = timestamp.clone();
        }
        output.push(projected);
    }
    Ok(output)
}
