//! Credential precedence and refresh stay in the host. None of these values are
//! frontend DTOs. Backend locking/CAS must span every host that writes this vault.
use super::http::*;
use ag1_core::{Clock, DomainError, PortFuture, SecretBytes};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    Runtime,
    SecretFile,
    Environment,
    Stored,
    Fallback,
}
pub struct CredentialRecord {
    pub revision: u64,
    pub raw: SecretBytes,
}
/// Native vault tombstones retain revision history so logout cannot be erased by
/// an older in-flight login/refresh. They contain no credential material.
pub const REVOKED_CREDENTIAL_TYPE: &str = "ag1:revoked";
pub trait CredentialLease: Send {}
impl<T: Send> CredentialLease for T {}
pub trait CredentialBackend: Send + Sync {
    /// All credential coordinators sharing this vault must acquire the same lock.
    /// The lease spans load, remote refresh, and durable compare_exchange.
    fn acquire<'a>(&'a self, provider: &'a str) -> PortFuture<'a, Box<dyn CredentialLease + 'a>>;
    /// Runtime/file/environment/fallback values have already passed host config policy.
    fn resolve<'a>(
        &'a self,
        provider: &'a str,
        source: CredentialSource,
    ) -> PortFuture<'a, Option<SecretBytes>>;
    fn load<'a>(&'a self, provider: &'a str) -> PortFuture<'a, Option<CredentialRecord>>;
    /// Create only an absent record while holding the shared vault lease. A
    /// concurrent creator must conflict rather than replace an existing secret.
    fn create<'a>(&'a self, _provider: &'a str, _raw: SecretBytes) -> PortFuture<'a, ()> {
        Box::pin(async {
            Err(DomainError::Unavailable(
                "credential creation is unavailable".into(),
            ))
        })
    }
    fn compare_exchange<'a>(
        &'a self,
        provider: &'a str,
        revision: u64,
        raw: SecretBytes,
    ) -> PortFuture<'a, ()>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OAuthService {
    Anthropic,
    Codex,
    GitHubCopilot,
    GeminiCli,
    Antigravity,
}
pub fn codex_account_id(token: &SecretBytes) -> Result<String, DomainError> {
    let error = || DomainError::InvalidInput("Codex token account identity".into());
    if token.expose().len() > 64 * 1024 {
        return Err(error());
    }
    let text = std::str::from_utf8(token.expose()).map_err(|_| error())?;
    let parts: Vec<_> = text.split('.').collect();
    if parts.len() != 3 {
        return Err(error());
    }
    let payload = parts[1]
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_");
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| error())?;
    let json: Value = serde_json::from_slice(&bytes).map_err(|_| error())?;
    json["https://api.openai.com/auth"]["chatgpt_account_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(error)
}
pub trait OAuthRefresh: Send + Sync {
    fn refresh<'a>(
        &'a self,
        service: OAuthService,
        raw: &'a SecretBytes,
    ) -> PortFuture<'a, SecretBytes>;
}
pub struct ResolvedCredential {
    pub value: SecretBytes,
    pub source: CredentialSource,
    pub oauth: Option<OAuthService>,
}
pub struct AuthCoordinator<B: CredentialBackend, R: OAuthRefresh> {
    backend: B,
    refresh: R,
    clock: Arc<dyn Clock>,
    locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
}
fn parse(raw: &SecretBytes) -> Result<Value, DomainError> {
    if raw.expose().len() > 1024 * 1024 {
        return Err(DomainError::LimitExceeded("credential record".into()));
    }
    let value: Value = serde_json::from_slice(raw.expose())
        .map_err(|_| DomainError::InvalidInput("corrupt credential record".into()))?;
    if !value.is_object() {
        return Err(DomainError::InvalidInput("credential object".into()));
    }
    Ok(value)
}
fn secret_field(value: &Value, key: &str) -> Result<SecretBytes, DomainError> {
    let text = value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| DomainError::Unavailable(format!("credential missing {key}")))?;
    Ok(SecretBytes::new(text.as_bytes().to_vec()))
}
fn key_for(service: OAuthService, value: &Value) -> Result<SecretBytes, DomainError> {
    if matches!(service, OAuthService::GeminiCli | OAuthService::Antigravity) {
        let project = value["projectId"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| DomainError::Unavailable("OAuth project is missing".into()))?;
        let access = secret_field(value, "access")?;
        Ok(SecretBytes::new(serde_json::to_vec(&serde_json::json!({"token":std::str::from_utf8(access.expose()).map_err(|_|DomainError::InvalidInput("OAuth token UTF-8".into()))?,"projectId":project})).map_err(|_|DomainError::InvalidInput("OAuth token envelope".into()))?))
    } else {
        secret_field(value, "access")
    }
}
impl<B: CredentialBackend, R: OAuthRefresh> AuthCoordinator<B, R> {
    pub fn new(backend: B, refresh: R, clock: Arc<dyn Clock>) -> Self {
        Self {
            backend,
            refresh,
            clock,
            locks: Mutex::new(BTreeMap::new()),
        }
    }
    pub async fn resolve(
        &self,
        provider: &str,
        service: Option<OAuthService>,
        include_fallback: bool,
    ) -> Result<Option<ResolvedCredential>, DomainError> {
        if provider.is_empty() || provider.len() > 128 {
            return Err(DomainError::InvalidInput("credential provider".into()));
        }
        for source in [
            CredentialSource::Runtime,
            CredentialSource::SecretFile,
            CredentialSource::Environment,
        ] {
            if let Some(value) = self
                .backend
                .resolve(provider, source)
                .await?
                .filter(|v| !v.expose().is_empty())
            {
                return Ok(Some(ResolvedCredential {
                    value,
                    source,
                    oauth: None,
                }));
            }
        }
        if provider == "nvidia-nim" {
            for source in [
                CredentialSource::Runtime,
                CredentialSource::SecretFile,
                CredentialSource::Environment,
                CredentialSource::Stored,
            ] {
                if let Some(value) = self
                    .backend
                    .resolve("nvidia", source)
                    .await?
                    .filter(|v| !v.expose().is_empty())
                {
                    return Ok(Some(ResolvedCredential {
                        value,
                        source,
                        oauth: None,
                    }));
                }
            }
        }
        // One refresh per provider within this host. The backend's writer/CAS protects
        // against another authorized host changing credentials while refresh is in flight.
        let lock = {
            let mut locks = self.locks.lock().await;
            if !locks.contains_key(provider) && locks.len() >= 128 {
                return Err(DomainError::QueueFull);
            }
            locks
                .entry(provider.into())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        let _vault_guard = self.backend.acquire(provider).await?;
        if let Some(record) = self.backend.load(provider).await? {
            let mut value = parse(&record.raw)?;
            match value["type"].as_str() {
                Some("api_key") => {
                    let resolved = self
                        .backend
                        .resolve(provider, CredentialSource::Stored)
                        .await?
                        .ok_or_else(|| {
                            DomainError::Unavailable("stored API key resolution failed".into())
                        })?;
                    return Ok(Some(ResolvedCredential {
                        value: resolved,
                        source: CredentialSource::Stored,
                        oauth: None,
                    }));
                }
                Some("oauth") => {
                    let service = service.ok_or_else(|| {
                        DomainError::Unavailable("unsupported OAuth provider".into())
                    })?;
                    let expires = value["expires"]
                        .as_i64()
                        .ok_or_else(|| DomainError::InvalidInput("OAuth expiry".into()))?;
                    if i128::from(expires) <= i128::from(self.clock.now_ms()) {
                        let refreshed = self.refresh.refresh(service, &record.raw).await?;
                        value = parse(&refreshed)?;
                        // Validate the new record before persistence; never replace corrupt
                        // or failed refresh state with an empty credential.
                        let _ = key_for(service, &value)?;
                        if value["expires"].as_i64().is_none() || value["type"] != "oauth" {
                            return Err(DomainError::InvalidInput(
                                "refreshed credential shape".into(),
                            ));
                        }
                        self.backend
                            .compare_exchange(provider, record.revision, refreshed)
                            .await?;
                    }
                    return Ok(Some(ResolvedCredential {
                        value: key_for(service, &value)?,
                        source: CredentialSource::Stored,
                        oauth: Some(service),
                    }));
                }
                Some(REVOKED_CREDENTIAL_TYPE) => {}
                _ => return Err(DomainError::InvalidInput("unknown credential type".into())),
            }
        }
        if include_fallback
            && let Some(value) = self
                .backend
                .resolve(provider, CredentialSource::Fallback)
                .await?
        {
            return Ok(Some(ResolvedCredential {
                value,
                source: CredentialSource::Fallback,
                oauth: None,
            }));
        }
        Ok(None)
    }
}
pub struct HttpOAuthRefresh {
    http: ProviderHttp,
    clock: Arc<dyn Clock>,
}
impl HttpOAuthRefresh {
    pub fn new(http: ProviderHttp, clock: Arc<dyn Clock>) -> Self {
        Self { http, clock }
    }
}
/// Build an OAuth request without performing I/O or modifying the credential vault.
pub fn refresh_request(
    service: OAuthService,
    raw: &SecretBytes,
) -> Result<HttpRequest, DomainError> {
    if matches!(service, OAuthService::GeminiCli | OAuthService::Antigravity) {
        return Err(DomainError::Unavailable(
            "OAuth provider has placeholder client credentials at the pinned source revision"
                .into(),
        ));
    }
    let value = parse(raw)?;
    let refresh = secret_field(&value, "refresh")?;
    let mut headers = HostHeaders::default();
    let refresh_text = std::str::from_utf8(refresh.expose())
        .map_err(|_| DomainError::InvalidInput("OAuth refresh UTF-8".into()))?;
    let (url, method, body) = match service {
        OAuthService::Anthropic => {
            headers.insert_public("content-type", "application/json")?;
            headers.insert_public("accept", "application/json")?;
            ("https://platform.claude.com/v1/oauth/token".into(),HttpMethod::Post,serde_json::to_vec(&serde_json::json!({"grant_type":"refresh_token","client_id":"9d1c250a-e61b-44d9-88ed-5944d1962f5e","refresh_token":refresh_text})).map_err(|_|DomainError::InvalidInput("OAuth refresh JSON".into()))?)
        }
        OAuthService::Codex => {
            headers.insert_public("content-type", "application/x-www-form-urlencoded")?;
            let mut url = reqwest::Url::parse("https://auth.openai.com/oauth/token")
                .expect("static OAuth URL");
            url.query_pairs_mut()
                .append_pair("grant_type", "refresh_token")
                .append_pair("refresh_token", refresh_text)
                .append_pair("client_id", "app_EMoamEEZ73f0CkXaXp7hrann");
            let body = url.query().expect("form body").as_bytes().to_vec();
            url.set_query(None);
            (url.to_string(), HttpMethod::Post, body)
        }
        OAuthService::GitHubCopilot => {
            let domain = value["enterpriseUrl"].as_str().unwrap_or("github.com");
            if domain.is_empty()
                || !domain
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
            {
                return Err(DomainError::InvalidInput(
                    "Copilot enterprise domain".into(),
                ));
            }
            headers.insert_secret(
                "authorization",
                &SecretBytes::new(format!("Bearer {refresh_text}").into_bytes()),
            )?;
            headers.insert_public("accept", "application/json")?;
            headers.insert_public("user-agent", "GitHubCopilotChat/0.35.0")?;
            headers.insert_public("editor-version", "vscode/1.107.0")?;
            headers.insert_public("editor-plugin-version", "copilot-chat/0.35.0")?;
            headers.insert_public("copilot-integration-id", "vscode-chat")?;
            (
                format!("https://api.{domain}/copilot_internal/v2/token"),
                HttpMethod::Get,
                vec![],
            )
        }
        _ => unreachable!("placeholder providers rejected above"),
    };
    Ok(HttpRequest {
        url,
        method,
        headers,
        body,
        max_response_bytes: 1024 * 1024,
    })
}
impl OAuthRefresh for HttpOAuthRefresh {
    fn refresh<'a>(
        &'a self,
        service: OAuthService,
        raw: &'a SecretBytes,
    ) -> PortFuture<'a, SecretBytes> {
        Box::pin(async move {
            let value = parse(raw)?;
            let request = refresh_request(service, raw)?;
            let (_source, mut cancel) = cancellation();
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut response = self.http.send(request, deadline, &mut cancel).await?;
            if !(200..300).contains(&response.status()) {
                return Err(DomainError::Unavailable(format!(
                    "OAuth refresh HTTP {}",
                    response.status()
                )));
            }
            let mut bytes = vec![];
            while let Some(chunk) = response.chunk(deadline, &mut cancel).await? {
                bytes.extend(chunk);
            }
            refreshed_record(
                service,
                &value,
                &SecretBytes::new(bytes),
                self.clock.now_ms(),
            )
        })
    }
}
/// Pure response validation used by fixture transports and the real host HTTP adapter.
pub fn refreshed_record(
    service: OAuthService,
    old: &Value,
    response: &SecretBytes,
    now_ms: u64,
) -> Result<SecretBytes, DomainError> {
    let data = parse(response)?;
    let mut merged = old.clone();
    let (access, refresh, expires) = if service == OAuthService::GitHubCopilot {
        let access = secret_field(&data, "token")?;
        let expires = data["expires_at"]
            .as_i64()
            .and_then(|v| v.checked_mul(1000))
            .and_then(|v| v.checked_sub(300000))
            .ok_or_else(|| DomainError::InvalidInput("Copilot expiry".into()))?;
        (access, secret_field(old, "refresh")?, expires)
    } else {
        let access = secret_field(&data, "access_token")?;
        let refresh = if matches!(service, OAuthService::GeminiCli | OAuthService::Antigravity)
            && data["refresh_token"].as_str().is_none_or(str::is_empty)
        {
            secret_field(old, "refresh")?
        } else {
            secret_field(&data, "refresh_token")?
        };
        let expires = data["expires_in"]
            .as_i64()
            .filter(|v| *v > 0)
            .and_then(|v| v.checked_mul(1000))
            .and_then(|v| {
                i64::try_from(now_ms)
                    .ok()
                    .and_then(|now| v.checked_add(now))
            })
            .and_then(|v| v.checked_sub(300000))
            .ok_or_else(|| DomainError::InvalidInput("OAuth expires_in".into()))?;
        (access, refresh, expires)
    };
    merged["type"] = "oauth".into();
    if service == OAuthService::Codex {
        merged["accountId"] = codex_account_id(&access)?.into();
    }
    merged["access"] = std::str::from_utf8(access.expose())
        .map_err(|_| DomainError::InvalidInput("OAuth access UTF-8".into()))?
        .into();
    merged["refresh"] = std::str::from_utf8(refresh.expose())
        .map_err(|_| DomainError::InvalidInput("OAuth refresh UTF-8".into()))?
        .into();
    merged["expires"] = expires.into();
    Ok(SecretBytes::new(serde_json::to_vec(&merged).map_err(
        |_| DomainError::InvalidInput("OAuth record serialization".into()),
    )?))
}

