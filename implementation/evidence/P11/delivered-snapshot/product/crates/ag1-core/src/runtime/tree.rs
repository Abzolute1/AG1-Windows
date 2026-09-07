//! Tree navigation projections and lighter branch-summary hygiene from the
//! pinned SDK. Selection, original custody and inference remain host owned.
use crate::{
    DomainError,
    context::{self, compaction, hygiene, summary_request},
    isolated,
    storage::StoredJournal,
    task::{branch_values, invalid},
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub struct Navigation {
    pub branch: Vec<Value>,
    pub entries: Vec<Value>,
    pub common_ancestor: Option<String>,
    pub editor_text: Option<String>,
}
pub fn collect(journal: &StoredJournal, target: &str) -> Result<Navigation, DomainError> {
    let old = branch_values(journal)?;
    let mut selected = journal.clone();
    selected.leaf = Some(target.into());
    let mut branch = branch_values(&selected)?;
    let entry = branch.last().ok_or(DomainError::NotFound)?;
    let common = old
        .iter()
        .zip(&branch)
        .take_while(|(a, b)| a["id"] == b["id"])
        .count();
    let common_ancestor = common
        .checked_sub(1)
        .and_then(|i| old[i]["id"].as_str())
        .map(str::to_owned);
    let editor_text = if entry["type"] == "message" && entry["message"]["role"] == "user" {
        Some(hygiene::content(&entry["message"], ""))
    } else if entry["type"] == "custom_message" {
        Some(hygiene::content(entry, ""))
    } else {
        None
    };
    if editor_text.is_some() {
        branch.pop();
    }
    Ok(Navigation {
        branch,
        entries: old[common..].to_vec(),
        common_ancestor,
        editor_text,
    })
}
pub fn prepare(journal: &StoredJournal, target: &str) -> Result<Navigation, DomainError> {
    let navigation = collect(journal, target)?;
    // A navigation target may not make an executed call look unattempted.
    // Require a closed call/result boundary before accepting the selection.
    let mut pending = BTreeSet::new();
    for entry in &navigation.branch {
        let message = &entry["message"];
        if message["role"] == "assistant" {
            for block in message["content"].as_array().into_iter().flatten() {
                if block["type"] == "toolCall" {
                    pending.insert(
                        block["id"]
                            .as_str()
                            .ok_or_else(|| invalid("tree tool call"))?,
                    );
                }
            }
        } else if message["role"] == "toolResult" {
            pending.remove(message["toolCallId"].as_str().unwrap_or(""));
        }
    }
    if !pending.is_empty() {
        return Err(DomainError::Unavailable(
            "tree target has unresolved tool calls".into(),
        ));
    }
    Ok(navigation)
}

pub struct Summary {
    pub messages: Vec<Value>,
    pub read: BTreeSet<String>,
    pub written: BTreeSet<String>,
    pub edited: BTreeSet<String>,
    pub tokens: u64,
}
pub fn summary(entries: &[Value], budget: i64) -> Result<Summary, DomainError> {
    let mut output = Summary {
        messages: vec![],
        read: BTreeSet::new(),
        written: BTreeSet::new(),
        edited: BTreeSet::new(),
        tokens: 0,
    };
    for entry in entries {
        if entry["type"] == "branch_summary" && entry["fromHook"] != true {
            for (key, list) in [
                ("readFiles", &mut output.read),
                ("modifiedFiles", &mut output.edited),
            ] {
                list.extend(
                    entry["details"][key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
            }
        }
    }
    for entry in entries.iter().rev() {
        let message = if entry["type"] == "compaction" {
            let timestamp = entry["timestamp"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.timestamp_millis())
                .unwrap_or(0);
            Some(
                json!({"role":"compactionSummary","summary":entry["summary"],"tokensBefore":entry["tokensBefore"],"timestamp":timestamp}),
            )
        } else {
            compaction::entry_message(entry)
        };
        let Some(message) = message.filter(|m| m["role"] != "toolResult") else {
            continue;
        };
        if message["role"] == "assistant" {
            for call in message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "toolCall")
            {
                let Some(path) = call["arguments"]["path"].as_str().filter(|p| !p.is_empty())
                else {
                    continue;
                };
                match call["name"].as_str() {
                    Some("read") => {
                        output.read.insert(path.into());
                    }
                    Some("write") => {
                        output.written.insert(path.into());
                    }
                    Some("edit") => {
                        output.edited.insert(path.into());
                    }
                    _ => {}
                }
            }
        }
        let tokens = context::estimate_tokens(&message);
        if budget > 0 && output.tokens + tokens > budget as u64 {
            if ["compaction", "branch_summary"]
                .iter()
                .any(|kind| entry["type"] == *kind)
                && (output.tokens as f64) < budget as f64 * 0.9
            {
                output.messages.push(message);
                output.tokens += tokens;
            }
            break;
        }
        output.messages.push(message);
        output.tokens += tokens;
    }
    output.messages.reverse();
    Ok(output)
}
impl Summary {
    pub fn request(
        &self,
        focus: Option<&str>,
        replace: bool,
        root: &str,
        model_max: u64,
    ) -> Result<isolated::IsolatedContext, DomainError> {
        let messages = hygiene::copy(
            &self.messages,
            &hygiene::HygieneOptions {
                keep_turns: 0,
                ..Default::default()
            },
        )?;
        let conversation = summary_request::serialize(&messages);
        let mut sections = vec![];
        if !conversation.trim().is_empty() {
            sections.push(format!(
                "<conversation>\n{}\n</conversation>",
                conversation.trim()
            ));
        }
        let base = if replace && focus.is_some_and(|f| !f.is_empty()) {
            focus.unwrap()
        } else {
            if let Some(focus) = focus.map(str::trim).filter(|s| !s.is_empty()) {
                sections.push(format!("<additional-focus>\n{focus}\n</additional-focus>"));
            }
            include_str!("../context/prompts/branch_summarization_prompt.txt")
        };
        isolated::context(isolated::IsolatedInput {
            system: include_str!("../context/prompts/summarization_system_prompt.txt"),
            prompt: &format!("{}\n\n{base}", sections.join("\n\n")),
            purpose: "branch-summary",
            requested_max: Some(2048),
            model_max,
            root_session: Some(root),
        })
    }
    pub fn finish(&self, text: &str) -> Value {
        let modified: BTreeSet<_> = self.edited.union(&self.written).cloned().collect();
        let read: BTreeSet<_> = self.read.difference(&modified).cloned().collect();
        let mut summary = format!(
            "The user explored a different conversation branch before returning here.\nSummary of that exploration:\n\n{text}"
        );
        for (tag, files) in [("read-files", &read), ("modified-files", &modified)] {
            if !files.is_empty() {
                summary.push_str(&format!(
                    "\n\n<{tag}>\n{}\n</{tag}>",
                    files.iter().cloned().collect::<Vec<_>>().join("\n")
                ));
            }
        }
        json!({"summary":summary,"details":{"readFiles":read,"modifiedFiles":modified}})
    }
}
