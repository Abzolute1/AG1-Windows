//! Legacy request-only hygiene. CV projections bypass this pass; originals stay in durable custody.
use super::{blocks, slice_utf16, string, validate_history};
use crate::{DomainError, task::units};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct HygieneOptions {
    pub keep_turns: usize,
    pub max_tool_result_chars: usize,
    pub max_hidden_custom_chars: usize,
}
impl Default for HygieneOptions {
    fn default() -> Self {
        Self {
            keep_turns: 3,
            max_tool_result_chars: 220,
            max_hidden_custom_chars: 180,
        }
    }
}
pub fn content(message: &Value, separator: &str) -> String {
    if let Some(s) = message["content"].as_str() {
        return s.into();
    }
    blocks(&message["content"])
        .iter()
        .filter(|b| b["type"] == "text")
        .map(|b| string(b, "text"))
        .collect::<Vec<_>>()
        .join(separator)
}
fn protected(messages: &[Value], keep: usize) -> BTreeSet<usize> {
    let user = messages.iter().rposition(|m| m["role"] == "user");
    let mut set = BTreeSet::new();
    if let Some(i) = user {
        set.insert(i);
    }
    if keep > 0 {
        let mut found = 0;
        let mut start = messages.len();
        for i in (user.map_or(0, |i| i + 1)..messages.len()).rev() {
            if messages[i]["role"] == "assistant" {
                found += 1;
                start = i;
                if found >= keep {
                    break;
                }
            }
        }
        set.extend(start..messages.len());
    }
    set
}
fn tool_name(messages: &[Value], index: usize) -> Option<&str> {
    for i in (0..index).rev() {
        if messages[i]["role"] == "user" {
            break;
        }
        if messages[i]["role"] == "assistant" {
            let offset = messages[i + 1..index]
                .iter()
                .filter(|m| m["role"] == "toolResult")
                .count();
            return blocks(&messages[i]["content"])
                .iter()
                .filter(|b| b["type"] == "toolCall" && b.get("name").is_some())
                .nth(offset)
                .and_then(|b| b["name"].as_str());
        }
    }
    None
}
fn attachment(m: &Value) -> bool {
    ["user", "custom", "toolResult"].contains(&string(m, "role"))
        && blocks(&m["content"]).iter().any(|b| b["type"] != "text")
}
fn attachment_marker(m: &Value) -> &'static str {
    if blocks(&m["content"]).iter().any(|b| b["type"] == "image") {
        "[older image attachment omitted to preserve context]"
    } else {
        "[older non-text attachment omitted to preserve context]"
    }
}
fn hidden(m: &Value) -> &'static str {
    match string(m, "customType") {
        "runtime_continuation_state" => {
            "[older hidden continuation state omitted to preserve context]"
        }
        "post_tool_continuation_state" => {
            "[older hidden tool continuation state omitted to preserve context]"
        }
        "tool_failure_warning" => "[older hidden tool failure warning omitted to preserve context]",
        _ => "[older hidden runtime message omitted to preserve context]",
    }
}
fn text_only(m: &Value, text: String) -> Value {
    let mut out = m.clone();
    out["content"] = json!([{"type":"text","text":text}]);
    out
}
fn with_attachment(text: String, marker: &str) -> String {
    if text.trim().is_empty() {
        marker.into()
    } else {
        format!("{text}\n\n{marker}")
    }
}
fn without_thinking(m: &Value) -> Value {
    let mut out = m.clone();
    let mut c: Vec<_> = blocks(&m["content"])
        .iter()
        .filter(|b| b["type"] != "thinking")
        .cloned()
        .collect();
    if c.is_empty() {
        c.push(json!({"type":"text","text":"[older reasoning omitted to preserve context]"}));
    }
    out["content"] = c.into();
    out
}
pub fn copy(messages: &[Value], options: &HygieneOptions) -> Result<Vec<Value>, DomainError> {
    validate_history(messages)?;
    let protected = protected(messages, options.keep_turns);
    let mut out = messages.to_vec();
    for (i, m) in messages.iter().enumerate() {
        if protected.contains(&i) {
            continue;
        }
        match string(m, "role") {
            "toolResult" => {
                let name = tool_name(messages, i);
                let text = content(m, "");
                let compact = name.is_some_and(|n| {
                    [
                        "read",
                        "bash",
                        "grep",
                        "glob",
                        "webSearch",
                        "webFetch",
                        "cat",
                        "ls",
                        "find",
                    ]
                    .contains(&n)
                }) && units(&text) > options.max_tool_result_chars;
                let strip = attachment(m);
                if !compact && !strip {
                    continue;
                }
                let mut next = if compact {
                    format!(
                        "[truncated] {} result: {}... [original was {} chars]",
                        name.unwrap(),
                        slice_utf16(&text, 120).replace('\n', " "),
                        units(&text)
                    )
                } else {
                    text
                };
                if strip {
                    next = with_attachment(next, attachment_marker(m));
                }
                out[i] = text_only(m, next);
            }
            "user" | "custom" => {
                if attachment(m) {
                    out[i] = text_only(m, with_attachment(content(m, "\n"), attachment_marker(m)));
                }
                if m["role"] == "custom"
                    && m["display"] == false
                    && units(&content(m, "\n")) > options.max_hidden_custom_chars
                {
                    out[i] = text_only(m, hidden(m).into());
                }
            }
            "assistant"
                if blocks(&m["content"])
                    .iter()
                    .any(|b| b["type"] == "thinking") =>
            {
                out[i] = without_thinking(m);
            }
            _ => {}
        }
    }
    Ok(out)
}
pub fn strip_old_thinking(messages: &[Value]) -> Result<Vec<Value>, DomainError> {
    validate_history(messages)?;
    let last = messages.iter().rposition(|m| m["role"] == "assistant");
    Ok(messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if Some(i) != last
                && m["role"] == "assistant"
                && blocks(&m["content"])
                    .iter()
                    .any(|b| b["type"] == "thinking")
            {
                without_thinking(m)
            } else {
                m.clone()
            }
        })
        .collect())
}
pub fn live(messages: &[Value], now: u64) -> Vec<Value> {
    let mut trailing = vec![];
    for m in messages.iter().rev() {
        if m["role"] == "toolResult" {
            trailing.push(m);
        } else if !(m["role"] == "custom" && m["display"] == false) {
            break;
        }
    }
    trailing.reverse();
    if !trailing
        .iter()
        .any(|m| m["isError"] != true && units(content(m, "").trim()) >= 500)
    {
        return messages.to_vec();
    }
    let text = if trailing.len() == 1 {
        let m = trailing[0];
        let name = m["toolName"].as_str().unwrap_or("tool");
        let detail = ["url", "path", "query", "command"]
            .iter()
            .find_map(|k| m["details"][k].as_str());
        let described = detail
            .filter(|s| !s.is_empty())
            .map(|s| format!("{name} around `{s}`"))
            .unwrap_or(name.into());
        format!(
            "Runtime continuation: The bulky {described} output above should be compressed into a compact working state before continuing. Summarize the task tool output above and continue with your task. Reuse the result above instead of quoting it verbatim or rerunning the same broad tool step. If it already answers the user's request, answer now. Otherwise extract only the decisive facts, paths, symbols, errors, or URLs needed for one next step."
        )
    } else {
        "Runtime continuation: The bulky tool outputs above should be compressed into a compact working state before continuing. Summarize the task tool output above and continue with your task. Reuse the results above instead of quoting them verbatim or restarting broad discovery. If the tool results already answer the user's request, answer now. Otherwise extract only the decisive facts, paths, symbols, errors, or URLs needed for one next step.".into()
    };
    let mut out = messages.to_vec();
    out.push(json!({"role":"custom","customType":"post_tool_continuation_state","display":false,"content":[{"type":"text","text":text}],"details":{"kind":"post_tool_continuation_state"},"timestamp":now}));
    out
}
pub fn provider_working_set(
    messages: &[Value],
    cv: bool,
    options: &HygieneOptions,
    now: u64,
) -> Result<Vec<Value>, DomainError> {
    validate_history(messages)?;
    if cv {
        Ok(messages.to_vec())
    } else {
        Ok(live(&copy(messages, options)?, now))
    }
}