#[cfg(test)]
mod oracle_tests {
    use super::*;
    #[test]
    fn oauth_refresh_requests_and_results_match_original_sdk() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../../../fixtures/providers/oauth.json"))
                .unwrap();
        for group in fixture["groups"].as_array().unwrap() {
            let service = match group["service"].as_str().unwrap() {
                "anthropic" => OAuthService::Anthropic,
                "openai-codex" => OAuthService::Codex,
                "github-copilot" => OAuthService::GitHubCopilot,
                _ => panic!("uncovered OAuth family"),
            };
            let raw = SecretBytes::new(serde_json::to_vec(&group["old"]).unwrap());
            let request = refresh_request(service, &raw).unwrap();
            assert_eq!(request.url, group["request"]["url"]);
            assert_eq!(
                match request.method {
                    HttpMethod::Get => "GET",
                    HttpMethod::Post => "POST",
                },
                group["request"]["method"]
            );
            let headers: BTreeMap<_, _> = request
                .headers
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.to_str().unwrap()))
                .collect();
            assert_eq!(
                serde_json::to_value(headers).unwrap(),
                group["request"]["headers"]
            );
            let body = if request.body.is_empty() {
                Value::Null
            } else if group["request"]["body"].is_object() {
                serde_json::from_slice(&request.body).unwrap()
            } else {
                Value::String(String::from_utf8(request.body).unwrap())
            };
            assert_eq!(body, group["request"]["body"]);
            let result = refreshed_record(
                service,
                &group["old"],
                &SecretBytes::new(serde_json::to_vec(&group["response"]).unwrap()),
                1_000_000,
            )
            .unwrap();
            let mut value = parse(&result).unwrap();
            value.as_object_mut().unwrap().remove("type");
            assert_eq!(value, group["result"]);
        }
    }
}
