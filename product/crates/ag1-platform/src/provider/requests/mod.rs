//! Pure bounded request conversion from the pinned AG1 message format. No I/O,
//! credential lookup, model startup or scheduler acquisition occurs here.
mod chat;
mod options;
mod session_options;
pub use session_options::session_options;
mod structured;
use ag1_core::{DomainError, provider::ApiFamily};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub struct RequestContext<'a> {
    pub model: &'a Value,
    pub context: &'a Value,
    /// Provider options, resolved by the host. Credentials must not be included.
    pub options: &'a Value,
    pub anthropic_oauth: bool,
    pub google_project: Option<&'a str>,
    pub request_id: &'a str,
}
pub fn build_payload(input: &RequestContext<'_>) -> Result<Value, DomainError> {
    for value in [input.model, input.context, input.options] {
        let size = serde_json::to_vec(value)
            .map_err(|_| invalid("request JSON"))?
            .len();
        if size > 32 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("provider input".into()));
        }
        if !value.is_object() {
            return Err(invalid("request object"));
        }
    }
    if string(input.model, "id").is_empty() {
        return Err(invalid("model identity"));
    }
    let family: ApiFamily = serde_json::from_value(input.model["api"].clone())
        .map_err(|_| invalid("provider family"))?;
    let messages = transform(input, family)?;
    let mut result = match family {
        ApiFamily::Completions | ApiFamily::Mistral => chat::build(input, family, &messages),
        _ => structured::build(input, family, &messages),
    }?;
    options::finish(&mut result, input, family)?;
    if serde_json::to_vec(&result)
        .map_err(|_| invalid("request JSON"))?
        .len()
        > 32 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("provider payload".into()));
    }
    Ok(result)
}
fn invalid(text: &str) -> DomainError {
    DomainError::InvalidInput(text.into())
}
fn string<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
fn flag(v: &Value, k: &str) -> bool {
    v[k].as_bool().unwrap_or(false)
}
fn blocks(v: &Value) -> Vec<Value> {
    if let Some(s) = v.as_str() {
        vec![json!({"type":"text","text":s})]
    } else {
        v.as_array().cloned().unwrap_or_default()
    }
}
fn text(v: &Value) -> String {
    blocks(v)
        .iter()
        .filter(|v| v["type"] == "text")
        .map(|v| string(v, "text"))
        .collect::<Vec<_>>()
        .join("\n")
}
fn same(m: &Value, model: &Value) -> bool {
    m["provider"] == model["provider"] && m["model"] == model["id"] && m["api"] == model["api"]
}
fn clean_id(id: &str, max: usize) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "_-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .take(max)
        .collect()
}
fn base36(mut n: u32) -> String {
    let mut out = vec![];
    loop {
        out.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize] as char);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.iter().rev().collect()
}
/// JavaScript's UTF-16 shortHash used by the pinned adapters for foreign tool IDs.
fn short_hash(value: &str) -> String {
    let (mut h1, mut h2) = (0xdeadbeefu32, 0x41c6ce57u32);
    for ch in value.encode_utf16() {
        h1 = (h1 ^ u32::from(ch)).wrapping_mul(2654435761);
        h2 = (h2 ^ u32::from(ch)).wrapping_mul(1597334677);
    }
    h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507) ^ (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507) ^ (h1 ^ (h1 >> 13)).wrapping_mul(3266489909);
    format!("{}{}", base36(h2), base36(h1))
}
fn normalize_id(id: &str, family: ApiFamily, model: &Value) -> String {
    match family {
        ApiFamily::Mistral => {
            let clean: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
            if clean.len() == 9 {
                clean
            } else {
                short_hash(id).chars().take(9).collect()
            }
        }
        ApiFamily::Completions => {
            if id.contains('|') {
                clean_id(id.split('|').next().unwrap_or(id), 40)
            } else if model["provider"] == "openai" {
                clean_id(id, 40)
            } else {
                id.into()
            }
        }
        ApiFamily::Responses | ApiFamily::AzureResponses | ApiFamily::CodexResponses => {
            let allowed = ["openai", "openai-codex", "opencode"]
                .contains(&string(model, "provider"))
                || (family == ApiFamily::AzureResponses
                    && model["provider"] == "azure-openai-responses");
            if !allowed {
                return clean_id(id, 64).trim_end_matches('_').into();
            }
            if let Some((call, item)) = id.split_once('|') {
                format!("{}|fc_{}", clean_id(call, 64), short_hash(item))
            } else {
                clean_id(id, 64)
            }
        }
        ApiFamily::Google | ApiFamily::GeminiCli | ApiFamily::Vertex if !google_ids(model) => {
            id.into()
        }
        _ => clean_id(id, 64),
    }
}
fn google_ids(model: &Value) -> bool {
    let id = string(model, "id");
    id.starts_with("claude-") || id.starts_with("gpt-oss-")
}
fn transform(input: &RequestContext<'_>, family: ApiFamily) -> Result<Vec<Value>, DomainError> {
    let source = input.context["messages"]
        .as_array()
        .ok_or_else(|| invalid("request messages"))?;
    if source.len() > 100_000 {
        return Err(DomainError::LimitExceeded("request messages".into()));
    }
    let mut transformed = vec![];
    let mut ids = BTreeMap::new();
    let mut used = BTreeSet::new();
    for original in source {
        let mut m = original.clone();
        let role = string(&m, "role").to_owned();
        if !["user", "assistant", "toolResult"].contains(&role.as_str()) {
            return Err(invalid("message role"));
        }
        let content = blocks(&m["content"]);
        if !m["content"].is_array() && !m["content"].is_string() {
            return Err(invalid("message content"));
        }
        if content.len() > 4096 {
            return Err(DomainError::LimitExceeded("message blocks".into()));
        }
        for b in &content {
            if !["text", "thinking", "toolCall", "image"].contains(&string(b, "type")) {
                return Err(invalid("content block type"));
            }
            if b["type"] == "image"
                && !input.model["input"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v == "image"))
            {
                return Err(DomainError::Unavailable(
                    "selected model does not support images".into(),
                ));
            }
            if b["type"] == "toolCall"
                && (!b["arguments"].is_object()
                    || string(b, "id").is_empty()
                    || string(b, "name").is_empty())
            {
                return Err(invalid("tool call"));
            }
        }
        if role == "assistant" {
            let same_model = same(&m, input.model);
            let mut out = vec![];
            for mut b in content {
                if !same_model {
                    match string(&b, "type") {
                        "thinking" => {
                            if flag(&b, "redacted") || string(&b, "thinking").trim().is_empty() {
                                continue;
                            }
                            b = json!({"type":"text","text":b["thinking"]});
                        }
                        "text" => {
                            b.as_object_mut().unwrap().remove("textSignature");
                        }
                        "toolCall" => {
                            b.as_object_mut().unwrap().remove("thoughtSignature");
                            let old = string(&b, "id").to_owned();
                            let mut id = normalize_id(&old, family, input.model);
                            if family == ApiFamily::Mistral {
                                let mut attempt = 0;
                                while used.contains(&id) {
                                    attempt += 1;
                                    id = short_hash(&format!("{old}:{attempt}"))
                                        .chars()
                                        .take(9)
                                        .collect();
                                }
                            }
                            used.insert(id.clone());
                            ids.insert(old, id.clone());
                            b["id"] = json!(id);
                        }
                        _ => {}
                    }
                }
                out.push(b);
            }
            m["content"] = json!(out);
        } else if role == "toolResult"
            && let Some(id) = ids.get(string(&m, "toolCallId"))
        {
            m["toolCallId"] = json!(id);
        }
        transformed.push(m);
    }
    let mut out = vec![];
    let mut pending: Vec<Value> = vec![];
    let mut seen = BTreeSet::new();
    fn flush(out: &mut Vec<Value>, pending: &mut Vec<Value>, seen: &mut BTreeSet<String>) {
        for call in pending.drain(..) {
            if !seen.contains(string(&call, "id")) {
                out.push(json!({"role":"toolResult","toolCallId":call["id"],"toolName":call["name"],"content":[{"type":"text","text":"No result provided"}],"isError":true,"timestamp":0}));
            }
        }
        seen.clear();
    }
    for m in transformed {
        match string(&m, "role") {
            "assistant" => {
                flush(&mut out, &mut pending, &mut seen);
                if ["error", "aborted"].contains(&string(&m, "stopReason")) {
                    continue;
                }
                pending = blocks(&m["content"])
                    .into_iter()
                    .filter(|v| v["type"] == "toolCall")
                    .collect();
            }
            "user" => flush(&mut out, &mut pending, &mut seen),
            "toolResult" => {
                if pending.is_empty() {
                    continue;
                }
                seen.insert(string(&m, "toolCallId").into());
            }
            _ => unreachable!(),
        }
        out.push(m);
    }
    flush(&mut out, &mut pending, &mut seen);
    Ok(out)
}
fn tools(input: &RequestContext<'_>) -> Result<Option<Vec<Value>>, DomainError> {
    let Some(v) = input.context.get("tools") else {
        return Ok(None);
    };
    let array = v.as_array().ok_or_else(|| invalid("tool definitions"))?;
    if array.len() > 1024 {
        return Err(DomainError::LimitExceeded("tool definitions".into()));
    }
    let mut names = BTreeSet::new();
    for tool in array {
        let name = string(tool, "name");
        if name.is_empty()
            || name.len() > 128
            || !names.insert(name)
            || !tool["parameters"].is_object()
        {
            return Err(invalid("tool definition"));
        }
    }
    Ok(Some(array.clone()))
}
fn image_url(b: &Value) -> String {
    format!(
        "data:{};base64,{}",
        string(b, "mimeType"),
        string(b, "data")
    )
}
fn option(out: &mut Value, options: &Value, from: &str, to: &str) {
    if let Some(value) = options.get(from) {
        out[to] = value.clone();
    }
}
