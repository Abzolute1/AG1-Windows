//! Native, one-use PKCE and loopback callback custody. No codes or state in DTOs.
use super::{
    Authorization, DomainError, Network, invalid,
    network::{check, within},
    url,
};
use crate::{
    mcp::auth::{Binding, MAX_SECRET_BYTES, Target},
    provider::http::CancelToken,
};
use ag1_core::SecretBytes;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::SemaphorePermit,
    time::Instant,
};
static LOGINS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
const CALLBACK_PATH: &str = "/ag1/mcp/oauth/callback";
const CALLBACK_BYTES: usize = 64 * 1024;
const FORM_BYTES: usize = 128 * 1024;

/// Native browser launch port. Implementations must not log/serialize the URL.
pub trait Browser: Send + Sync {
    fn open_authorization(&self, url: &SecretBytes) -> Result<(), DomainError>;
}
/// Public client registration; confidential-client credentials need a separate
/// supported method, never a provider/GitHub secret or implicit fallback.
pub struct ClientRegistration {
    pub(super) issuer: String,
    pub(super) client_id: String,
}
impl ClientRegistration {
    pub fn pre_registered(issuer: &str, client_id: &str) -> Result<Self, DomainError> {
        url(issuer, false)?;
        if client_id.is_empty() || client_id.len() > 4096 || client_id.chars().any(char::is_control)
        {
            return Err(invalid("OAuth public client ID"));
        }
        Ok(Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
        })
    }
    pub fn issuer(&self) -> &str {
        &self.issuer
    }
    pub fn client_id(&self) -> &str {
        &self.client_id
    }
}
pub struct Login {
    listener: TcpListener,
    auth: Authorization,
    registration: ClientRegistration,
    binding: Binding,
    state: SecretBytes,
    verifier: SecretBytes,
    authorization_url: SecretBytes,
    redirect_uri: String,
    attempted: bool,
    opened: bool,
    deadline: Instant,
    permit: SemaphorePermit<'static>,
}
/// Only a validated, consumed callback can construct this value. Token exchange
/// consumes it; callers receive only the nonsecret binding for manager CAS.
pub struct AuthorizedCode {
    pub(super) auth: Authorization,
    pub(super) registration: ClientRegistration,
    pub(super) binding: Binding,
    pub(super) code: SecretBytes,
    pub(super) verifier: SecretBytes,
    pub(super) redirect_uri: String,
    pub(super) deadline: Instant,
    pub(super) _permit: SemaphorePermit<'static>,
}
impl AuthorizedCode {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
}
struct Buffer(Vec<u8>);
impl Drop for Buffer {
    fn drop(&mut self) {
        drop(SecretBytes::new(std::mem::take(&mut self.0)));
    }
}
impl Buffer {
    fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }
    fn put(&mut self, bytes: &[u8]) -> Result<(), DomainError> {
        if bytes.len() > self.0.capacity() - self.0.len() {
            return Err(DomainError::LimitExceeded(
                "MCP OAuth private buffer".into(),
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(())
    }
    fn secret(mut self) -> SecretBytes {
        SecretBytes::new(std::mem::take(&mut self.0))
    }
}
fn random() -> Result<SecretBytes, DomainError> {
    let mut bytes = Buffer(vec![0; 32]);
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes.0))
        .map_err(|_| DomainError::Unavailable("MCP OAuth randomness unavailable".into()))?;
    Ok(SecretBytes::new(
        URL_SAFE_NO_PAD.encode(&bytes.0).into_bytes(),
    ))
}
fn encode(output: &mut Buffer, bytes: &[u8]) -> Result<(), DomainError> {
    const HEX: &[u8] = b"0123456789ABCDEF";
    for &b in bytes {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            output.put(&[b])?;
        } else {
            output.put(&[b'%', HEX[(b >> 4) as usize], HEX[(b & 15) as usize]])?;
        }
    }
    Ok(())
}
pub(super) fn form(pairs: &[(&str, &[u8])]) -> Result<SecretBytes, DomainError> {
    let mut output = Buffer::new(FORM_BYTES);
    for (i, (key, value)) in pairs.iter().enumerate() {
        if i != 0 {
            output.put(b"&")?;
        }
        encode(&mut output, key.as_bytes())?;
        output.put(b"=")?;
        encode(&mut output, value)?;
    }
    Ok(output.secret())
}
impl Network {
    pub async fn begin_login(
        &self,
        auth: Authorization,
        registration: ClientRegistration,
        binding: Binding,
        cancel: &mut CancelToken,
    ) -> Result<Login, DomainError> {
        auth.check_code_flow()?;
        binding.validate()?;
        if registration.issuer != auth.view.issuer
            || !matches!(&binding.target,Target::OAuth{endpoint,resource,issuer,token_endpoint,client_id} if endpoint==&auth.resource.endpoint&&resource==&auth.resource.resource&&issuer==&auth.view.issuer&&token_endpoint==&auth.view.token_endpoint&&client_id==&registration.client_id)
        {
            return Err(DomainError::PermissionDenied);
        }
        if !auth
            .view
            .token_auth_methods
            .iter()
            .any(|method| method == "none")
        {
            return Err(DomainError::Unavailable(
                "MCP OAuth public-client token authentication unsupported".into(),
            ));
        }
        let authorization_endpoint = url(&auth.view.authorization_endpoint, true)?;
        if authorization_endpoint.query_pairs().any(|(key, _)| {
            [
                "client_id",
                "redirect_uri",
                "response_type",
                "scope",
                "resource",
                "state",
                "code_challenge",
                "code_challenge_method",
                "nonce",
            ]
            .contains(&key.as_ref())
        }) {
            return Err(invalid(
                "OAuth authorization endpoint overrides client parameters",
            ));
        }
        let deadline = auth.expires.min(Instant::now() + Duration::from_secs(600));
        self.validate_destination(&authorization_endpoint, deadline, cancel)
            .await?;
        let permit = LOGINS.try_acquire().map_err(|_| DomainError::QueueFull)?;
        let listener = match TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(_) => TcpListener::bind("[::1]:0").await.map_err(|_| {
                DomainError::Unavailable("MCP OAuth loopback listener unavailable".into())
            })?,
        };
        let address = listener
            .local_addr()
            .map_err(|_| invalid("OAuth callback address"))?;
        if !address.ip().is_loopback() {
            return Err(DomainError::PermissionDenied);
        }
        let redirect_uri = format!("http://{address}{CALLBACK_PATH}");
        let verifier = random()?;
        let state = random()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.expose()));
        let scopes = auth
            .resource
            .scopes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let mut pairs = vec![
            ("response_type", b"code".as_slice()),
            ("client_id", registration.client_id.as_bytes()),
            ("redirect_uri", redirect_uri.as_bytes()),
            ("resource", auth.resource.resource.as_bytes()),
            ("state", state.expose()),
            ("code_challenge", challenge.as_bytes()),
            ("code_challenge_method", b"S256".as_slice()),
        ];
        if !scopes.is_empty() {
            pairs.push(("scope", scopes.as_bytes()));
        }
        let query = form(&pairs)?;
        let mut output = Buffer::new(FORM_BYTES);
        output.put(auth.view.authorization_endpoint.as_bytes())?;
        output.put(if authorization_endpoint.query().is_none() {
            b"?"
        } else if auth.view.authorization_endpoint.ends_with(['?', '&']) {
            b""
        } else {
            b"&"
        })?;
        output.put(query.expose())?;
        check(deadline, cancel)?;
        Ok(Login {
            listener,
            auth,
            registration,
            binding,
            state,
            verifier,
            authorization_url: output.secret(),
            redirect_uri,
            attempted: false,
            opened: false,
            deadline,
            permit,
        })
    }
}
impl Login {
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
    pub fn open(&mut self, browser: &dyn Browser, cancel: &CancelToken) -> Result<(), DomainError> {
        check(self.deadline, cancel)?;
        if self.attempted {
            return Err(DomainError::PermissionDenied);
        }
        self.attempted = true;
        browser.open_authorization(&self.authorization_url)?;
        check(self.deadline, cancel)?;
        self.opened = true;
        Ok(())
    }
    pub async fn wait(self, cancel: &mut CancelToken) -> Result<AuthorizedCode, DomainError> {
        if !self.opened {
            return Err(DomainError::PermissionDenied);
        }
        for _ in 0..32 {
            let (mut stream, address) = within(self.deadline, cancel, async {
                self.listener
                    .accept()
                    .await
                    .map_err(|_| invalid("OAuth callback listener"))
            })
            .await?;
            if !address.ip().is_loopback() {
                continue;
            }
            let deadline = self.deadline.min(Instant::now() + Duration::from_secs(2));
            let incoming = read_request(&stream, deadline, cancel).await;
            let verdict = match incoming {
                Ok(raw) => self.validate_callback(raw.expose()),
                Err(DomainError::Cancelled) => return Err(DomainError::Cancelled),
                Err(_) => Ok(None),
            };
            let message = if matches!(verdict, Ok(Some(_))) {
                "Sign-in response received. Return to AG1."
            } else {
                "This sign-in response was not accepted. Return to AG1."
            };
            let status = if matches!(verdict, Ok(Some(_))) {
                "200 OK"
            } else {
                "400 Bad Request"
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
                message.len()
            );
            let _ = write_response(&mut stream, response.as_bytes(), deadline, cancel).await;
            if let Some(code) = verdict? {
                check(self.deadline, cancel)?;
                return Ok(AuthorizedCode {
                    auth: self.auth,
                    registration: self.registration,
                    binding: self.binding,
                    code,
                    verifier: self.verifier,
                    redirect_uri: self.redirect_uri,
                    deadline: self.deadline,
                    _permit: self.permit,
                });
            }
        }
        Err(DomainError::LimitExceeded(
            "MCP OAuth callback attempts".into(),
        ))
    }
    fn validate_callback(&self, raw: &[u8]) -> Result<Option<SecretBytes>, DomainError> {
        let Ok(request) = std::str::from_utf8(raw) else {
            return Ok(None);
        };
        let Some((headers, body)) = request.split_once("\r\n\r\n") else {
            return Ok(None);
        };
        if !body.is_empty() {
            return Ok(None);
        }
        let mut lines = headers.split("\r\n");
        let Some(line) = lines.next() else {
            return Ok(None);
        };
        let parts = line.split(' ').collect::<Vec<_>>();
        if parts.len() != 3 || parts[0] != "GET" || parts[2] != "HTTP/1.1" {
            return Ok(None);
        }
        let host = self
            .redirect_uri
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap();
        let mut hosts = 0;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                return Ok(None);
            };
            if name.eq_ignore_ascii_case("host") {
                hosts += 1;
                if value.trim() != host {
                    return Ok(None);
                }
            }
            if name.eq_ignore_ascii_case("transfer-encoding")
                || name.eq_ignore_ascii_case("content-length") && value.trim() != "0"
            {
                return Ok(None);
            }
        }
        if hosts != 1 {
            return Ok(None);
        }
        let Some((path, query)) = parts[1].split_once('?') else {
            return Ok(None);
        };
        if path != CALLBACK_PATH {
            return Ok(None);
        }
        let Ok(mut params) = parameters(query) else {
            return Ok(None);
        };
        let Some(state) = params.remove("state") else {
            return Ok(None);
        };
        if !state_matches(self.state.expose(), state.expose()) {
            return Ok(None);
        }
        // Validate issuer before interpreting success or error fields. No URL
        // normalization, error description, external navigation or token send.
        match params.remove("iss") {
            Some(issuer) if issuer.expose() != self.auth.view.issuer.as_bytes() => {
                return Err(invalid("OAuth callback issuer mismatch"));
            }
            None if self.auth.view.issuer_response_required => {
                return Err(invalid("OAuth callback issuer missing"));
            }
            _ => {}
        }
        if params.contains_key("error") {
            if params.contains_key("code") {
                return Err(invalid("OAuth callback has success and error"));
            }
            return Err(DomainError::Unavailable(
                "MCP authorization server declined sign-in".into(),
            ));
        }
        let code = params
            .remove("code")
            .ok_or_else(|| invalid("OAuth callback code missing"))?;
        if code.expose().is_empty()
            || code.expose().len() > MAX_SECRET_BYTES
            || std::str::from_utf8(code.expose()).is_err()
            || code.expose().iter().any(|b| b.is_ascii_control())
        {
            return Err(invalid("OAuth callback code"));
        }
        Ok(Some(code))
    }
}
fn state_matches(expected: &[u8], received: &[u8]) -> bool {
    if expected.len() != received.len() {
        return false;
    }
    let mut left = Hmac::<Sha256>::new_from_slice(b"AG1 MCP OAuth state comparison")
        .expect("HMAC accepts this key");
    left.update(expected);
    let mut right = Hmac::<Sha256>::new_from_slice(b"AG1 MCP OAuth state comparison")
        .expect("HMAC accepts this key");
    right.update(received);
    left.verify_slice(&right.finalize().into_bytes()).is_ok()
}
fn component(raw: &str) -> Result<SecretBytes, DomainError> {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let mut out = Buffer::new(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = match bytes[i] {
            b'+' => b' ',
            b'%' => {
                if i + 2 >= bytes.len() {
                    return Err(invalid("OAuth callback encoding"));
                }
                let b = hex(bytes[i + 1])
                    .zip(hex(bytes[i + 2]))
                    .map(|(a, b)| a * 16 + b)
                    .ok_or_else(|| invalid("OAuth callback encoding"))?;
                i += 2;
                b
            }
            b => b,
        };
        out.put(&[b])?;
        i += 1;
    }
    Ok(out.secret())
}
fn parameters(query: &str) -> Result<BTreeMap<String, SecretBytes>, DomainError> {
    let mut result = BTreeMap::new();
    for item in query.split('&') {
        if result.len() >= 32 {
            return Err(invalid("OAuth callback parameter count"));
        }
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| invalid("OAuth callback parameter"))?;
        let key = component(key)?;
        let key = std::str::from_utf8(key.expose()).map_err(|_| invalid("OAuth callback key"))?;
        if key.is_empty()
            || key.len() > 128
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            return Err(invalid("OAuth callback key"));
        }
        if result.insert(key.to_owned(), component(value)?).is_some() {
            return Err(invalid("duplicate OAuth callback parameter"));
        }
    }
    Ok(result)
}
async fn read_request(
    stream: &TcpStream,
    deadline: Instant,
    cancel: &mut CancelToken,
) -> Result<SecretBytes, DomainError> {
    let mut bytes = Buffer::new(CALLBACK_BYTES);
    let mut chunk = Buffer(vec![0; 2048]);
    loop {
        within(deadline, cancel, async {
            stream
                .readable()
                .await
                .map_err(|_| invalid("OAuth callback read"))
        })
        .await?;
        match stream.try_read(&mut chunk.0) {
            Ok(0) => return Err(invalid("OAuth callback truncated")),
            Ok(n) => {
                bytes.put(&chunk.0[..n])?;
                if bytes.0.windows(4).any(|w| w == b"\r\n\r\n") {
                    return Ok(bytes.secret());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => return Err(invalid("OAuth callback read")),
        }
    }
}
async fn write_response(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
    cancel: &mut CancelToken,
) -> Result<(), DomainError> {
    let mut written = 0;
    while written < bytes.len() {
        within(deadline, cancel, async {
            stream
                .writable()
                .await
                .map_err(|_| invalid("OAuth callback write"))
        })
        .await?;
        match stream.try_write(&bytes[written..]) {
            Ok(0) => return Err(invalid("OAuth callback closed")),
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => return Err(invalid("OAuth callback write")),
        }
    }
    Ok(())
}
