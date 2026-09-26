//! Stateless summary/episode calls have no root transcript or tool definitions.
use crate::{
    DomainError,
    provider::{BlockKind, ProviderSnapshot, StopReason},
};
use serde_json::{Value, json};
pub struct IsolatedInput<'a> {
    pub system: &'a str,
    pub prompt: &'a str,
    pub purpose: &'a str,
    pub requested_max: Option<u64>,
    pub model_max: u64,
    pub root_session: Option<&'a str>,
}
#[derive(Clone)]
pub struct IsolatedContext {
    pub context: Value,
    pub max_tokens: u64,
    pub session_id: Option<String>,
    pub purpose: String,
}
pub fn context(input: IsolatedInput<'_>) -> Result<IsolatedContext, DomainError> {
    let system = input.system.trim();
    let prompt = input.prompt.trim();
    let purpose = input.purpose.trim();
    if system.is_empty() || prompt.is_empty() || purpose.is_empty() {
        return Err(DomainError::InvalidInput(
            "isolated prompt/system/purpose".into(),
        ));
    }
    if system.len() + prompt.len() > 16 * 1024 * 1024 || purpose.len() > 1024 {
        return Err(DomainError::LimitExceeded("isolated context".into()));
    }
    let requested = input.requested_max.unwrap_or(if input.model_max == 0 {
        4096
    } else {
        input.model_max.min(4096)
    });
    if requested == 0 {
        return Err(DomainError::InvalidInput("isolated maximum tokens".into()));
    }
    let mut suffix = String::new();
    let mut separator = false;
    for c in purpose.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || "_-".contains(c) {
            suffix.push(c);
            separator = false;
        } else if !separator {
            suffix.push('-');
            separator = true;
        }
    }
    let suffix: String = suffix.trim_matches('-').chars().take(48).collect();
    let session_id = input.root_session.map(|root| {
        format!(
            "{root}:isolated:{}",
            if suffix.is_empty() { "task" } else { &suffix }
        )
    });
    Ok(IsolatedContext {
        context: json!({"systemPrompt":system,"messages":[{"role":"user","content":[{"type":"text","text":prompt}],"timestamp":0}]}),
        max_tokens: requested.clamp(256, 16384),
        session_id,
        purpose: purpose.into(),
    })
}
pub fn accepted_text(
    snapshot: &ProviderSnapshot,
    allow_truncated: bool,
) -> Result<String, DomainError> {
    if !snapshot.settled
        || matches!(
            snapshot.stop_reason,
            None | Some(StopReason::Error | StopReason::Aborted | StopReason::ToolUse)
        )
        || snapshot.content.iter().any(|b| b.kind == BlockKind::Tool)
    {
        return Err(DomainError::Unavailable(
            "isolated call did not produce a settled analysis response".into(),
        ));
    }
    if snapshot.stop_reason == Some(StopReason::Length) && !allow_truncated {
        return Err(DomainError::Unavailable(
            "isolated output was truncated; refusing incomplete memory".into(),
        ));
    }
    let text = snapshot
        .content
        .iter()
        .filter(|b| b.kind == BlockKind::Text)
        .map(|b| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(DomainError::Unavailable(
            "isolated call returned no text".into(),
        ));
    }
    Ok(text)
}
