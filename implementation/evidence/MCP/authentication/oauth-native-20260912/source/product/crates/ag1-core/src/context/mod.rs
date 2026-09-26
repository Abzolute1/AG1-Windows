//! Deterministic request projections. The input transcript remains immutable.
use crate::{
    DomainError,
    task::{invalid, units},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub mod capsule;
pub mod compiler;
pub mod episode;
pub mod episode_retrieval;
pub mod features;
pub mod salient;
pub mod signals;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ContextArtifact {
    pub id: String,
    pub kind: String,
    pub bytes: u64,
    pub sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub salient_lines: Vec<String>,
}
/// put_text acknowledges durable original bytes before returning a replacement pointer.
/// Callers use the bounded host executor and a requester-authorized P05 store.
pub trait ContextArtifactPort {
    fn put_evidence(
        &mut self,
        _text: &str,
        _kind: &str,
        _producer: &Value,
        _salient: &[String],
        _parent: Option<&str>,
    ) -> Result<ContextArtifact, DomainError> {
        Err(invalid(
            "context store does not support explicit provenance",
        ))
    }
    fn put_text(
        &mut self,
        text: &str,
        kind: &str,
        tool: &str,
        call_id: &str,
        salient: &[String],
    ) -> Result<ContextArtifact, DomainError>;
    fn get(&self, id: &str) -> Result<ContextArtifact, DomainError>;
    fn leased_ids(&self) -> Result<Vec<String>, DomainError>;
}
pub fn estimate_tokens(message: &Value) -> u64 {
    let content = &message["content"];
    let chars = match message["role"].as_str() {
        Some("user") => {
            if let Some(s) = content.as_str() {
                units(s)
            } else {
                blocks(content)
                    .iter()
                    .filter(|b| b["type"] == "text")
                    .map(|b| units(string(b, "text")))
                    .sum()
            }
        }
        Some("assistant") => blocks(content)
            .iter()
            .map(|b| match b["type"].as_str() {
                Some("text") => units(string(b, "text")),
                Some("thinking") => units(string(b, "thinking")),
                Some("toolCall") => units(string(b, "name")) + units(&b["arguments"].to_string()),
                _ => 0,
            })
            .sum(),
        Some("custom" | "toolResult") => {
            if let Some(s) = content.as_str() {
                units(s)
            } else {
                blocks(content)
                    .iter()
                    .map(|b| {
                        if b["type"] == "image" {
                            4800
                        } else if b["type"] == "text" {
                            units(string(b, "text"))
                        } else {
                            0
                        }
                    })
                    .sum()
            }
        }
        Some("bashExecution") => {
            units(string(message, "command")) + units(string(message, "output"))
        }
        Some("branchSummary" | "compactionSummary") => units(string(message, "summary")),
        _ => 0,
    };
    (chars as u64).div_ceil(3)
}
pub(crate) fn blocks(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
pub(crate) fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
pub(crate) fn slice_utf16(s: &str, cap: usize) -> String {
    if s.is_ascii() {
        return s[..cap.min(s.len())].into();
    }
    let mut used = 0;
    s.chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= cap
        })
        .collect()
}
pub(crate) fn tail_utf16(s: &str, cap: usize) -> String {
    if s.is_ascii() {
        return s[s.len().saturating_sub(cap)..].into();
    }
    let mut used = 0;
    let tail: Vec<_> = s
        .chars()
        .rev()
        .take_while(|c| {
            used += c.len_utf16();
            used <= cap
        })
        .collect();
    tail.into_iter().rev().collect()
}
pub(crate) fn commas(value: u64) -> String {
    let s = value.to_string();
    s.chars()
        .enumerate()
        .flat_map(|(i, c)| {
            if i > 0 && (s.len() - i).is_multiple_of(3) {
                vec![',', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
pub fn validate_history(messages: &[Value]) -> Result<(), DomainError> {
    if messages.len() > 100000 {
        return Err(DomainError::LimitExceeded("context message count".into()));
    }
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("context size"))?;
            if self.0 > 128 * 1024 * 1024 {
                return Err(std::io::Error::other("context size"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    for message in messages {
        if !message.is_object() || !message["role"].is_string() {
            return Err(invalid("context message"));
        }
        serde_json::to_writer(&mut counter, message)
            .map_err(|_| DomainError::LimitExceeded("context input bytes".into()))?;
    }
    Ok(())
}

pub mod code;

pub mod retrieval;

pub mod projections;

pub mod prompt;

pub mod hygiene;

pub mod compaction;

pub mod working_memory;

pub mod summary_request;

pub mod episode_draft;

pub mod retry;

// ASCII transcripts avoid Unicode decoding during whitespace normalization. Vertical tab is
// handled by the Unicode path because split_ascii_whitespace deliberately excludes it.
pub(crate) fn normalize_whitespace(s: &str) -> String {
    if s.is_ascii() && !s.as_bytes().contains(&11) {
        s.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}
