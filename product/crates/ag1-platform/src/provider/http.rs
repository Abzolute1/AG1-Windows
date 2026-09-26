//! Host-only HTTP transport. Explicit origins, verified TLS, no redirect or
//! inherited proxy can move provider credentials to another endpoint.
use ag1_core::{DomainError, SecretBytes};
use reqwest::{
    Client, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::{collections::BTreeSet, time::Duration};
use tokio::{sync::watch, time::Instant};

/// A disabled watchdog has no timer, rather than a distant synthetic deadline.
async fn deadline_reached(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending::<()>().await,
    }
}

pub struct CancelSource {
    sender: watch::Sender<bool>,
}
pub struct CancelToken {
    receiver: watch::Receiver<bool>,
}
pub fn cancellation() -> (CancelSource, CancelToken) {
    let (sender, receiver) = watch::channel(false);
    (CancelSource { sender }, CancelToken { receiver })
}
impl CancelSource {
    pub fn cancel(&self) {
        let _ = self.sender.send(true);
    }
}
impl CancelToken {
    pub fn cancelled(&self) -> bool {
        *self.receiver.borrow() || self.receiver.has_changed().is_err()
    }
    pub(crate) async fn wait(&mut self) {
        if self.cancelled() {
            return;
        }
        while self.receiver.changed().await.is_ok() {
            if *self.receiver.borrow() {
                return;
            }
        }
    }
}
/// Deliberately no Debug/Serialize implementations on credential-bearing types.
pub struct HostHeaders {
    pub(super) headers: HeaderMap,
}
impl Default for HostHeaders {
    fn default() -> Self {
        Self {
            headers: HeaderMap::new(),
        }
    }
}
impl HostHeaders {
    pub fn insert_secret(&mut self, name: &str, value: &SecretBytes) -> Result<(), DomainError> {
        self.insert(name, value.expose(), true)
    }
    pub fn insert_public(&mut self, name: &str, value: &str) -> Result<(), DomainError> {
        self.insert(name, value.as_bytes(), false)
    }
    fn insert(&mut self, name: &str, value: &[u8], secret: bool) -> Result<(), DomainError> {
        if self.headers.len() >= 64 || value.len() > 16 * 1024 {
            return Err(DomainError::LimitExceeded("HTTP headers".into()));
        }
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| DomainError::InvalidInput("HTTP header name".into()))?;
        if ["host", "content-length", "transfer-encoding", "connection"].contains(&name.as_str()) {
            return Err(DomainError::PermissionDenied);
        }
        let mut value = HeaderValue::from_bytes(value)
            .map_err(|_| DomainError::InvalidInput("HTTP header value".into()))?;
        value.set_sensitive(secret);
        self.headers.insert(name, value);
        Ok(())
    }
}
pub struct HttpRequest {
    pub url: String,
    pub method: HttpMethod,
    pub headers: HostHeaders,
    pub body: Vec<u8>,
    pub max_response_bytes: usize,
}
#[derive(Clone, Copy)]
pub enum HttpMethod {
    Get,
    Post,
}
#[derive(Clone)]
pub struct ProviderHttp {
    client: Client,
    origins: BTreeSet<String>,
    runtime: Option<std::sync::Arc<dyn GenerationRuntime>>,
    response_observer: Option<ResponseObserver>,
    request_observer: Option<RequestObserver>,
}
/// Explicit host-only diagnostic hook. Receives bounded response body bytes before
/// provider decoding and None at actual EOF; never receives request credentials.
/// Callers own private storage, bounded buffering and any required redaction.
pub type ResponseObserver =
    std::sync::Arc<dyn Fn(Option<&[u8]>) -> Result<(), DomainError> + Send + Sync>;
/// Host-only diagnostic copy of the validated URL and outgoing body. Headers
/// and credentials are excluded; callers own private storage and byte limits.
pub type RequestObserver =
    std::sync::Arc<dyn Fn(&str, &[u8]) -> Result<(), DomainError> + Send + Sync>;
