//! Host-only authentication and URL construction. Model request bodies are pure;
//! resolved credentials and signed HTTP requests never cross the renderer protocol.
use super::{
    auth::codex_account_id,
    http::*,
    requests::*,
    signing::{AwsCredentials, sign_bedrock},
};
use ag1_core::{DomainError, SecretBytes, provider::ApiFamily};
use serde_json::Value;

pub enum HttpCredential<'a> {
    ApiKey(&'a SecretBytes),
    Bearer(&'a SecretBytes),
    /// The pinned Gemini CLI vault stores a JSON {token,projectId} API-key envelope.
    GeminiCli(&'a SecretBytes),
    /// Host-resolved credentials and clock; signing follows final serialization.
    Aws {
        credentials: &'a AwsCredentials,
        region: &'a str,
        date: &'a str,
    },
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
fn secret_text(secret: &SecretBytes) -> Result<&str, DomainError> {
    std::str::from_utf8(secret.expose()).map_err(|_| invalid("credential UTF-8"))
}
fn bearer(headers: &mut HostHeaders, secret: &SecretBytes) -> Result<(), DomainError> {
    headers.insert_secret(
        "authorization",
        &SecretBytes::new(format!("Bearer {}", secret_text(secret)?).into_bytes()),
    )
}
fn field<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn segment(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
pub fn generation_http_request(
    input: &RequestContext<'_>,
    credential: HttpCredential<'_>,
    max_response_bytes: usize,
) -> Result<HttpRequest, DomainError> {
    generation_http_request_with_headers(input, credential, None, max_response_bytes)
}
/// Explicit host headers preserve configured provider integrations. They are
/// merged before AWS signing and never accepted from a renderer request body.
pub fn generation_http_request_with_headers(
    input: &RequestContext<'_>,
    credential: HttpCredential<'_>,
    extra: Option<&HostHeaders>,
    max_response_bytes: usize,
) -> Result<HttpRequest, DomainError> {
    let family: ApiFamily = serde_json::from_value(input.model["api"].clone())
        .map_err(|_| invalid("provider family"))?;
    let base = field(input.model, "baseUrl").trim_end_matches('/');
    endpoint_origin(base)?;
    let parsed = reqwest::Url::parse(base).map_err(|_| invalid("provider URL"))?;
    if parsed.query().is_some() {
        return Err(invalid("provider base URL query"));
    }
    let model = field(input.model, "id");
    let opts = input.options;
    let mut headers = HostHeaders::default();
    headers.insert_public("content-type", "application/json")?;
    headers.insert_public("user-agent", "ag1-rust/0.1.0")?;
    let mut project = None;
    let oauth = matches!(&credential,HttpCredential::ApiKey(key)|HttpCredential::Bearer(key) if key.expose().starts_with(b"sk-ant-oat"));
    let url = match family {
        ApiFamily::Anthropic => {
            headers.insert_public("accept", "application/json")?;
            headers.insert_public("anthropic-version", "2023-06-01")?;
            let key = match &credential {
                HttpCredential::ApiKey(k) | HttpCredential::Bearer(k) => k,
                _ => return Err(invalid("Anthropic credential")),
            };
            let adaptive = ["opus-4-6", "opus-4.6", "sonnet-4-6", "sonnet-4.6"]
                .iter()
                .any(|s| model.contains(s));
            let interleaved = opts["interleavedThinking"].as_bool().unwrap_or(true) && !adaptive;
            let mut beta = vec![];
            if oauth {
                bearer(&mut headers, key)?;
                headers.insert_public("user-agent", "claude-cli/2.1.75")?;
                headers.insert_public("x-app", "cli")?;
                beta.extend(["claude-code-20250219", "oauth-2025-04-20"]);
            } else if matches!(credential, HttpCredential::Bearer(_))
                || field(input.model, "provider") == "github-copilot"
            {
                bearer(&mut headers, key)?;
            } else {
                headers.insert_secret("x-api-key", key)?;
            }
            if field(input.model, "provider") != "github-copilot" {
                beta.push("fine-grained-tool-streaming-2025-05-14");
            }
            if interleaved {
                beta.push("interleaved-thinking-2025-05-14");
            }
            if !beta.is_empty() {
                headers.insert_public("anthropic-beta", &beta.join(","))?;
            }
            format!("{base}/v1/messages")
        }
        ApiFamily::Completions
        | ApiFamily::Mistral
        | ApiFamily::Responses
        | ApiFamily::CodexResponses => {
            let key = match &credential {
                HttpCredential::ApiKey(k) | HttpCredential::Bearer(k) => k,
                _ => return Err(invalid("OpenAI-compatible credential")),
            };
            bearer(&mut headers, key)?;
            headers.insert_public(
                "accept",
                if matches!(family, ApiFamily::Mistral | ApiFamily::CodexResponses) {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            )?;
            if family == ApiFamily::CodexResponses {
                headers.insert_secret(
                    "chatgpt-account-id",
                    &SecretBytes::new(codex_account_id(key)?.into_bytes()),
                )?;
                headers.insert_public("originator", "ag1")?;
                headers.insert_public("openai-beta", "responses=experimental")?;
                if let Some(session) = opts["sessionId"].as_str() {
                    headers.insert_public("conversation_id", session)?;
                    headers.insert_public("session_id", session)?;
                }
                if base.ends_with("/codex/responses") {
                    base.into()
                } else if base.ends_with("/codex") {
                    format!("{base}/responses")
                } else {
                    format!("{base}/codex/responses")
                }
            } else {
                format!(
                    "{base}/{}",
                    match family {
                        ApiFamily::Responses => "responses",
                        ApiFamily::Mistral => "v1/chat/completions",
                        _ => "chat/completions",
                    }
                )
            }
        }
        ApiFamily::AzureResponses => {
            match &credential {
                HttpCredential::ApiKey(key) => headers.insert_secret("api-key", key)?,
                HttpCredential::Bearer(key) => bearer(&mut headers, key)?,
                _ => return Err(invalid("Azure credential")),
            }
            headers.insert_public("accept", "application/json")?;
            let azure = opts["azureBaseUrl"]
                .as_str()
                .unwrap_or(base)
                .trim_end_matches('/');
            endpoint_origin(azure)?;
            let version = opts["azureApiVersion"]
                .as_str()
                .ok_or_else(|| invalid("Azure API version"))?;
            let mut url = reqwest::Url::parse(&format!("{azure}/responses"))
                .map_err(|_| invalid("Azure URL"))?;
            url.query_pairs_mut().append_pair("api-version", version);
            url.to_string()
        }
        ApiFamily::Google | ApiFamily::Vertex => {
            match &credential {
                HttpCredential::ApiKey(key) => headers.insert_secret("x-goog-api-key", key)?,
                HttpCredential::Bearer(key) => bearer(&mut headers, key)?,
                _ => return Err(invalid("Google credential")),
            }
            if family == ApiFamily::Google {
                format!(
                    "{base}/models/{}:streamGenerateContent?alt=sse",
                    segment(model)
                )
            } else if matches!(credential, HttpCredential::ApiKey(_)) {
                format!(
                    "{base}/v1/publishers/google/models/{}:streamGenerateContent?alt=sse",
                    segment(model)
                )
            } else {
                let project = opts["project"]
                    .as_str()
                    .ok_or_else(|| invalid("Vertex project"))?;
                let location = opts["location"]
                    .as_str()
                    .ok_or_else(|| invalid("Vertex location"))?;
                format!(
                    "{base}/v1/projects/{}/locations/{}/publishers/google/models/{}:streamGenerateContent?alt=sse",
                    segment(project),
                    segment(location),
                    segment(model)
                )
            }
        }
        ApiFamily::GeminiCli => {
            let key = match &credential {
                HttpCredential::GeminiCli(key) => key,
                _ => return Err(invalid("Gemini CLI credential")),
            };
            let envelope: Value = serde_json::from_slice(key.expose())
                .map_err(|_| invalid("Gemini credential envelope"))?;
            let token = field(&envelope, "token");
            let p = field(&envelope, "projectId");
            if token.is_empty() || p.is_empty() {
                return Err(invalid("Gemini token/project"));
            }
            if input.google_project.is_some_and(|expected| expected != p) {
                return Err(DomainError::Conflict);
            }
            project = Some(p.to_owned());
            bearer(&mut headers, &SecretBytes::new(token.as_bytes().to_vec()))?;
            headers.insert_public("accept", "text/event-stream")?;
            headers.insert_public("user-agent", "google-cloud-sdk vscode_cloudshelleditor/0.1")?;
            headers.insert_public("x-goog-api-client", "gl-node/22.17.0")?;
            headers.insert_public("client-metadata",r#"{"ideType":"IDE_UNSPECIFIED","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}"#)?;
            format!("{base}/v1internal:streamGenerateContent?alt=sse")
        }
        ApiFamily::Bedrock => {
            if !matches!(credential, HttpCredential::Aws { .. }) {
                return Err(invalid("Bedrock signing credential"));
            }
            headers.insert_public("accept", "application/vnd.amazon.eventstream")?;
            format!("{base}/model/{}/converse-stream", segment(model))
        }
    };
    let context = RequestContext {
        model: input.model,
        context: input.context,
        options: opts,
        anthropic_oauth: oauth,
        google_project: project.as_deref().or(input.google_project),
        request_id: input.request_id,
    };
    let body =
        serde_json::to_vec(&build_payload(&context)?).map_err(|_| invalid("provider JSON"))?;
    endpoint_origin(&url)?;
    if let Some(extra) = extra {
        for (name, value) in &extra.headers {
            headers.insert_secret(name.as_str(), &SecretBytes::new(value.as_bytes().to_vec()))?;
        }
    }
    let mut request = HttpRequest {
        url,
        method: HttpMethod::Post,
        headers,
        body,
        max_response_bytes,
    };
    if let HttpCredential::Aws {
        credentials,
        region,
        date,
    } = credential
    {
        sign_bedrock(&mut request, credentials, region, date)?;
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn nine_http_auth_families_match_pinned_request_headers_and_urls() {
        let oracle: Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/providers/headers.json"
        ))
        .unwrap();
        for group in oracle["groups"].as_array().unwrap() {
            let api = field(group, "api");
            if api == "bedrock-converse-stream" {
                continue;
            }
            let expected = &group["transport"][0];
            let mut key = "fixture-key".to_owned();
            if api == "openai-codex-responses" {
                key = field(&expected["headers"], "authorization")
                    .strip_prefix("Bearer ")
                    .unwrap()
                    .into();
            }
            if api == "google-gemini-cli" {
                key = r#"{"token":"fixture-token","projectId":"fixture-project"}"#.into();
            }
            let secret = SecretBytes::new(key.into_bytes());
            let credential = if api == "google-gemini-cli" {
                HttpCredential::GeminiCli(&secret)
            } else {
                HttpCredential::ApiKey(&secret)
            };
            let model = json!({"id":"fixture","api":api,"provider":if api=="openai-codex-responses"{"openai-codex"}else{api},"baseUrl":"https://fixture.invalid/v1","reasoning":true,"input":["text","image"],"maxTokens":4096});
            let context = json!({"systemPrompt":"Fixture system","messages":[{"role":"user","content":"Return a tool call or answer."}],"tools":[{"name":"read","description":"Fixture read","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]});
            let options = json!({"maxTokens":256,"azureBaseUrl":"https://fixture.invalid","azureDeploymentName":"fixture","azureApiVersion":"2025-04-01-preview"});
            let request = generation_http_request(
                &RequestContext {
                    model: &model,
                    context: &context,
                    options: &options,
                    anthropic_oauth: false,
                    google_project: Some("fixture-project"),
                    request_id: "<fixture-request-id>",
                },
                credential,
                1024 * 1024,
            )
            .unwrap();
            assert_eq!(request.url, expected["url"], "{api}");
            for (name, value) in expected["headers"].as_object().unwrap() {
                // SDK telemetry and browser permission flags are not protocol/auth headers.
                if name.starts_with("x-stainless-")
                    || [
                        "user-agent",
                        "x-goog-api-client",
                        "anthropic-dangerous-direct-browser-access",
                        "cookie",
                    ]
                    .contains(&name.as_str())
                {
                    continue;
                }
                assert_eq!(
                    request
                        .headers
                        .headers
                        .get(name)
                        .and_then(|v| v.to_str().ok()),
                    value.as_str(),
                    "{api}/{name}"
                );
            }
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body, expected["body"], "{api}");
        }
    }
}
