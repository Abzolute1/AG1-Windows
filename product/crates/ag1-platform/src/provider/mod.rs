//! Wire adapters for the ten API families registered by the pinned reference.
//! Credentials and HTTP live in the host; these decoders only consume bounded frames.
pub mod auth;
pub mod capability;
pub mod codex_subscription;
pub mod credential_storage;
pub mod framing;
pub mod generation;
pub mod http;
pub mod reasoning_controls;
pub mod request_http;
pub mod requests;
pub mod signing;
use ag1_core::{DomainError, provider::*};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub struct WireDecoder {
    family: ApiFamily,
    blocks: BTreeSet<String>,
    closed: BTreeSet<String>,
    current: Option<(BlockKind, String)>,
    sequence: u64,
    item_keys: BTreeMap<String, String>,
    request_scope: Option<String>,
    tool_names: BTreeMap<String, String>,
}
fn str_at<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn n(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(Value::as_u64)
}
fn delta(out: &mut Vec<ProviderEvent>, key: &str, text: &str) {
    if !text.is_empty() {
        out.push(ProviderEvent::Delta {
            key: key.into(),
            text: text.into(),
        });
    }
}
fn signature(out: &mut Vec<ProviderEvent>, key: &str, text: &str) {
    if !text.is_empty() {
        out.push(ProviderEvent::Signature {
            key: key.into(),
            text: text.into(),
        });
    }
}
fn replace_signature(out: &mut Vec<ProviderEvent>, key: &str, text: &str) {
    if !text.is_empty() {
        out.push(ProviderEvent::Replace {
            key: key.into(),
            text: text.into(),
            signature: true,
        });
    }
}
fn stop(value: &str) -> StopReason {
    match value {
        "stop" | "end" | "end_turn" | "stop_sequence" | "pause_turn" | "STOP" | "completed" => {
            StopReason::Stop
        }
        "length" | "model_length" | "max_tokens" | "MAX_TOKENS" | "incomplete" => {
            StopReason::Length
        }
        "tool_calls" | "function_call" | "tool_use" => StopReason::ToolUse,
        "cancelled" | "aborted" => StopReason::Aborted,
        _ => StopReason::Error,
    }
}
impl WireDecoder {
    pub fn new(family: ApiFamily) -> Self {
        Self {
            family,
            blocks: BTreeSet::new(),
            closed: BTreeSet::new(),
            current: None,
            sequence: 0,
            item_keys: BTreeMap::new(),
            request_scope: None,
            tool_names: BTreeMap::new(),
        }
    }
    /// A durable request identity scopes generated tool IDs across turns and restarts.
    pub fn for_request(family: ApiFamily, request_id: &str) -> Result<Self, DomainError> {
        if request_id.is_empty() || request_id.len() > 1024 {
            return Err(DomainError::InvalidInput(
                "provider request identity".into(),
            ));
        }
        use sha2::{Digest, Sha256};
        let mut decoder = Self::new(family);
        decoder.request_scope =
            Some(format!("{:x}", Sha256::digest(request_id.as_bytes()))[..24].into());
        Ok(decoder)
    }
    pub fn with_tool_names(mut self, names: &[String]) -> Result<Self, DomainError> {
        if names.len() > 1024 {
            return Err(DomainError::LimitExceeded("provider tool names".into()));
        }
        for name in names {
            if name.is_empty()
                || name.len() > 128
                || self
                    .tool_names
                    .insert(name.to_lowercase(), name.clone())
                    .is_some()
            {
                return Err(DomainError::InvalidInput("ambiguous tool name".into()));
            }
        }
        Ok(self)
    }
    fn begin(
        &mut self,
        out: &mut Vec<ProviderEvent>,
        key: &str,
        kind: BlockKind,
        id: &str,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<(), DomainError> {
        if self.blocks.len() >= 1024 || key.len() > 1024 || id.len() > 1024 || name.len() > 1024 {
            return Err(DomainError::LimitExceeded("provider block identity".into()));
        }
        if self.blocks.insert(key.into()) {
            out.push(ProviderEvent::Begin {
                key: key.into(),
                kind,
                id: id.into(),
                name: if self.family == ApiFamily::Anthropic && kind == BlockKind::Tool {
                    self.tool_names
                        .get(&name.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| name.into())
                } else {
                    name.into()
                },
                arguments,
                redacted: false,
            });
        } else if self.closed.contains(key) {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    fn end(&mut self, out: &mut Vec<ProviderEvent>, key: &str) {
        if self.blocks.contains(key) && self.closed.insert(key.into()) {
            out.push(ProviderEvent::End { key: key.into() });
        }
    }
    fn text(
        &mut self,
        out: &mut Vec<ProviderEvent>,
        kind: BlockKind,
        text: &str,
    ) -> Result<(), DomainError> {
        if text.is_empty() {
            return Ok(());
        }
        if self.current.as_ref().is_none_or(|(k, _)| *k != kind) {
            if let Some((_, key)) = self.current.take() {
                self.end(out, &key);
            }
            let key = format!("content:{}", self.sequence);
            self.sequence += 1;
            self.begin(out, &key, kind, "", "", None)?;
            self.current = Some((kind, key));
        }
        delta(
            out,
            &self.current.as_ref().expect("current content set").1,
            text,
        );
        Ok(())
    }
    /// JSON is bounded before parsing; no transport data can allocate an unbounded tree.
    pub fn decode_json(&mut self, bytes: &[u8]) -> Result<Vec<ProviderEvent>, DomainError> {
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("provider frame".into()));
        }
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|_| DomainError::InvalidInput("provider JSON frame".into()))?;
        self.decode(&value)
    }
    pub fn decode(&mut self, value: &Value) -> Result<Vec<ProviderEvent>, DomainError> {
        if !value.is_object() {
            return Err(DomainError::InvalidInput("provider object".into()));
        }
        let mut out = Vec::new();
        if let Some(error) = value.get("error").filter(|v| !v.is_null()) {
            out.push(ProviderEvent::Fail(error.to_string()));
            return Ok(out);
        }
        match self.family {
            ApiFamily::Completions | ApiFamily::Mistral => self.completions(value, &mut out)?,
            ApiFamily::Anthropic => self.anthropic(value, &mut out)?,
            ApiFamily::Responses | ApiFamily::AzureResponses | ApiFamily::CodexResponses => {
                self.responses(value, &mut out)?
            }
            ApiFamily::Google | ApiFamily::GeminiCli | ApiFamily::Vertex => {
                self.google(value, &mut out)?
            }
            ApiFamily::Bedrock => self.bedrock(value, &mut out)?,
        }
        Ok(out)
    }
    fn completions(&mut self, v: &Value, out: &mut Vec<ProviderEvent>) -> Result<(), DomainError> {
        let mistral = self.family == ApiFamily::Mistral;
        if !str_at(v, "id").is_empty() {
            out.push(ProviderEvent::ResponseId(str_at(v, "id").into()));
        }
        if let Some(u) = v.get("usage").filter(|v| !v.is_null()) {
            let write = if mistral {
                0
            } else {
                n(&u["prompt_tokens_details"], "cache_write_tokens").unwrap_or(0)
            };
            let cached = if mistral {
                0
            } else {
                n(&u["prompt_tokens_details"], "cached_tokens")
                    .unwrap_or(0)
                    .saturating_sub(write)
            };
            let input = n(u, "prompt_tokens")
                .or_else(|| n(u, "promptTokens"))
                .unwrap_or(0)
                .saturating_sub(cached)
                .saturating_sub(write);
            let reasoning = if mistral {
                0
            } else {
                n(&u["completion_tokens_details"], "reasoning_tokens").unwrap_or(0)
            };
            let output = n(u, "completion_tokens")
                .or_else(|| n(u, "completionTokens"))
                .unwrap_or(0)
                .checked_add(reasoning)
                .ok_or_else(|| DomainError::InvalidInput("completion usage overflow".into()))?;
            out.push(ProviderEvent::Usage(UsagePatch {
                input: Some(input),
                output: Some(output),
                cache_read: Some(cached),
                cache_write: Some(write),
                total: if mistral {
                    n(u, "total_tokens")
                        .or_else(|| n(u, "totalTokens"))
                        .filter(|v| *v > 0)
                } else {
                    None
                },
            }));
        }
        let choice = &v["choices"][0];
        let d = &choice["delta"];
        let reasoning = str_at(d, "reasoning_content");
        self.text(
            out,
            BlockKind::Thinking,
            if reasoning.is_empty() {
                str_at(d, "reasoning")
            } else {
                reasoning
            },
        )?;
        if !mistral
            && (!reasoning.is_empty() || !str_at(d, "reasoning").is_empty())
            && let Some((_, key)) = &self.current
        {
            replace_signature(
                out,
                key,
                if reasoning.is_empty() {
                    "reasoning"
                } else {
                    "reasoning_content"
                },
            );
        }
        if let Some(text) = d["content"].as_str() {
            self.text(out, BlockKind::Text, text)?;
        } else if let Some(parts) = d["content"].as_array() {
            for part in parts {
                if let Some(text) = part.as_str() {
                    self.text(out, BlockKind::Text, text)?;
                } else if str_at(part, "type") == "text" {
                    self.text(out, BlockKind::Text, str_at(part, "text"))?;
                } else if mistral && str_at(part, "type") == "thinking" {
                    if let Some(thinking) = part["thinking"].as_array() {
                        for p in thinking {
                            self.text(out, BlockKind::Thinking, str_at(p, "text"))?;
                        }
                    }
                } else {
                    return Err(DomainError::Unavailable(
                        "unsupported completion content part".into(),
                    ));
                }
            }
        }
        if let Some(tools) = d
            .get("tool_calls")
            .or_else(|| d.get("toolCalls"))
            .and_then(Value::as_array)
        {
            if let Some((_, key)) = self.current.take() {
                self.end(out, &key);
            }
            for tool in tools {
                let key = format!("tool:{}", n(tool, "index").unwrap_or(0));
                let existed = self.blocks.contains(&key);
                self.begin(
                    out,
                    &key,
                    BlockKind::Tool,
                    str_at(tool, "id"),
                    str_at(&tool["function"], "name"),
                    None,
                )?;
                if existed {
                    out.push(ProviderEvent::ToolIdentity {
                        key: key.clone(),
                        id: tool["id"]
                            .as_str()
                            .filter(|v| !v.is_empty())
                            .map(str::to_owned),
                        name: if mistral {
                            None
                        } else {
                            tool["function"]["name"]
                                .as_str()
                                .filter(|v| !v.is_empty())
                                .map(str::to_owned)
                        },
                    });
                }
                match &tool["function"]["arguments"] {
                    Value::String(text) => delta(out, &key, text),
                    Value::Object(_) if mistral => {
                        delta(out, &key, &tool["function"]["arguments"].to_string())
                    }
                    Value::Null => (),
                    _ => return Err(DomainError::InvalidInput("tool argument fragment".into())),
                }
            }
        }
        if let Some(reason) = choice
            .get("finish_reason")
            .or_else(|| choice.get("finishReason"))
            .and_then(Value::as_str)
        {
            out.push(ProviderEvent::Stop(stop(reason)));
        }
        Ok(())
    }
    fn anthropic(&mut self, v: &Value, out: &mut Vec<ProviderEvent>) -> Result<(), DomainError> {
        let key = format!("block:{}", n(v, "index").unwrap_or(0));
        match str_at(v, "type") {
            "message_start" => {
                if !str_at(&v["message"], "id").is_empty() {
                    out.push(ProviderEvent::ResponseId(
                        str_at(&v["message"], "id").into(),
                    ));
                }
                out.push(ProviderEvent::Usage(anthropic_usage(
                    &v["message"]["usage"],
                )));
            }
            "content_block_start" => {
                let b = &v["content_block"];
                let kind = match str_at(b, "type") {
                    "text" => BlockKind::Text,
                    "thinking" | "redacted_thinking" => BlockKind::Thinking,
                    "tool_use" => BlockKind::Tool,
                    _ => {
                        return Err(DomainError::Unavailable(
                            "unsupported Anthropic block".into(),
                        ));
                    }
                };
                self.begin(
                    out,
                    &key,
                    kind,
                    str_at(b, "id"),
                    str_at(b, "name"),
                    b.get("input").cloned(),
                )?;
                if str_at(b, "type") == "redacted_thinking" {
                    if let Some(ProviderEvent::Begin { redacted, .. }) = out.last_mut() {
                        *redacted = true;
                    }
                    delta(out, &key, "[Reasoning redacted]");
                    signature(out, &key, str_at(b, "data"));
                }
            }
            "content_block_delta" => {
                let d = &v["delta"];
                match str_at(d, "type") {
                    "text_delta" => delta(out, &key, str_at(d, "text")),
                    "thinking_delta" => delta(out, &key, str_at(d, "thinking")),
                    "input_json_delta" => delta(out, &key, str_at(d, "partial_json")),
                    "signature_delta" => signature(out, &key, str_at(d, "signature")),
                    _ => {
                        return Err(DomainError::Unavailable(
                            "unsupported Anthropic delta".into(),
                        ));
                    }
                }
            }
            "content_block_stop" => self.end(out, &key),
            "message_delta" => {
                if let Some(reason) = v["delta"]["stop_reason"].as_str() {
                    out.push(ProviderEvent::Stop(stop(reason)));
                }
                out.push(ProviderEvent::Usage(anthropic_usage(&v["usage"])));
            }
            "message_stop" | "ping" => (),
            "error" => out.push(ProviderEvent::Fail(v.to_string())),
            _ => {
                return Err(DomainError::Unavailable(
                    "unknown Anthropic stream event".into(),
                ));
            }
        }
        Ok(())
    }
    fn responses(&mut self, v: &Value, out: &mut Vec<ProviderEvent>) -> Result<(), DomainError> {
        let key = format!("output:{}", n(v, "output_index").unwrap_or(0));
        match str_at(v, "type") {
            "response.created" => {
                let id = str_at(&v["response"], "id");
                if !id.is_empty() {
                    out.push(ProviderEvent::ResponseId(id.into()));
                }
            }
            "response.output_item.added" => {
                let item = &v["item"];
                let kind = match str_at(item, "type") {
                    "message" => BlockKind::Text,
                    "reasoning" => BlockKind::Thinking,
                    "function_call" => BlockKind::Tool,
                    _ => {
                        return Err(DomainError::Unavailable(
                            "unsupported Responses item".into(),
                        ));
                    }
                };
                let id = if kind == BlockKind::Tool {
                    format!("{}|{}", str_at(item, "call_id"), str_at(item, "id"))
                } else {
                    str_at(item, "id").into()
                };
                self.begin(out, &key, kind, &id, str_at(item, "name"), None)?;
                self.item_keys
                    .insert(str_at(item, "id").into(), key.clone());
                if kind == BlockKind::Tool {
                    delta(out, &key, str_at(item, "arguments"));
                }
            }
            "response.output_text.delta"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_text.delta"
            | "response.refusal.delta"
            | "response.function_call_arguments.delta" => {
                let resolved = self
                    .item_keys
                    .get(str_at(v, "item_id"))
                    .cloned()
                    .unwrap_or(key);
                delta(out, &resolved, str_at(v, "delta"));
            }
            "response.output_item.done" => {
                let item = &v["item"];
                let resolved = self
                    .item_keys
                    .get(str_at(item, "id"))
                    .cloned()
                    .unwrap_or(key);
                if str_at(item, "type") == "reasoning" {
                    if let Some(summary) = item["summary"]
                        .as_array()
                        .filter(|summary| !summary.is_empty())
                    {
                        out.push(ProviderEvent::Replace {
                            key: resolved.clone(),
                            text: summary
                                .iter()
                                .map(|v| str_at(v, "text"))
                                .collect::<Vec<_>>()
                                .join("\n\n"),
                            signature: false,
                        });
                    }
                    signature(out, &resolved, &item.to_string());
                }
                if str_at(item, "type") == "message" {
                    if let Some(content) = item["content"].as_array() {
                        out.push(ProviderEvent::Replace {
                            key: resolved.clone(),
                            text: content
                                .iter()
                                .map(|v| {
                                    if str_at(v, "type") == "output_text" {
                                        str_at(v, "text")
                                    } else {
                                        str_at(v, "refusal")
                                    }
                                })
                                .collect::<String>(),
                            signature: false,
                        });
                    }
                    let mut text_signature = serde_json::json!({"v":1,"id":item["id"]});
                    if let Some(phase) = item.get("phase").filter(|v| !v.is_null()) {
                        text_signature["phase"] = phase.clone();
                    }
                    signature(out, &resolved, &text_signature.to_string());
                }
                if str_at(item, "type") == "function_call"
                    && !str_at(item, "arguments").trim().is_empty()
                {
                    out.push(ProviderEvent::Replace {
                        key: resolved.clone(),
                        text: str_at(item, "arguments").into(),
                        signature: false,
                    });
                }
                self.end(out, &resolved);
            }
            "response.completed" | "response.incomplete" => {
                let response = &v["response"];
                if self.family == ApiFamily::Responses
                    && let Some(tier) = response["service_tier"].as_str()
                {
                    out.push(ProviderEvent::ServiceTier(tier.into()));
                }
                let u = &response["usage"];
                let cached = n(&u["input_tokens_details"], "cached_tokens").unwrap_or(0);
                out.push(ProviderEvent::Usage(UsagePatch {
                    input: n(u, "input_tokens").map(|v| v.saturating_sub(cached)),
                    output: n(u, "output_tokens"),
                    cache_read: Some(cached),
                    cache_write: Some(0),
                    total: n(u, "total_tokens"),
                }));
                out.push(ProviderEvent::Stop(stop(str_at(response, "status"))));
            }
            "response.failed" | "response.cancelled" | "error" => {
                out.push(ProviderEvent::Fail(v.to_string()))
            }
            "response.in_progress"
            | "response.queued"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.output_text.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done"
            | "response.reasoning_summary_text.done"
            | "response.reasoning_text.done"
            | "response.function_call_arguments.done"
            | "response.refusal.done" => (),
            "" => {
                return Err(DomainError::InvalidInput(
                    "Responses stream event requires a type".into(),
                ));
            }
            // Responses streams may add informational events. Only the known
            // output-item and terminal branches can produce content, execute
            // tools or finish a response; additional events carry no authority.
            // Malformed JSON, unsupported output items and missing terminal
            // evidence still fail through their existing validation paths.
            _ => (),
        }
        Ok(())
    }
    fn google(&mut self, value: &Value, out: &mut Vec<ProviderEvent>) -> Result<(), DomainError> {
        let v = if self.family == ApiFamily::GeminiCli {
            value.get("response").unwrap_or(value)
        } else {
            value
        };
        if !str_at(v, "responseId").is_empty() {
            out.push(ProviderEvent::ResponseId(str_at(v, "responseId").into()));
        }
        if let Some(parts) = v["candidates"][0]["content"]["parts"].as_array() {
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    self.text(
                        out,
                        if part["thought"] == true {
                            BlockKind::Thinking
                        } else {
                            BlockKind::Text
                        },
                        text,
                    )?;
                    if let Some((_, key)) = &self.current {
                        replace_signature(out, key, str_at(part, "thoughtSignature"));
                    }
                } else if let Some(call) = part.get("functionCall") {
                    if let Some((_, key)) = self.current.take() {
                        self.end(out, &key);
                    }
                    let key = format!("function:{}", self.sequence);
                    self.sequence += 1;
                    // Deterministic stream-local identity when the provider omits one.
                    let id = call["id"]
                        .as_str()
                        .map(str::to_owned)
                        .map(Ok)
                        .unwrap_or_else(|| {
                            self.request_scope
                                .as_ref()
                                .map(|scope| {
                                    format!("{}_{}_{}", str_at(call, "name"), scope, self.sequence)
                                })
                                .ok_or_else(|| {
                                    DomainError::InvalidInput(
                                        "missing request scope for generated tool identity".into(),
                                    )
                                })
                        })?;
                    self.begin(
                        out,
                        &key,
                        BlockKind::Tool,
                        &id,
                        str_at(call, "name"),
                        Some(call.get("args").cloned().unwrap_or(serde_json::json!({}))),
                    )?;
                    replace_signature(out, &key, str_at(part, "thoughtSignature"));
                    self.end(out, &key);
                } else {
                    return Err(DomainError::Unavailable(
                        "unsupported Google response part".into(),
                    ));
                }
            }
        }
        if let Some(reason) = v["candidates"][0]["finishReason"].as_str() {
            out.push(ProviderEvent::Stop(stop(reason)));
        }
        if let Some(u) = v.get("usageMetadata") {
            let cached = n(u, "cachedContentTokenCount").unwrap_or(0);
            out.push(ProviderEvent::Usage(UsagePatch {
                input: n(u, "promptTokenCount").map(|v| v.saturating_sub(cached)),
                output: Some(
                    n(u, "candidatesTokenCount")
                        .unwrap_or(0)
                        .checked_add(n(u, "thoughtsTokenCount").unwrap_or(0))
                        .ok_or_else(|| DomainError::InvalidInput("Google usage overflow".into()))?,
                ),
                cache_read: Some(cached),
                cache_write: Some(0),
                total: n(u, "totalTokenCount"),
            }));
        }
        if let Some(reason) = v["promptFeedback"]["blockReason"].as_str() {
            out.push(ProviderEvent::Fail(format!(
                "Google prompt blocked: {reason}"
            )));
        }
        Ok(())
    }
    fn bedrock(&mut self, v: &Value, out: &mut Vec<ProviderEvent>) -> Result<(), DomainError> {
        if let Some(start) = v.get("messageStart") {
            if str_at(start, "role") != "assistant" {
                return Err(DomainError::InvalidInput("Bedrock assistant role".into()));
            }
        } else if let Some(start) = v.get("contentBlockStart") {
            let key = format!("block:{}", n(start, "contentBlockIndex").unwrap_or(0));
            let tool = &start["start"]["toolUse"];
            if !tool.is_null() {
                self.begin(
                    out,
                    &key,
                    BlockKind::Tool,
                    str_at(tool, "toolUseId"),
                    str_at(tool, "name"),
                    None,
                )?;
            }
        } else if let Some(event) = v.get("contentBlockDelta") {
            let key = format!("block:{}", n(event, "contentBlockIndex").unwrap_or(0));
            let d = &event["delta"];
            if let Some(text) = d["text"].as_str() {
                self.begin(out, &key, BlockKind::Text, "", "", None)?;
                delta(out, &key, text);
            } else if let Some(tool) = d.get("toolUse") {
                delta(out, &key, str_at(tool, "input"));
            } else if let Some(reasoning) = d.get("reasoningContent") {
                self.begin(out, &key, BlockKind::Thinking, "", "", None)?;
                delta(out, &key, str_at(reasoning, "text"));
                signature(out, &key, str_at(reasoning, "signature"));
            } else {
                return Err(DomainError::Unavailable("unsupported Bedrock delta".into()));
            }
        } else if let Some(event) = v.get("contentBlockStop") {
            self.end(
                out,
                &format!("block:{}", n(event, "contentBlockIndex").unwrap_or(0)),
            );
        } else if let Some(event) = v.get("messageStop") {
            out.push(ProviderEvent::Stop(stop(str_at(event, "stopReason"))));
        } else if let Some(event) = v.get("metadata") {
            let u = &event["usage"];
            out.push(ProviderEvent::Usage(UsagePatch {
                input: n(u, "inputTokens"),
                output: n(u, "outputTokens"),
                cache_read: n(u, "cacheReadInputTokens"),
                cache_write: n(u, "cacheWriteInputTokens"),
                total: n(u, "totalTokens"),
            }));
        } else if v
            .as_object()
            .is_some_and(|o| o.keys().any(|k| k.ends_with("Exception")))
        {
            out.push(ProviderEvent::Fail(v.to_string()));
        } else {
            return Err(DomainError::Unavailable("unknown Bedrock event".into()));
        }
        Ok(())
    }
}
fn anthropic_usage(u: &Value) -> UsagePatch {
    UsagePatch {
        input: n(u, "input_tokens"),
        output: n(u, "output_tokens"),
        cache_read: n(u, "cache_read_input_tokens"),
        cache_write: n(u, "cache_creation_input_tokens"),
        total: None,
    }
}
