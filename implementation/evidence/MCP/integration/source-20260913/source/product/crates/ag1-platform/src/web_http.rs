//! Host-owned web transport. Every redirect is resolved, checked and pinned before
//! connecting. No ambient proxy, cookie jar, credential discovery or TLS bypass.
use crate::provider::http::CancelToken;
use ag1_core::{DomainError, PortFuture, SecretBytes};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::{
    collections::BTreeSet,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::time::Instant;

pub const MAX_FETCH_BYTES: usize = 1_000_000;
static FETCH_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
pub trait WebResolver: Send + Sync {
    fn resolve<'a>(&'a self, hostname: &'a str, port: u16) -> PortFuture<'a, Vec<IpAddr>>;
}
pub struct SystemResolver;
impl WebResolver for SystemResolver {
    fn resolve<'a>(&'a self, hostname: &'a str, port: u16) -> PortFuture<'a, Vec<IpAddr>> {
        Box::pin(async move {
            let values = tokio::net::lookup_host((hostname, port))
                .await
                .map_err(|_| DomainError::Unavailable("web DNS lookup failed".into()))?;
            let mut result = BTreeSet::new();
            for address in values {
                result.insert(address.ip());
                if result.len() > 32 {
                    return Err(DomainError::LimitExceeded("web DNS addresses".into()));
                }
            }
            Ok(result.into_iter().collect())
        })
    }
}
/// Public Internet destinations only. Mapped IPv4 is checked as IPv4; transition
/// ranges are rejected so a public-looking IPv6 literal cannot tunnel to a LAN.
pub fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(matches!(a, 0 | 10 | 127 | 224..=255)
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || (b == 0 && matches!(c, 0 | 2))))
                || (a == 198 && (matches!(b, 18 | 19) || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return public_address(IpAddr::V4(v4));
            }
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && matches!(s[1], 0 | 0xdb8))
        }
    }
}
fn parse_url(value: &str) -> Result<Url, DomainError> {
    if value.len() > 8192 {
        return Err(DomainError::LimitExceeded("web URL".into()));
    }
    let mut url =
        Url::parse(value).map_err(|_| DomainError::InvalidInput("invalid web URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(DomainError::PermissionDenied);
    }
    url.set_fragment(None);
    Ok(url)
}
fn host(url: &Url) -> &str {
    url.host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
}

/// Created by the trusted host, never from tool arguments or workspace env.
/// Exceptions are exact origins, including scheme and port (e.g. a fixture server).
#[derive(Default)]
pub struct WebPolicy {
    private_origins: BTreeSet<String>,
}
impl WebPolicy {
    pub fn with_private_origins(origins: &[String]) -> Result<Self, DomainError> {
        if origins.len() > 32 {
            return Err(DomainError::LimitExceeded("web origin exceptions".into()));
        }
        let mut values = BTreeSet::new();
        for value in origins {
            let url = parse_url(value)?;
            if url.path() != "/" || url.query().is_some() || value.contains('#') {
                return Err(DomainError::InvalidInput(
                    "web exception must be an origin".into(),
                ));
            }
            values.insert(url.origin().ascii_serialization());
        }
        Ok(Self {
            private_origins: values,
        })
    }
    pub fn validate(&self, url: &str, addresses: &[IpAddr]) -> Result<(), DomainError> {
        let url = parse_url(url)?;
        if addresses.is_empty() {
            return Err(DomainError::Unavailable(
                "web DNS returned no addresses".into(),
            ));
        }
        if addresses.len() > 32 {
            return Err(DomainError::LimitExceeded("web DNS addresses".into()));
        }
        if !self
            .private_origins
            .contains(&url.origin().ascii_serialization())
        {
            let hostname = host(&url).trim_end_matches('.').to_ascii_lowercase();
            if hostname == "localhost"
                || hostname.ends_with(".localhost")
                || hostname
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| !public_address(ip))
                || addresses.iter().any(|ip| !public_address(*ip))
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        Ok(())
    }
}
/// Credential material is host supplied and has no Debug or serialization surface.
/// Only HTTPS hosts in this allowlist receive it; redirects recheck the allowlist.
pub struct WebCredential {
    hosts: Vec<String>,
    name: HeaderName,
    value: HeaderValue,
}
impl WebCredential {
    pub fn new(hosts: &[String], header: &str, secret: &SecretBytes) -> Result<Self, DomainError> {
        if hosts.is_empty() || hosts.len() > 32 || secret.expose().len() > 16384 {
            return Err(DomainError::LimitExceeded(
                "web credential configuration".into(),
            ));
        }
        let mut allow = vec![];
        for value in hosts {
            let name = value
                .trim()
                .trim_start_matches('.')
                .trim_end_matches('.')
                .to_ascii_lowercase();
            let url = parse_url(&format!("https://{name}/"))?;
            if host(&url) != name
                || url.port().is_some()
                || url.path() != "/"
                || url.query().is_some()
            {
                return Err(DomainError::InvalidInput("web credential hostname".into()));
            }
            allow.push(name);
        }
        let name = HeaderName::from_bytes(header.as_bytes())
            .map_err(|_| DomainError::InvalidInput("web credential header".into()))?;
        if matches!(
            name.as_str(),
            "host"
                | "content-length"
                | "transfer-encoding"
                | "connection"
                | "proxy-authorization"
                | "cookie"
        ) {
            return Err(DomainError::PermissionDenied);
        }
        let mut value = HeaderValue::from_bytes(secret.expose())
            .map_err(|_| DomainError::InvalidInput("web credential value".into()))?;
        value.set_sensitive(true);
        Ok(Self {
            hosts: allow,
            name,
            value,
        })
    }
    pub fn permits(&self, value: &str) -> bool {
        let Ok(url) = parse_url(value) else {
            return false;
        };
        let hostname = host(&url).trim_end_matches('.').to_ascii_lowercase();
        url.scheme() == "https"
            && self
                .hosts
                .iter()
                .any(|allowed| hostname == *allowed || hostname.ends_with(&format!(".{allowed}")))
    }
    fn headers(&self, url: &Url) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if self.permits(url.as_str()) {
            headers.insert(self.name.clone(), self.value.clone());
        }
        headers
    }
}
#[derive(Debug)]
pub struct WebResponse {
    pub requested_url: String,
    pub final_url: String,
    pub redirect_chain: Vec<String>,
    pub status: u16,
    pub content_type: String,
    pub bytes: Vec<u8>,
}
pub struct WebHttp<R = SystemResolver> {
    policy: WebPolicy,
    resolver: R,
    certificates: Vec<reqwest::Certificate>,
}
#[derive(Clone, Copy)]
pub enum WebPurpose {
    Fetch,
    SearchHtml,
    SearchJson,
}
impl WebHttp<SystemResolver> {
    pub fn new(policy: WebPolicy) -> Self {
        Self {
            policy,
            resolver: SystemResolver,
            certificates: vec![],
        }
    }
}
impl<R: WebResolver> WebHttp<R> {
    /// Resolver injection is a trusted host capability, useful for deterministic DNS tests.
    pub fn with_resolver(policy: WebPolicy, resolver: R) -> Self {
        Self {
            policy,
            resolver,
            certificates: vec![],
        }
    }
    /// Additional trust roots are a host capability. This does not disable normal
    /// certificate-chain or hostname verification, including on redirects.
    pub fn with_certificates(mut self, certificates: &[Vec<u8>]) -> Result<Self, DomainError> {
        if certificates.len() > 4 || certificates.iter().any(|v| v.len() > 65536) {
            return Err(DomainError::LimitExceeded("web TLS trust roots".into()));
        }
        self.certificates = certificates
            .iter()
            .map(|pem| {
                reqwest::Certificate::from_pem(pem)
                    .map_err(|_| DomainError::InvalidInput("web TLS trust root".into()))
            })
            .collect::<Result<_, _>>()?;
        Ok(self)
    }
    pub async fn fetch(
        &self,
        value: &str,
        credential: Option<&WebCredential>,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<WebResponse, DomainError> {
        self.fetch_for(value, credential, WebPurpose::Fetch, deadline, cancel)
            .await
    }
    pub async fn fetch_for(
        &self,
        value: &str,
        credential: Option<&WebCredential>,
        purpose: WebPurpose,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<WebResponse, DomainError> {
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        let _permit = FETCH_SLOTS
            .try_acquire()
            .map_err(|_| DomainError::QueueFull)?;
        let mut url = parse_url(value)?;
        if credential.is_some_and(|credential| !credential.permits(url.as_str())) {
            return Err(DomainError::PermissionDenied);
        }
        let requested_url = url.to_string();
        let deadline = deadline.min(Instant::now() + Duration::from_secs(30));
        let mut chain = vec![];
        for hop in 0..=5 {
            if cancel.cancelled() {
                return Err(DomainError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(DomainError::TimedOut);
            }
            let hostname = host(&url);
            let port = url
                .port_or_known_default()
                .ok_or(DomainError::PermissionDenied)?;
            let addresses = if let Ok(ip) = hostname.parse::<IpAddr>() {
                vec![ip]
            } else {
                tokio::select! {
                    biased;
                    _=cancel.wait()=>return Err(DomainError::Cancelled),
                    _=tokio::time::sleep_until(deadline)=>return Err(DomainError::TimedOut),
                    value=self.resolver.resolve(hostname, port)=>value?,
                }
            };
            self.policy.validate(url.as_str(), &addresses)?;
            let sockets: Vec<_> = addresses
                .into_iter()
                .map(|ip| SocketAddr::new(ip, port))
                .collect();
            // A fresh client prevents pooled connections or a second DNS lookup from
            // escaping this hop's verified address set. TLS still verifies URL hostname.
            let mut builder = Client::builder()
                .no_proxy()
                .user_agent(match purpose {
                    WebPurpose::Fetch => "AG1 webFetch",
                    WebPurpose::SearchHtml => "Mozilla/5.0",
                    WebPurpose::SearchJson => "AG1 webSearch",
                })
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .pool_max_idle_per_host(0)
                .resolve_to_addrs(hostname, &sockets);
            for certificate in &self.certificates {
                builder = builder.add_root_certificate(certificate.clone());
            }
            let client = builder
                .build()
                .map_err(|_| DomainError::Unavailable("web TLS initialization".into()))?;
            let mut headers = credential.map(|c| c.headers(&url)).unwrap_or_default();
            match purpose {
                WebPurpose::Fetch => {}
                WebPurpose::SearchHtml => {
                    headers.insert(
                        reqwest::header::ACCEPT,
                        HeaderValue::from_static("text/html,application/xhtml+xml"),
                    );
                }
                WebPurpose::SearchJson => {
                    headers.insert(
                        reqwest::header::ACCEPT,
                        HeaderValue::from_static("application/json"),
                    );
                }
            }
            let mut response = tokio::select! {
                biased;
                _=cancel.wait()=>return Err(DomainError::Cancelled),
                _=tokio::time::sleep_until(deadline)=>return Err(DomainError::TimedOut),
                response=client.get(url.clone()).headers(headers).send()=>response.map_err(|e| if e.is_timeout() { DomainError::TimedOut } else { DomainError::Unavailable("web connection failed".into()) })?,
            };
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err(DomainError::LimitExceeded("web redirects".into()));
                }
                let location = response
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| {
                        DomainError::InvalidInput("web redirect missing Location".into())
                    })?;
                if location.len() > 8192 {
                    return Err(DomainError::LimitExceeded("web redirect URL".into()));
                }
                let next = url
                    .join(location)
                    .map_err(|_| DomainError::InvalidInput("web redirect URL".into()))?;
                chain.push(url.to_string());
                url = parse_url(next.as_str())?;
                continue;
            }
            if !response.status().is_success() {
                return Err(DomainError::Unavailable(format!(
                    "web HTTP status {}",
                    response.status().as_u16()
                )));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_FETCH_BYTES as u64)
            {
                return Err(DomainError::LimitExceeded(
                    "web response content length".into(),
                ));
            }
            let status = response.status().as_u16();
            let content_type = response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            if content_type.len() > 4096 {
                return Err(DomainError::LimitExceeded("web content type".into()));
            }
            let mut bytes = Vec::new();
            loop {
                let chunk = tokio::select! {
                    biased;
                    _=cancel.wait()=>return Err(DomainError::Cancelled),
                    _=tokio::time::sleep_until(deadline)=>return Err(DomainError::TimedOut),
                    value=response.chunk()=>value.map_err(|_|DomainError::Unavailable("web response interrupted".into()))?,
                };
                let Some(chunk) = chunk else { break };
                if chunk.len() > MAX_FETCH_BYTES.saturating_sub(bytes.len()) {
                    return Err(DomainError::LimitExceeded("web response body".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            return Ok(WebResponse {
                requested_url,
                final_url: url.to_string(),
                redirect_chain: chain,
                status,
                content_type,
                bytes,
            });
        }
        unreachable!("redirect count checked before continuing")
    }
}
