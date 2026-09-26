//! Native OAuth discovery and authorization coordination. No ambient credentials.
//! Public metadata is not a login, activation or operation-approval capability.
use super::{
    invalid,
    wire::{Limits, decode_json},
};
use crate::provider::http::CancelToken;
use ag1_core::DomainError;
use reqwest::Url;
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};
use tokio::time::Instant;
pub mod challenge;
#[cfg(target_os = "linux")]
pub mod login;
mod network;
pub use network::Network;
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
const MAX_AUTHORIZATION_SERVERS: usize = 8;

pub(super) fn url(value: &str, query: bool) -> Result<Url, DomainError> {
    if value.is_empty()
        || value.len() > 8192
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
    {
        return Err(invalid("OAuth URL"));
    }
    let url = Url::parse(value).map_err(|_| invalid("OAuth URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || (!query && url.query().is_some())
        || url.port_or_known_default() == Some(0)
    {
        return Err(invalid(
            "OAuth requires HTTPS without credentials or fragments",
        ));
    }
    if url.query_pairs().any(|(k, _)| {
        [
            "access_token",
            "refresh_token",
            "client_secret",
            "code",
            "code_verifier",
            "authorization",
            "password",
        ]
        .contains(&k.to_ascii_lowercase().as_str())
    }) {
        return Err(invalid("credential-bearing OAuth URL"));
    }
    Ok(url)
}
pub(super) fn scopes(input: &str) -> Result<BTreeSet<String>, DomainError> {
    let mut result = BTreeSet::new();
    if input.is_empty() {
        return Ok(result);
    }
    for s in input.split(' ') {
        if s.is_empty()
            || s.len() > 256
            || !s
                .bytes()
                .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
        {
            return Err(invalid("OAuth scope"));
        }
        result.insert(s.to_owned());
        if result.len() > 64 {
            return Err(DomainError::LimitExceeded("MCP OAuth scopes".into()));
        }
    }
    Ok(result)
}
fn strings(value: &Value, key: &str, limit: usize) -> Result<Option<Vec<String>>, DomainError> {
    let Some(v) = value.get(key) else {
        return Ok(None);
    };
    let a = v
        .as_array()
        .filter(|a| a.len() <= limit)
        .ok_or_else(|| invalid("OAuth metadata list"))?;
    let mut result = Vec::new();
    for s in a {
        let s = s
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 8192)
            .ok_or_else(|| invalid("OAuth metadata string"))?;
        if result.iter().any(|v| v == s) {
            return Err(invalid("duplicate OAuth metadata list entry"));
        }
        result.push(s.to_owned());
    }
    Ok(Some(result))
}
fn required(value: &Value, key: &str) -> Result<String, DomainError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .map(str::to_owned)
        .ok_or_else(|| invalid("required OAuth metadata field"))
}
fn flag(value: &Value, key: &str) -> Result<bool, DomainError> {
    match value.get(key) {
        None => Ok(false),
        Some(Value::Bool(v)) => Ok(*v),
        _ => Err(invalid("OAuth metadata flag")),
    }
}
fn document(reply: network::Reply) -> Result<Value, DomainError> {
    if reply.status != 200 {
        return Err(DomainError::Unavailable(
            "MCP OAuth metadata unavailable".into(),
        ));
    }
    let mut types = reply.headers.get_all("content-type").iter();
    let kind = types
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next());
    if kind.is_none_or(|s| !s.trim().eq_ignore_ascii_case("application/json"))
        || types.next().is_some()
    {
        return Err(invalid("OAuth metadata content type"));
    }
    let value = decode_json(
        reply.body.expose(),
        Limits::new(MAX_DOCUMENT_BYTES, 16, 4096)?,
    )?;
    if !value.is_object() || value.get("signed_metadata").is_some() {
        return Err(invalid("unsupported signed or non-object OAuth metadata"));
    }
    Ok(value)
}
/// Public projection only. A caller cannot deserialize it into native custody.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceView {
    pub endpoint: String,
    pub resource: String,
    pub metadata_url: String,
    pub authorization_servers: Vec<String>,
    pub scopes: BTreeSet<String>,
}
pub struct Resource {
    view: ResourceView,
    expires: Instant,
}
impl Resource {
    pub fn view(&self) -> &ResourceView {
        &self.view
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthorizationView {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    pub revocation_endpoint: Option<String>,
    pub cimd_supported: bool,
    pub issuer_response_required: bool,
    pub pkce_s256: bool,
    pub authorization_code: bool,
    pub token_auth_methods: Vec<String>,
}
pub struct Authorization {
    resource: ResourceView,
    view: AuthorizationView,
    expires: Instant,
}
impl Authorization {
    pub fn resource(&self) -> &ResourceView {
        &self.resource
    }
    pub fn view(&self) -> &AuthorizationView {
        &self.view
    }
    pub fn check_code_flow(&self) -> Result<(), DomainError> {
        if Instant::now() >= self.expires {
            return Err(DomainError::TimedOut);
        }
        if !self.view.pkce_s256 || !self.view.authorization_code {
            return Err(DomainError::Unavailable(
                "MCP authorization server must advertise S256 PKCE and authorization-code support"
                    .into(),
            ));
        }
        Ok(())
    }
}
fn well_known(identifier: &str, suffix: &str, append: bool) -> Result<String, DomainError> {
    let mut target = url(identifier, false)?;
    let path = target.path().trim_end_matches('/').to_owned();
    target.set_path(&if append {
        format!("{path}/.well-known/{suffix}")
    } else {
        format!("/.well-known/{suffix}{path}")
    });
    Ok(target.into())
}
impl Network {
    /// Explicit metadata discovery only: never invokes an MCP capability or
    /// sends credentials, registration or browser authorization automatically.
    pub async fn discover(
        &self,
        endpoint: &str,
        cancel: &mut CancelToken,
    ) -> Result<Resource, DomainError> {
        let target = url(endpoint, false)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let initial = self.get(endpoint, true, deadline, cancel).await?;
        if ![200, 401, 403, 404, 405, 406].contains(&initial.status) {
            return Err(DomainError::Unavailable(
                "MCP authorization discovery endpoint unavailable".into(),
            ));
        }
        let headers = initial
            .headers
            .get_all("www-authenticate")
            .iter()
            .map(|v| v.to_str().map_err(|_| invalid("OAuth challenge header")))
            .collect::<Result<Vec<_>, _>>()?;
        let challenge = challenge::parse(&headers)?;
        let mut candidates = Vec::new();
        if let Some(metadata) = challenge
            .as_ref()
            .and_then(|c| c.resource_metadata.as_ref())
        {
            url(metadata, true)?;
            candidates.push((metadata.clone(), endpoint.to_owned()));
        } else {
            candidates.push((
                well_known(endpoint, "oauth-protected-resource", false)?,
                endpoint.to_owned(),
            ));
            let origin = target.origin().ascii_serialization();
            let root = (
                well_known(&origin, "oauth-protected-resource", false)?,
                origin,
            );
            if root.0 != candidates[0].0 {
                candidates.push(root);
            }
        }
        for (metadata_url, expected_resource) in candidates {
            let reply = self.get(&metadata_url, false, deadline, cancel).await?;
            if [404, 405].contains(&reply.status) {
                continue;
            }
            let value = document(reply)?;
            let resource = required(&value, "resource")?;
            if resource != expected_resource {
                return Err(invalid("OAuth protected resource identity mismatch"));
            }
            url(&resource, false)?;
            let authorization_servers =
                strings(&value, "authorization_servers", MAX_AUTHORIZATION_SERVERS)?
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| invalid("OAuth authorization server list"))?;
            for issuer in &authorization_servers {
                url(issuer, false)?;
            }
            if strings(&value, "bearer_methods_supported", 8)?
                .is_some_and(|a| !a.iter().any(|s| s == "header"))
            {
                return Err(DomainError::Unavailable(
                    "MCP OAuth server does not support Bearer headers".into(),
                ));
            }
            let metadata_scopes = strings(&value, "scopes_supported", 64)?.unwrap_or_default();
            let mut supported = BTreeSet::new();
            for s in metadata_scopes {
                let parsed = scopes(&s)?;
                if parsed.len() != 1 {
                    return Err(invalid("OAuth metadata scope entry"));
                }
                supported.extend(parsed);
            }
            let selected = challenge
                .as_ref()
                .and_then(|c| c.scopes.clone())
                .unwrap_or(supported);
            return Ok(Resource {
                view: ResourceView {
                    endpoint: endpoint.to_owned(),
                    resource,
                    metadata_url,
                    authorization_servers,
                    scopes: selected,
                },
                expires: Instant::now() + Duration::from_secs(600),
            });
        }
        Err(DomainError::Unavailable(
            "MCP protected resource metadata was not found".into(),
        ))
    }
    /// Selection is an explicit native caller choice among the advertised issuers.
    pub async fn authorization(
        &self,
        resource: &Resource,
        issuer: &str,
        cancel: &mut CancelToken,
    ) -> Result<Authorization, DomainError> {
        if Instant::now() >= resource.expires {
            return Err(DomainError::TimedOut);
        }
        if !resource
            .view
            .authorization_servers
            .iter()
            .any(|s| s == issuer)
        {
            return Err(DomainError::PermissionDenied);
        }
        let target = url(issuer, false)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut candidates = vec![
            well_known(issuer, "oauth-authorization-server", false)?,
            well_known(issuer, "openid-configuration", false)?,
        ];
        if target.path() != "/" {
            let appended = well_known(issuer, "openid-configuration", true)?;
            if !candidates.contains(&appended) {
                candidates.push(appended);
            }
        }
        for metadata_url in candidates {
            let reply = self.get(&metadata_url, false, deadline, cancel).await?;
            if [404, 405].contains(&reply.status) {
                continue;
            }
            let value = document(reply)?;
            if required(&value, "issuer")? != issuer {
                return Err(invalid("OAuth authorization issuer identity mismatch"));
            }
            let authorization_endpoint = required(&value, "authorization_endpoint")?;
            let token_endpoint = required(&value, "token_endpoint")?;
            url(&authorization_endpoint, true)?;
            url(&token_endpoint, true)?;
            let endpoint = |key| -> Result<Option<String>, DomainError> {
                value
                    .get(key)
                    .map(|_| {
                        required(&value, key).and_then(|s| {
                            url(&s, true)?;
                            Ok(s)
                        })
                    })
                    .transpose()
            };
            let response_types = strings(&value, "response_types_supported", 16)?
                .ok_or_else(|| invalid("OAuth response types metadata"))?;
            let grants = strings(&value, "grant_types_supported", 16)?;
            let pkce = strings(&value, "code_challenge_methods_supported", 16)?.unwrap_or_default();
            let view = AuthorizationView {
                issuer: issuer.to_owned(),
                authorization_endpoint,
                token_endpoint,
                registration_endpoint: endpoint("registration_endpoint")?,
                revocation_endpoint: endpoint("revocation_endpoint")?,
                cimd_supported: flag(&value, "client_id_metadata_document_supported")?,
                issuer_response_required: flag(
                    &value,
                    "authorization_response_iss_parameter_supported",
                )?,
                pkce_s256: pkce.iter().any(|s| s == "S256"),
                authorization_code: response_types.iter().any(|s| s == "code")
                    && grants.is_none_or(|a| a.iter().any(|s| s == "authorization_code")),
                token_auth_methods: strings(&value, "token_endpoint_auth_methods_supported", 16)?
                    .unwrap_or_else(|| vec!["client_secret_basic".into()]),
            };
            return Ok(Authorization {
                resource: resource.view.clone(),
                view,
                expires: resource.expires,
            });
        }
        Err(DomainError::Unavailable(
            "MCP OAuth authorization metadata was not found".into(),
        ))
    }
}
