//! Transport-independent provider stream reduction. Fragments are observable but
//! incomplete tool arguments are never executable. EOF settles exactly once.
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiFamily {
    #[serde(rename = "anthropic-messages")]
    Anthropic,
    #[serde(rename = "openai-completions")]
    Completions,
    #[serde(rename = "mistral-conversations")]
    Mistral,
    #[serde(rename = "openai-responses")]
    Responses,
    #[serde(rename = "azure-openai-responses")]
    AzureResponses,
    #[serde(rename = "openai-codex-responses")]
    CodexResponses,
    #[serde(rename = "google-generative-ai")]
    Google,
    #[serde(rename = "google-gemini-cli")]
    GeminiCli,
    #[serde(rename = "google-vertex")]
    Vertex,
    #[serde(rename = "bedrock-converse-stream")]
    Bedrock,
}
pub const BUILTIN_FAMILIES: [ApiFamily; 10] = [
    ApiFamily::Anthropic,
    ApiFamily::Completions,
    ApiFamily::Mistral,
    ApiFamily::Responses,
    ApiFamily::AzureResponses,
    ApiFamily::CodexResponses,
    ApiFamily::Google,
    ApiFamily::GeminiCli,
    ApiFamily::Vertex,
    ApiFamily::Bedrock,
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    Text,
    Thinking,
    Tool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ContentBlock {
    pub kind: BlockKind,
    pub text: String,
    pub signature: String,
    pub id: String,
    pub name: String,
    pub arguments: Option<Value>,
    pub malformed_arguments: bool,
    pub redacted: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub total: u64,
}
#[derive(Clone, Debug, Default)]
pub struct UsagePatch {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub total: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderSnapshot {
    pub family: ApiFamily,
    pub response_id: Option<String>,
    pub content: Vec<ContentBlock>,
    pub usage: Usage,
    pub stop_reason: Option<StopReason>,
    pub error: Option<String>,
    pub settled: bool,
    pub service_tier: Option<String>,
}
#[derive(Clone, Debug)]
pub enum ProviderEvent {
    Replace {
        key: String,
        text: String,
        signature: bool,
    },
    ResponseId(String),
    ServiceTier(String),
    Begin {
        key: String,
        kind: BlockKind,
        id: String,
        name: String,
        arguments: Option<Value>,
        redacted: bool,
    },
    Delta {
        key: String,
        text: String,
    },
    Signature {
        key: String,
        text: String,
    },
    ToolIdentity {
        key: String,
        id: Option<String>,
        name: Option<String>,
    },
    End {
        key: String,
    },
    Usage(UsagePatch),
    Stop(StopReason),
    Fail(String),
}
pub struct StreamReducer {
    snapshot: ProviderSnapshot,
    keys: BTreeMap<String, usize>,
    closed: Vec<bool>,
    bytes: usize,
    limit: usize,
    start_ms: u64,
    last_progress_ms: u64,
}
impl StreamReducer {
    pub fn new(family: ApiFamily, now_ms: u64, max_bytes: usize) -> Result<Self, DomainError> {
        if !(1024..=32 * 1024 * 1024).contains(&max_bytes) {
            return Err(DomainError::InvalidInput("provider stream limit".into()));
        }
        Ok(Self {
            snapshot: ProviderSnapshot {
                family,
                response_id: None,
                content: vec![],
                usage: Usage::default(),
                stop_reason: None,
                error: None,
                settled: false,
                service_tier: None,
            },
            keys: BTreeMap::new(),
            closed: vec![],
            bytes: 0,
            limit: max_bytes,
            start_ms: now_ms,
            last_progress_ms: now_ms,
        })
    }
    pub fn snapshot(&self) -> ProviderSnapshot {
        self.snapshot.clone()
    }
    pub fn has_content(&self) -> bool {
        self.snapshot
            .content
            .iter()
            .any(|b| !b.text.is_empty() || b.kind == BlockKind::Tool)
    }
    pub fn retained_bytes(&self) -> usize {
        self.bytes
    }
    pub fn stop_reason(&self) -> Option<StopReason> {
        self.snapshot.stop_reason
    }
    pub fn watchdog_deadline_ms(&self, prefill_ms: u64, idle_ms: u64) -> Option<u64> {
        let (since, limit) = if self.has_content() {
            (self.last_progress_ms, idle_ms)
        } else {
            (self.start_ms, prefill_ms)
        };
        (limit > 0).then(|| since.saturating_add(limit))
    }
    fn charge(&mut self, count: usize) -> Result<(), DomainError> {
        if count > self.limit.saturating_sub(self.bytes) {
            return Err(DomainError::LimitExceeded("provider response".into()));
        }
        self.bytes += count;
        Ok(())
    }
    pub fn apply(&mut self, event: ProviderEvent, now_ms: u64) -> Result<(), DomainError> {
        if self.snapshot.settled {
            return Err(DomainError::Conflict);
        }
        let result = self.reduce(event, now_ms);
        if result.is_err() {
            self.snapshot.stop_reason = Some(StopReason::Error);
            self.snapshot.error = Some("invalid or oversized provider stream".into());
        }
        result
    }
    fn reduce(&mut self, event: ProviderEvent, now_ms: u64) -> Result<(), DomainError> {
        if now_ms < self.last_progress_ms {
            return Err(DomainError::InvalidInput(
                "non-monotonic provider clock".into(),
            ));
        }
        match event {
            ProviderEvent::Replace {
                key,
                text,
                signature,
            } => {
                let index = *self.keys.get(&key).ok_or(DomainError::NotFound)?;
                if self.closed[index] {
                    return Err(DomainError::Conflict);
                }
                self.charge(text.len())?;
                if signature {
                    self.snapshot.content[index].signature = text;
                } else {
                    self.snapshot.content[index].text = text;
                }
            }
            ProviderEvent::ResponseId(id) => {
                if let Some(existing) = &self.snapshot.response_id {
                    if existing != &id {
                        return Err(DomainError::Conflict);
                    }
                } else {
                    self.charge(id.len())?;
                    self.snapshot.response_id = Some(id);
                }
            }
            ProviderEvent::Begin {
                key,
                kind,
                id,
                name,
                arguments,
                redacted,
            } => {
                if self.keys.contains_key(&key) || self.keys.len() >= 1024 {
                    return Err(DomainError::InvalidInput(
                        "duplicate/too many provider blocks".into(),
                    ));
                }
                let arg_len = arguments.as_ref().map(|v| v.to_string().len()).unwrap_or(0);
                self.charge(
                    key.len()
                        + id.len()
                        + name.len()
                        + arg_len
                        + std::mem::size_of::<ContentBlock>(),
                )?;
                self.keys.insert(key, self.snapshot.content.len());
                self.closed.push(false);
                self.snapshot.content.push(ContentBlock {
                    kind,
                    text: String::new(),
                    signature: String::new(),
                    id,
                    name,
                    arguments,
                    malformed_arguments: false,
                    redacted,
                });
            }
            ProviderEvent::Delta { key, text } => return self.fragment(&key, &text, false, now_ms),
            ProviderEvent::Signature { key, text } => {
                return self.fragment(&key, &text, true, now_ms);
            }
            ProviderEvent::ToolIdentity { key, id, name } => {
                let index = *self.keys.get(&key).ok_or(DomainError::NotFound)?;
                if self.closed[index] || self.snapshot.content[index].kind != BlockKind::Tool {
                    return Err(DomainError::Conflict);
                }
                self.charge(
                    id.as_ref().map(String::len).unwrap_or(0)
                        + name.as_ref().map(String::len).unwrap_or(0),
                )?;
                let block = &mut self.snapshot.content[index];
                if let Some(id) = id {
                    if !block.id.is_empty() && block.id != id {
                        return Err(DomainError::Conflict);
                    }
                    block.id = id;
                }
                if let Some(name) = name {
                    block.name.push_str(&name);
                }
            }
            ProviderEvent::End { key } => {
                let index = *self.keys.get(&key).ok_or(DomainError::NotFound)?;
                self.close(index)?;
            }
            ProviderEvent::ServiceTier(tier) => {
                if tier.len() > 64 {
                    return Err(DomainError::LimitExceeded("provider service tier".into()));
                }
                self.snapshot.service_tier = Some(tier);
            }
            ProviderEvent::Usage(patch) => {
                let usage = &mut self.snapshot.usage;
                if let Some(v) = patch.input {
                    usage.input = v;
                }
                if let Some(v) = patch.output {
                    usage.output = v;
                }
                if let Some(v) = patch.cache_read {
                    usage.cache_read = v;
                }
                if let Some(v) = patch.cache_write {
                    usage.cache_write = v;
                }
                usage.total = patch.total.unwrap_or(
                    usage
                        .input
                        .checked_add(usage.output)
                        .and_then(|v| v.checked_add(usage.cache_read))
                        .and_then(|v| v.checked_add(usage.cache_write))
                        .ok_or_else(|| DomainError::InvalidInput("usage overflow".into()))?,
                );
            }
            ProviderEvent::Stop(reason) => {
                if self.snapshot.stop_reason != Some(StopReason::Error) {
                    self.snapshot.stop_reason = Some(reason);
                }
            }
            ProviderEvent::Fail(message) => {
                self.charge(message.len())?;
                self.snapshot.error = Some(message);
                self.snapshot.stop_reason = Some(StopReason::Error);
            }
        }
        self.last_progress_ms = now_ms;
        Ok(())
    }
    fn close(&mut self, index: usize) -> Result<(), DomainError> {
        if self.closed[index] {
            return Err(DomainError::Conflict);
        }
        let block = &mut self.snapshot.content[index];
        if block.kind == BlockKind::Tool {
            if !block.text.trim().is_empty() {
                block.arguments = serde_json::from_str(&block.text)
                    .ok()
                    .filter(Value::is_object);
            }
            block.malformed_arguments = block.arguments.as_ref().is_none_or(|v| !v.is_object())
                || block.id.is_empty()
                || block.name.is_empty();
        }
        self.closed[index] = true;
        Ok(())
    }
    fn fragment(
        &mut self,
        key: &str,
        text: &str,
        signature: bool,
        now_ms: u64,
    ) -> Result<(), DomainError> {
        if self.snapshot.settled || now_ms < self.last_progress_ms {
            return Err(DomainError::Conflict);
        }
        let index = *self.keys.get(key).ok_or(DomainError::NotFound)?;
        if self.closed[index] {
            return Err(DomainError::Conflict);
        }
        self.charge(text.len())?;
        if signature {
            self.snapshot.content[index].signature.push_str(text);
        } else {
            self.snapshot.content[index].text.push_str(text);
        }
        if !text.is_empty() {
            self.last_progress_ms = now_ms;
        }
        Ok(())
    }
    pub fn finish(&mut self, aborted: bool) -> Result<ProviderSnapshot, DomainError> {
        if self.snapshot.settled {
            return Err(DomainError::Conflict);
        }
        for index in 0..self.closed.len() {
            if !self.closed[index] {
                self.close(index)?;
            }
        }
        if aborted {
            self.snapshot.stop_reason = Some(StopReason::Aborted);
        }
        if self.snapshot.stop_reason.is_none() {
            self.snapshot.stop_reason = Some(StopReason::Error);
            self.snapshot.error = Some("provider stream ended before terminal evidence".into());
        }
        if self.snapshot.stop_reason == Some(StopReason::Stop)
            && self
                .snapshot
                .content
                .iter()
                .any(|b| b.kind == BlockKind::Tool)
        {
            self.snapshot.stop_reason = Some(StopReason::ToolUse);
        }
        self.snapshot.settled = true;
        Ok(self.snapshot())
    }
    /// Returns a cancellation reason. It does not release endpoint ownership.
    pub fn watchdog(&self, now_ms: u64, prefill_ms: u64, idle_ms: u64) -> Result<(), DomainError> {
        if self.snapshot.settled {
            return Ok(());
        }
        if now_ms < self.start_ms {
            return Err(DomainError::InvalidInput("provider clock".into()));
        }
        let (since, limit) = if self.has_content() {
            (self.last_progress_ms, idle_ms)
        } else {
            (self.start_ms, prefill_ms)
        };
        if limit > 0 && now_ms.saturating_sub(since) >= limit {
            Err(DomainError::TimedOut)
        } else {
            Ok(())
        }
    }
}
impl Usage {
    /// Rates are per million tokens, supplied by the pinned model registry.
    pub fn cost(&self, rates: [f64; 4]) -> Result<[f64; 5], DomainError> {
        if rates.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(DomainError::InvalidInput("model cost rates".into()));
        }
        let mut cost = [0.0; 5];
        for (i, tokens) in [self.input, self.output, self.cache_read, self.cache_write]
            .into_iter()
            .enumerate()
        {
            cost[i] = tokens as f64 * rates[i] / 1_000_000.0;
            cost[4] += cost[i];
        }
        if cost.iter().any(|v| !v.is_finite()) {
            return Err(DomainError::InvalidInput("model cost overflow".into()));
        }
        Ok(cost)
    }
}

impl ProviderSnapshot {
    pub fn cost(&self, rates: [f64; 4]) -> Result<[f64; 5], DomainError> {
        let multiplier = if self.family == ApiFamily::Responses {
            match self.service_tier.as_deref() {
                Some("flex") => 0.5,
                Some("priority") => 2.0,
                _ => 1.0,
            }
        } else {
            1.0
        };
        self.usage.cost(rates.map(|rate| rate * multiplier))
    }
}

fn stop(reason: Option<StopReason>) -> &'static str {
    match reason {
        Some(StopReason::Length) => "length",
        Some(StopReason::ToolUse) => "toolUse",
        Some(StopReason::Error) => "error",
        Some(StopReason::Aborted) => "aborted",
        _ => "stop",
    }
}
/// Shared lossless SDK-shaped message projection for the session runtime and optional host.
pub fn assistant_message(
    s: &ProviderSnapshot,
    model: &Value,
    timestamp: u64,
) -> Result<Value, DomainError> {
    let mut content = vec![];
    for block in &s.content {
        let mut value = match block.kind {
            BlockKind::Text => json!({"type":"text","text":block.text}),
            BlockKind::Thinking => json!({"type":"thinking","thinking":block.text}),
            BlockKind::Tool => {
                json!({"type":"toolCall","id":block.id,"name":block.name,"arguments":block.arguments.clone().unwrap_or_else(||json!({}))})
            }
        };
        if !block.signature.is_empty() {
            value[match block.kind {
                BlockKind::Thinking => "thinkingSignature",
                BlockKind::Tool => "thoughtSignature",
                BlockKind::Text => "textSignature",
            }] = json!(block.signature);
        }
        if block.kind == BlockKind::Tool
            && matches!(
                s.family,
                ApiFamily::Responses | ApiFamily::AzureResponses | ApiFamily::CodexResponses
            )
        {
            value["partialJson"] = json!(block.text);
        }
        if block.redacted {
            value["redacted"] = json!(true);
        }
        if block.malformed_arguments {
            value["_argumentsTruncated"] = json!(true);
        }
        content.push(value);
    }
    let rates = ["input", "output", "cacheRead", "cacheWrite"]
        .map(|key| model["cost"][key].as_f64().unwrap_or(0.));
    let cost = s.cost(rates)?;
    let mut result = json!({"role":"assistant","content":content,"api":s.family,"provider":model["provider"],"model":model["id"],"usage":{"input":s.usage.input,"output":s.usage.output,"cacheRead":s.usage.cache_read,"cacheWrite":s.usage.cache_write,"totalTokens":s.usage.total,"cost":{"input":cost[0],"output":cost[1],"cacheRead":cost[2],"cacheWrite":cost[3],"total":cost[4]}},"stopReason":stop(s.stop_reason),"timestamp":timestamp});
    if let Some(error) = &s.error {
        result["errorMessage"] = json!(error);
    }
    if let Some(id) = &s.response_id {
        result["responseId"] = json!(id);
    }
    Ok(result)
}
