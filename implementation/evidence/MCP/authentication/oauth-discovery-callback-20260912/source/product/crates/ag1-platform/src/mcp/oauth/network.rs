//! HTTPS-only OAuth network custody, with no redirects, ambient auth or retries.
use super::{DomainError, MAX_DOCUMENT_BYTES, invalid, url};
use crate::{
    provider::http::CancelToken,
    web_http::{SystemResolver, WebPolicy, WebResolver},
};
use ag1_core::SecretBytes;
use reqwest::{Client, Method, Url, header::HeaderMap};
use std::{future::Future, net::SocketAddr, sync::Arc, time::Duration};
use tokio::time::Instant;
static REQUESTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

pub struct Network {
    resolver: Arc<dyn WebResolver>,
    policy: WebPolicy,
    roots: Vec<reqwest::Certificate>,
}
pub(super) struct Reply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: SecretBytes,
}
impl Default for Network {
    fn default() -> Self {
        Self {
            resolver: Arc::new(SystemResolver),
            policy: WebPolicy::default(),
            roots: Vec::new(),
        }
    }
}
impl Network {
    /// Native composition/testing port only. No deserialization or renderer path.
    pub fn with_policy(
        resolver: Arc<dyn WebResolver>,
        policy: WebPolicy,
        roots: &[Vec<u8>],
    ) -> Result<Self, DomainError> {
        if roots.len() > 4 || roots.iter().any(|r| r.len() > 64 * 1024) {
            return Err(invalid("OAuth TLS roots"));
        }
        let roots = roots
            .iter()
            .map(|r| reqwest::Certificate::from_pem(r).map_err(|_| invalid("OAuth TLS root")))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            resolver,
            policy,
            roots,
        })
    }
    pub(super) async fn validate_destination(
        &self,
        target: &Url,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<Vec<SocketAddr>, DomainError> {
        check(deadline, cancel)?;
        let host = target
            .host_str()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let port = target
            .port_or_known_default()
            .ok_or_else(|| invalid("OAuth endpoint port"))?;
        let addresses = if let Ok(ip) = host.parse() {
            vec![ip]
        } else {
            within(
                deadline.min(Instant::now() + Duration::from_secs(3)),
                cancel,
                self.resolver.resolve(host, port),
            )
            .await?
        };
        self.policy.validate(target.as_str(), &addresses)?;
        Ok(addresses
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect())
    }
    pub(super) async fn get(
        &self,
        target: &str,
        headers_only: bool,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<Reply, DomainError> {
        self.request(Method::GET, target, None, headers_only, deadline, cancel)
            .await
    }
    pub(super) async fn request(
        &self,
        method: Method,
        target: &str,
        body: Option<(&'static str, SecretBytes)>,
        headers_only: bool,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<Reply, DomainError> {
        let _permit = REQUESTS.try_acquire().map_err(|_| DomainError::QueueFull)?;
        let target = url(target, true)?;
        let addresses = self.validate_destination(&target, deadline, cancel).await?;
        let host = target
            .host_str()
            .unwrap_or_default()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let mut builder = Client::builder()
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .http1_only()
            .pool_max_idle_per_host(0)
            .connect_timeout(Duration::from_secs(3))
            .resolve_to_addrs(host, &addresses);
        for root in &self.roots {
            builder = builder.add_root_certificate(root.clone());
        }
        let client = builder
            .build()
            .map_err(|_| invalid("OAuth TLS client initialization"))?;
        let mut request = client
            .request(method, target)
            .header("accept", "application/json");
        if let Some((content_type, body)) = body {
            if body.expose().len() > MAX_DOCUMENT_BYTES {
                return Err(DomainError::LimitExceeded("MCP OAuth request bytes".into()));
            }
            request = request
                .header("content-type", content_type)
                .body(body.expose().to_vec());
        }
        let mut response = within(deadline, cancel, async {
            request
                .send()
                .await
                .map_err(|_| DomainError::Unavailable("MCP OAuth HTTP/TLS request failed".into()))
        })
        .await?;
        if response.status().is_redirection() {
            return Err(DomainError::PermissionDenied);
        }
        if response.headers().len() > 64
            || response
                .headers()
                .iter()
                .map(|(k, v)| k.as_str().len() + v.as_bytes().len())
                .sum::<usize>()
                > 16 * 1024
        {
            return Err(DomainError::LimitExceeded(
                "MCP OAuth response headers".into(),
            ));
        }
        let mut encoding = response.headers().get_all("content-encoding").iter();
        if encoding.next().is_some_and(|v| v != "identity") || encoding.next().is_some() {
            return Err(invalid("compressed OAuth response unsupported"));
        }
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        if headers_only {
            return Ok(Reply {
                status,
                headers,
                body: SecretBytes::new(Vec::new()),
            });
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_DOCUMENT_BYTES as u64)
        {
            return Err(DomainError::LimitExceeded(
                "MCP OAuth response bytes".into(),
            ));
        }
        // Fixed-capacity native custody: no growing allocation of a token response.
        let mut bytes = Buffer(vec![0; MAX_DOCUMENT_BYTES]);
        let mut used = 0;
        let result = async {
            while let Some(chunk) = within(deadline, cancel, async {
                response
                    .chunk()
                    .await
                    .map_err(|_| invalid("OAuth response body"))
            })
            .await?
            {
                if chunk.len() > MAX_DOCUMENT_BYTES - used {
                    return Err(DomainError::LimitExceeded(
                        "MCP OAuth response bytes".into(),
                    ));
                }
                bytes.0[used..used + chunk.len()].copy_from_slice(&chunk);
                used += chunk.len();
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            drop(bytes);
            return Err(error);
        }
        bytes.0.truncate(used);
        Ok(Reply {
            status,
            headers,
            body: SecretBytes::new(std::mem::take(&mut bytes.0)),
        })
    }
}
pub(super) fn check(deadline: Instant, cancel: &CancelToken) -> Result<(), DomainError> {
    if cancel.cancelled() {
        Err(DomainError::Cancelled)
    } else if Instant::now() >= deadline {
        Err(DomainError::TimedOut)
    } else {
        Ok(())
    }
}
pub(super) async fn within<T>(
    deadline: Instant,
    cancel: &mut CancelToken,
    work: impl Future<Output = Result<T, DomainError>>,
) -> Result<T, DomainError> {
    check(deadline, cancel)?;
    tokio::select! {biased; _=cancel.wait()=>Err(DomainError::Cancelled), _=tokio::time::sleep_until(deadline)=>Err(DomainError::TimedOut), result=work=>result}
}

struct Buffer(Vec<u8>);
impl Drop for Buffer {
    fn drop(&mut self) {
        drop(SecretBytes::new(std::mem::take(&mut self.0)));
    }
}
