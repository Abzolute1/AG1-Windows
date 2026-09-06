//! Bounded accumulation from acknowledged child journal messages. These are
//! result summaries; original tool receipts remain in the isolated journal.
use super::checkpoint::{Checkpoint, Evidence};
use crate::{
    DomainError,
    context::{hygiene::content, slice_utf16},
};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
pub struct Tracking {
    pub seen: usize,
    pub summary: String,
    pub cache_read: u64,
    pub cache_write: u64,
    pending: BTreeMap<String, (String, Value)>,
    recent: Vec<String>,
}
fn add(value: &mut u64, increment: u64) -> Result<(), DomainError> {
    *value = value
        .checked_add(increment)
        .ok_or(DomainError::LimitExceeded("subagent usage".into()))?;
    Ok(())
}
impl Tracking {
    pub fn repeats(&mut self, name: &str, args: &Value) -> bool {
        use sha2::{Digest, Sha256};
        let signature = format!("{name}:{:x}", Sha256::digest(args.to_string().as_bytes()));
        self.recent.push(signature.clone());
        if self.recent.len() > 3 {
            self.recent.remove(0);
        }
        self.recent.len() == 3 && self.recent.iter().all(|old| old == &signature)
    }
    pub fn capture(
        &mut self,
        next: &mut Checkpoint,
        branch: &[Value],
        budget: u64,
        at: &str,
    ) -> Result<(), DomainError> {
        if branch.len() < self.seen {
            return Err(DomainError::Conflict);
        }
        for entry in &branch[self.seen..] {
            let message = &entry["message"];
            if message["role"] == "assistant" {
                let usage = &message["usage"];
                add(&mut next.usage.input, usage["input"].as_u64().unwrap_or(0))?;
                add(
                    &mut next.usage.output,
                    usage["output"].as_u64().unwrap_or(0),
                )?;
                add(
                    &mut next.usage.total_tokens,
                    usage["totalTokens"].as_u64().unwrap_or(0),
                )?;
                add(
                    &mut self.cache_read,
                    usage["cacheRead"].as_u64().unwrap_or(0),
                )?;
                add(
                    &mut self.cache_write,
                    usage["cacheWrite"].as_u64().unwrap_or(0),
                )?;
                next.usage.cost += usage["cost"]["total"].as_f64().unwrap_or(0.);
                let text = content(message, "\n");
                next.add_summary(&text, next.last_turn, at)?;
                if !text.trim().is_empty() {
                    self.summary = slice_utf16(&text, 6000);
                }
                add(&mut next.last_turn, 1)?;
                for block in message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|block| block["type"] == "toolCall")
                {
                    let id = block["id"].as_str().ok_or(DomainError::Conflict)?;
                    if self.pending.len() >= 128 || self.pending.contains_key(id) {
                        return Err(DomainError::Conflict);
                    }
                    self.pending.insert(
                        id.into(),
                        (
                            block["name"].as_str().ok_or(DomainError::Conflict)?.into(),
                            block["arguments"].clone(),
                        ),
                    );
                }
            } else if message["role"] == "toolResult" {
                let id = message["toolCallId"]
                    .as_str()
                    .ok_or(DomainError::Conflict)?;
                let Some((name, args)) = self.pending.remove(id) else {
                    return Err(DomainError::Conflict);
                };
                if message["toolName"] != name {
                    return Err(DomainError::Conflict);
                }
                // Count every final call once; unsupported/failed tools must not
                // duplicate the previous evidence entry as the source did.
                let path = args["path"].as_str().or_else(|| args["file_path"].as_str());
                let evidence = match name.as_str() {
                    "bash" => Some(Evidence::Bash {
                        command: slice_utf16(args["command"].as_str().unwrap_or(""), 200),
                        exit_code: message["details"]["exitCode"].as_i64(),
                    }),
                    "write" | "edit" if message["isError"] != true => {
                        path.map(|path| Evidence::File {
                            action: name.clone(),
                            path: path.into(),
                        })
                    }
                    "read" | "grep" | "find" | "ls" | "repo" if message["isError"] != true => {
                        Some(Evidence::Inspect {
                            tool: name.clone(),
                            detail: slice_utf16(
                                path.or_else(|| args["pattern"].as_str())
                                    .or_else(|| args["action"].as_str())
                                    .unwrap_or(""),
                                200,
                            ),
                        })
                    }
                    "webFetch" if message["isError"] != true => args["url"]
                        .as_str()
                        .map(|url| Evidence::Web { url: url.into() }),
                    _ => None,
                };
                if let Some(evidence) = evidence {
                    next.add_evidence(evidence, next.last_turn.saturating_sub(1), at)?;
                } else {
                    add(&mut next.tool_calls, 1)?;
                }
            }
        }
        let ratio = next.usage.total_tokens as f64 / budget.max(1) as f64;
        next.budget_pressure = if ratio > 0.9 {
            "critical"
        } else if ratio > 0.75 {
            "high"
        } else if ratio > 0.5 {
            "medium"
        } else {
            "low"
        }
        .into();
        next.updated_at = at.into();
        next.validate()?;
        self.seen = branch.len();
        Ok(())
    }
}