/// Host custody preparation, called with an already held generation permit.
/// It may load the pinned model but must not issue a generation itself.
pub trait GenerationRuntime: Send + Sync {
    /// Explicit host request: prepare resident weights without generating a message.
    /// The caller must hold the shared local endpoint scheduler permit.
    fn load_model<'a>(&'a self, _model: &'a str) -> ag1_core::PortFuture<'a, ()> {
        Box::pin(async { Err(DomainError::Unavailable("Manual model loading is unavailable".into())) })
    }
    /// Advisory UI phase, independent of the blocking model-loading lock.
    fn progress(&self) -> Option<serde_json::Value> {
        None
    }
    fn prepare<'a>(&'a self, request: &'a HttpRequest) -> ag1_core::PortFuture<'a, ()>;
    fn shutdown(&self) -> ag1_core::PortFuture<'_, ()>;
    /// Success must prove this request's owned runtime has stopped. A remote
    /// transport disconnect alone is not sufficient settlement evidence.
    fn cancel_generation<'a>(&'a self, _url: &'a str) -> ag1_core::PortFuture<'a, ()> {
        Box::pin(async {
            Err(DomainError::Unavailable(
                "No owned cancellation proof".into(),
            ))
        })
    }
}
pub fn endpoint_origin(value: &str) -> Result<String, DomainError> {
    let url = Url::parse(value).map_err(|_| DomainError::InvalidInput("provider URL".into()))?;
    if !["http", "https"].contains(&url.scheme())
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(DomainError::InvalidInput("provider HTTP origin".into()));
    }
    Ok(url.origin().ascii_serialization())
}
impl ProviderHttp {
    pub(super) async fn cancel_generation(&self, url: &str) -> Result<(), DomainError> {
        self.runtime
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("No owned runtime".into()))?
            .cancel_generation(url)
            .await
    }
    pub fn new(origins: &[String], connect_timeout: Duration) -> Result<Self, DomainError> {
        if origins.is_empty()
            || origins.len() > 256
            || connect_timeout.is_zero()
            || connect_timeout > Duration::from_secs(120)
        {
            return Err(DomainError::InvalidInput(
                "HTTP transport configuration".into(),
            ));
        }
        let origins = origins
            .iter()
            .map(|s| endpoint_origin(s))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let client = Client::builder()
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(connect_timeout)
            .pool_max_idle_per_host(3)
            .build()
            .map_err(|_| DomainError::Unavailable("HTTP/TLS initialization".into()))?;
        Ok(Self {
            client,
            origins,
            runtime: None,
            response_observer: None,
            request_observer: None,
        })
    }
    pub fn with_response_observer(mut self, observer: ResponseObserver) -> Self {
        self.response_observer = Some(observer);
        self
    }
    pub fn with_request_observer(mut self, observer: RequestObserver) -> Self {
        self.request_observer = Some(observer);
        self
    }
    pub fn with_runtime(mut self, runtime: std::sync::Arc<dyn GenerationRuntime>) -> Self {
        self.runtime = Some(runtime);
        self
    }
    pub(crate) async fn prepare_generation(
        &self,
        request: &HttpRequest,
        deadline: Option<Instant>,
        cancel: &mut CancelToken,
    ) -> Result<(), DomainError> {
        self.validate_request(request)?;
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        if let Some(runtime) = &self.runtime {
            tokio::select! {
                biased;
                _ = cancel.wait() => return Err(DomainError::Cancelled),
                _ = deadline_reached(deadline) => return Err(DomainError::TimedOut),
                result = runtime.prepare(request) => result?,
            }
        }
        Ok(())
    }
    /// Validate without starting a network operation.
    pub fn validate_request(&self, request: &HttpRequest) -> Result<(), DomainError> {
        if !self.origins.contains(&endpoint_origin(&request.url)?) {
            return Err(DomainError::PermissionDenied);
        }
        if request.body.len() > 32 * 1024 * 1024
            || request.max_response_bytes == 0
            || request.max_response_bytes > 64 * 1024 * 1024
        {
            return Err(DomainError::LimitExceeded(
                "provider HTTP size budget".into(),
            ));
        }
        Ok(())
    }
    pub async fn send(
        &self,
        request: HttpRequest,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<HttpResponse, DomainError> {
        self.send_until(request, Some(deadline), cancel).await
    }
    pub(crate) async fn send_until(
        &self,
        request: HttpRequest,
        deadline: Option<Instant>,
        cancel: &mut CancelToken,
    ) -> Result<HttpResponse, DomainError> {
        self.validate_request(&request)?;
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        let method = match request.method {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Post => reqwest::Method::POST,
        };
        if let Some(observer) = &self.request_observer {
            observer(&request.url, &request.body)?;
        }
        let pending = self
            .client
            .request(method, &request.url)
            .headers(request.headers.headers)
            .body(request.body)
            .send();
        let response = tokio::select! {
            biased;
            _=cancel.wait()=>return Err(DomainError::Cancelled),
            _=deadline_reached(deadline)=>return Err(DomainError::TimedOut),
            value=pending=>value.map_err(|e|if e.is_timeout(){DomainError::TimedOut}else{DomainError::Unavailable("provider HTTP connection failed".into())})?,
        };
        if response
            .content_length()
            .is_some_and(|v| v > request.max_response_bytes as u64)
        {
            return Err(DomainError::LimitExceeded(
                "provider response content length".into(),
            ));
        }
        Ok(HttpResponse {
            response,
            read: 0,
            max: request.max_response_bytes,
            ended: false,
            observer: self.response_observer.clone(),
        })
    }
}
pub struct HttpResponse {
    response: reqwest::Response,
    read: usize,
    max: usize,
    ended: bool,
    observer: Option<ResponseObserver>,
}
impl HttpResponse {
    pub fn status(&self) -> u16 {
        self.response.status().as_u16()
    }
    pub fn content_type(&self) -> Option<&str> {
        self.response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
    }
    pub fn retry_after(&self) -> Option<&str> {
        self.response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
    }
    pub fn settled(&self) -> bool {
        self.ended
    }
    /// A None result proves transport EOF; cancellation/timeouts do not.
    pub async fn chunk(
        &mut self,
        deadline: Instant,
        cancel: &mut CancelToken,
    ) -> Result<Option<Vec<u8>>, DomainError> {
        self.chunk_until(Some(deadline), cancel).await
    }
    pub(crate) async fn chunk_until(
        &mut self,
        deadline: Option<Instant>,
        cancel: &mut CancelToken,
    ) -> Result<Option<Vec<u8>>, DomainError> {
        if self.ended {
            return Ok(None);
        }
        if cancel.cancelled() {
            return Err(DomainError::Cancelled);
        }
        let result = tokio::select! {
            biased;
            _=cancel.wait()=>return Err(DomainError::Cancelled),
            _=deadline_reached(deadline)=>return Err(DomainError::TimedOut),
            value=self.response.chunk()=>value.map_err(|_|DomainError::Unavailable("provider response interrupted".into()))?,
        };
        if let Some(bytes) = result {
            if bytes.len() > self.max.saturating_sub(self.read) {
                return Err(DomainError::LimitExceeded("provider response body".into()));
            }
            self.read += bytes.len();
            if let Some(observer) = &self.observer {
                observer(Some(&bytes))?;
            }
            Ok(Some(bytes.to_vec()))
        } else {
            if let Some(observer) = &self.observer {
                observer(None)?;
            }
            self.ended = true;
            Ok(None)
        }
    }
}

/// Credential-bearing headers may be echoed by an upstream error. Keep a private
/// redaction set for that request; never derive frontend diagnostics from secrets.
pub(crate) struct ErrorRedactor {
    values: Vec<ag1_core::SecretBytes>,
}
impl ErrorRedactor {
    pub(crate) fn new(request: &HttpRequest) -> Self {
        let mut values = vec![];
        for value in request
            .headers
            .headers
            .values()
            .filter(|v| v.is_sensitive())
        {
            if let Ok(text) = value.to_str() {
                values.push(SecretBytes::new(text.as_bytes().to_vec()));
                if let Some((scheme, token)) = text.split_once(' ')
                    && ["bearer", "basic"].contains(&scheme.to_lowercase().as_str())
                {
                    values.push(SecretBytes::new(token.as_bytes().to_vec()));
                }
            }
        }
        values.sort_by_key(|v| std::cmp::Reverse(v.expose().len()));
        Self { values }
    }
    pub(crate) fn redact(&self, mut text: String) -> String {
        for secret in &self.values {
            if let Ok(value) = std::str::from_utf8(secret.expose())
                && !value.is_empty()
            {
                text = text.replace(value, "[redacted]");
            }
        }
        text
    }
}
