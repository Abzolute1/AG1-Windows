//! Capability observations are model/endpoint scoped. Read-only introspection is
//! separate from the explicitly admitted image canary; no discovery call generates.
use super::http::*;
use ag1_core::{
    DomainError, SecretBytes,
    inference::GenerationPermit,
    model::{VisionObservation, VisionVerdict},
};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::Instant;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    LlamaProps,
    LmStudio,
    Ollama,
    Canary,
}
impl ProbeKind {
    fn source(self) -> &'static str {
        match self {
            Self::LlamaProps => "llamacpp",
            Self::LmStudio => "lmstudio",
            Self::Ollama => "ollama",
            Self::Canary => "canary",
        }
    }
}
pub const CANARY_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEElEQVR4nGNgaGjAjoaWBADiMEABWMSWggAAAABJRU5ErkJggg==";
fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
pub fn introspection_kind(provider: &str, base: &str) -> Option<ProbeKind> {
    let p = provider.to_lowercase();
    if ["llamacpp", "llama.cpp"].contains(&p.as_str()) || p.contains("llama-cpp") {
        return Some(ProbeKind::LlamaProps);
    }
    if p.contains("lmstudio") || p.contains("lm-studio") {
        return Some(ProbeKind::LmStudio);
    }
    if p.contains("ollama") {
        return Some(ProbeKind::Ollama);
    }
    match reqwest::Url::parse(base).ok().and_then(|u| u.port()) {
        Some(1234) => Some(ProbeKind::LmStudio),
        Some(11434) => Some(ProbeKind::Ollama),
        _ => None,
    }
}
pub fn probe_request(
    kind: ProbeKind,
    base: &str,
    model: &str,
    key: Option<&SecretBytes>,
) -> Result<HttpRequest, DomainError> {
    endpoint_origin(base)?;
    if model.is_empty() || model.len() > 1024 {
        return Err(DomainError::InvalidInput("probe model identity".into()));
    }
    let base = base.trim_end_matches('/');
    let origin = if base.to_lowercase().ends_with("/v1") {
        &base[..base.len() - 3]
    } else {
        base
    };
    let mut headers = HostHeaders::default();
    let (url, method, body) = match kind {
        ProbeKind::LlamaProps => (format!("{origin}/props"), HttpMethod::Get, vec![]),
        ProbeKind::LmStudio => (format!("{origin}/api/v0/models"), HttpMethod::Get, vec![]),
        ProbeKind::Ollama => {
            headers.insert_public("content-type", "application/json")?;
            (
                format!("{origin}/api/show"),
                HttpMethod::Post,
                serde_json::to_vec(&json!({"model":model}))
                    .map_err(|_| DomainError::InvalidInput("probe body".into()))?,
            )
        }
        ProbeKind::Canary => {
            headers.insert_public("content-type", "application/json")?;
            if let Some(key) = key {
                let key = std::str::from_utf8(key.expose())
                    .map_err(|_| DomainError::InvalidInput("probe credential".into()))?;
                headers.insert_secret(
                    "authorization",
                    &SecretBytes::new(format!("Bearer {key}").into_bytes()),
                )?;
            }
            let body = json!({"model":model,"messages":[{"role":"user","content":[{"type":"text","text":"This is a capability probe. The attached image is filled with a single color. Reply with exactly one word: the color."},{"type":"image_url","image_url":{"url":format!("data:image/png;base64,{CANARY_PNG}")}}]}],"max_tokens":512});
            (
                format!("{base}/chat/completions"),
                HttpMethod::Post,
                serde_json::to_vec(&body)
                    .map_err(|_| DomainError::InvalidInput("probe body".into()))?,
            )
        }
    };
    Ok(HttpRequest {
        url,
        method,
        headers,
        body,
        max_response_bytes: 2 * 1024 * 1024,
    })
}
pub fn interpret_probe(
    kind: ProbeKind,
    model: &str,
    status: u16,
    bytes: &[u8],
) -> VisionObservation {
    let mut result = VisionObservation {
        verdict: VisionVerdict::Unknown,
        source: Some(kind.source().into()),
        reason: Some("capability probe was inconclusive".into()),
    };
    if bytes.len() > 2 * 1024 * 1024 {
        return result;
    }
    if !(200..300).contains(&status) {
        if kind == ProbeKind::Canary {
            let body = String::from_utf8_lossy(bytes).to_lowercase();
            let wrong_model = body.contains("does not exist")
                || body
                    .find("model")
                    .and_then(|i| body[i..].find("not").map(|j| i + j))
                    .is_some_and(|i| body[i..].contains("found"));
            if status != 404 && !wrong_model {
                result.verdict = VisionVerdict::No;
                result.reason = Some(format!("endpoint rejected image input with HTTP {status}"));
            }
        }
        return result;
    }
    let Ok(body) = serde_json::from_slice::<Value>(bytes) else {
        return result;
    };
    let verdict = match kind {
        ProbeKind::LlamaProps => body["modalities"]["vision"].as_bool().map(|v| {
            if v {
                VisionVerdict::Yes
            } else {
                VisionVerdict::No
            }
        }),
        ProbeKind::Ollama => body["capabilities"].as_array().map(|a| {
            if a.iter().any(|v| v == "vision") {
                VisionVerdict::Yes
            } else {
                VisionVerdict::No
            }
        }),
        ProbeKind::LmStudio => {
            let needle = model.to_lowercase();
            let base = needle.rsplit('/').next().unwrap_or("");
            let mut exact = None;
            let mut suffix = None;
            let mut basename = None;
            if let Some(entries) = body["data"].as_array() {
                for entry in entries {
                    let id = str_at(entry, "id").to_lowercase();
                    if id.is_empty() {
                        continue;
                    }
                    if id == needle {
                        exact = Some(entry);
                        break;
                    }
                    if suffix.is_none()
                        && (id.ends_with(&format!("/{needle}"))
                            || needle.ends_with(&format!("/{id}")))
                    {
                        suffix = Some(entry);
                    }
                    if basename.is_none() && !base.is_empty() && id.rsplit('/').next() == Some(base)
                    {
                        basename = Some(entry);
                    }
                }
            }
            exact.or(suffix).or(basename).map(|m| {
                if m["type"] == "vlm" {
                    VisionVerdict::Yes
                } else {
                    VisionVerdict::No
                }
            })
        }
        ProbeKind::Canary => {
            let content = &body["choices"][0]["message"]["content"];
            let text = content.as_str().map(str::to_owned).unwrap_or_else(|| {
                content
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|b| b["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("")
                    })
                    .unwrap_or_default()
            });
            if text.to_lowercase().contains("teal") {
                Some(VisionVerdict::Yes)
            } else {
                None
            }
        }
    };
    if let Some(verdict) = verdict {
        result.verdict = verdict;
        result.reason = None;
    }
    result
}
async fn collect(
    http: &ProviderHttp,
    request: HttpRequest,
    timeout: Duration,
    cancel: &mut CancelToken,
) -> Result<(u16, Vec<u8>), DomainError> {
    let deadline = Instant::now() + timeout;
    let mut response = http.send(request, deadline, cancel).await?;
    let status = response.status();
    let mut body = vec![];
    while let Some(bytes) = response.chunk(deadline, cancel).await? {
        body.extend(bytes);
    }
    Ok((status, body))
}
pub async fn probe_introspection(
    http: &ProviderHttp,
    kind: ProbeKind,
    base: &str,
    model: &str,
    cancel: &mut CancelToken,
) -> Result<VisionObservation, DomainError> {
    if kind == ProbeKind::Canary {
        return Err(DomainError::PermissionDenied);
    }
    let request = probe_request(kind, base, model, None)?;
    let (status, body) = collect(http, request, Duration::from_secs(3), cancel).await?;
    Ok(interpret_probe(kind, model, status, &body))
}
/// Called only by the composition host after explicit endpoint designation and
/// admission to the shared scheduler. Interrupted I/O quarantines the permit.
pub async fn probe_canary(
    http: &ProviderHttp,
    base: &str,
    model: &str,
    key: Option<&SecretBytes>,
    permit: GenerationPermit,
    cancel: &mut CancelToken,
) -> Result<VisionObservation, DomainError> {
    let request = match probe_request(ProbeKind::Canary, base, model, key).and_then(|r| {
        http.validate_request(&r)?;
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        Ok(r)
    }) {
        Ok(r) => r,
        Err(error) => {
            permit.terminal()?;
            return Err(error);
        }
    };
    let (status, body) = collect(http, request, Duration::from_secs(30), cancel).await?;
    permit.terminal()?;
    Ok(interpret_probe(ProbeKind::Canary, model, status, &body))
}
