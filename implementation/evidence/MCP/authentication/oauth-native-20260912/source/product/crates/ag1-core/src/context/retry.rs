//! Legacy stateless-summary retry transforms. Never apply these directly to the durable journal.
//! The first analysis group may be omitted by the pinned reference algorithm; the canonical
//! task/pinned request and raw trajectory remain separate inputs to the summary transaction.
use super::{estimate_tokens, string, validate_history};
use crate::{DomainError, task::invalid};
use regex::Regex;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::LazyLock};
const MARKER: &str = "[earlier conversation truncated for compaction retry]";
pub const MAX_RETRIES: usize = 3;
fn gap(error: &str) -> Option<u64> {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"(?i)max.*?(\d+).*?current.*?(\d+)",
            r"(?i)(\d+)\s*tokens?\s*over.*?(\d+)",
            r"(?i)input\s*length\s*(\d+)\s*[>]+\s*max\s*(\d+)",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
    });
    for (i, p) in PATTERNS.iter().enumerate() {
        if let Some(c) = p.captures(error) {
            let a = c[1].parse::<u64>().ok()?;
            let b = c[2].parse::<u64>().ok()?;
            let gap = match i {
                0 => b.checked_sub(a),
                1 => Some(a),
                _ => a.checked_sub(b),
            };
            if gap.is_some_and(|n| n > 0) {
                return gap;
            }
        }
    }
    None
}
pub fn head(messages: &[Value], error: &str, now: u64) -> Result<Option<Value>, DomainError> {
    validate_history(messages)?;
    if error.len() > 1024 * 1024 {
        return Err(invalid("summary retry diagnostic cap"));
    }
    let clean: Vec<_> = messages
        .iter()
        .filter(|m| {
            !(m["role"] == "user" && m["content"].as_str().is_some_and(|s| s.contains(MARKER)))
        })
        .cloned()
        .collect();
    let mut groups = vec![];
    let mut i = 0;
    while i < clean.len() {
        let start = i;
        if clean[i]["role"] == "user" {
            i += 1;
            if i < clean.len() && clean[i]["role"] == "assistant" {
                i += 1;
            }
            while i < clean.len() && clean[i]["role"] == "toolResult" {
                i += 1;
            }
        } else {
            i += 1;
        }
        groups.push((start, i));
    }
    if groups.len() < 4 {
        return Ok(None);
    }
    let droppable = &groups[1..groups.len() - 1];
    let mut count = if let Some(gap) = gap(error) {
        let mut sum = 0;
        let mut n = 0;
        for (start, end) in droppable {
            sum += clean[*start..*end].iter().map(estimate_tokens).sum::<u64>();
            n += 1;
            if sum >= gap {
                break;
            }
        }
        n
    } else {
        droppable.len().div_ceil(5).max(1)
    };
    count = count.min(droppable.len() - 1);
    if count == 0 {
        return Ok(None);
    }
    let cut = droppable[count - 1].1;
    let dropped: u64 = clean[..cut].iter().map(estimate_tokens).sum();
    let mut retained = clean[cut..].to_vec();
    if retained[0]["role"] == "assistant" {
        retained.insert(0, json!({"role":"user","content":MARKER,"timestamp":now}));
    }
    Ok(Some(json!({"messages":retained,"droppedTokens":dropped})))
}
pub fn snip(messages: &[Value], target: u64, keep: usize, now: u64) -> Result<Value, DomainError> {
    validate_history(messages)?;
    let mut protected = BTreeSet::new();
    if !messages.is_empty() {
        protected.insert(0);
    }
    if let Some(i) = messages.iter().position(|m| m["role"] == "user") {
        protected.insert(i);
    }
    for i in messages
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, m)| m["role"] == "user")
        .take(keep)
        .map(|(i, _)| i)
    {
        protected.extend(i..messages.len());
    }
    let mut groups = vec![];
    let mut i = 0;
    while i < messages.len() {
        if protected.contains(&i) {
            i += 1;
            continue;
        }
        let start = i;
        if messages[i]["role"] == "user" {
            i += 1;
            if i < messages.len() && messages[i]["role"] == "assistant" && !protected.contains(&i) {
                i += 1;
            }
            while i < messages.len()
                && messages[i]["role"] == "toolResult"
                && !protected.contains(&i)
            {
                i += 1;
            }
        } else {
            while i < messages.len() && !protected.contains(&i) && messages[i]["role"] != "user" {
                i += 1;
            }
        }
        groups.push((
            start,
            i,
            messages[start..i].iter().map(estimate_tokens).sum::<u64>(),
        ));
    }
    let mut freed = 0;
    let mut removed = BTreeSet::new();
    let mut insert = None;
    for (start, end, tokens) in groups {
        if freed >= target {
            break;
        }
        freed += tokens;
        insert.get_or_insert(start);
        removed.extend(start..end);
    }
    let snipped: Vec<_> = messages
        .iter()
        .enumerate()
        .filter(|(i, _)| removed.contains(i))
        .map(|(_, m)| m.clone())
        .collect();
    let mut out: Vec<_> = messages
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(i))
        .map(|(_, m)| m.clone())
        .collect();
    if let Some(index) = insert {
        out.insert(index,json!({"role":"user","content":format!("[Previous conversation context removed to free tokens. ~{}K tokens freed.]",(freed+500)/1000),"timestamp":now}));
    }
    Ok(json!({"messages":out,"freedTokens":freed,"snippedMessages":snipped}))
}
pub fn is_prompt_too_long(error: &str) -> bool {
    let e = error.to_lowercase();
    [
        "prompt too long",
        "context length",
        "max context",
        "input length",
    ]
    .iter()
    .any(|s| e.contains(s))
        || e.contains("max") && e.contains("current")
}
/// Checks whether a transformed suffix contains a result with no matching retained call.
/// The runtime must use a complete-pair compiler projection instead of admitting that suffix.
pub fn complete_pairs(messages: &[Value]) -> bool {
    let mut calls = BTreeSet::new();
    for m in messages {
        if m["role"] == "assistant" {
            for b in super::blocks(&m["content"]) {
                if b["type"] == "toolCall" {
                    calls.insert(string(b, "id"));
                }
            }
        } else if m["role"] == "toolResult" && !calls.contains(string(m, "toolCallId")) {
            return false;
        }
    }
    true
}
