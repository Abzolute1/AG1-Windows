//! Native Streamable HTTP with manager-admitted tool calls and optional exact
//! MCP credential leases. No ambient credentials, redirects, retries or recovery.
use super::{
    Revision,
    auth::{CredentialLease, Material, Target},
    catalog::{CatalogBinding, CatalogSlot, ToolsCatalog},
    discovery::{
        CatalogKind, DiscoveryCatalog, DiscoveryLimits, Health, client_info, cursor_params,
        validate_entry, validate_server,
    },
    invalid, sse,
    wire::{Limits, Message, MessageKind, decode_message},
};
use crate::{
    provider::http::{CancelToken, cancellation},
    web_http::{SystemResolver, WebPolicy, WebResolver},
};
use ag1_core::{DomainError, SecretBytes};
use reqwest::{
    Client, Method, Response, Url,
    header::{HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    future::Future,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;

static OPERATIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);
const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Native configuration, never registry metadata or model arguments. Constructing
/// an endpoint performs no DNS or network I/O. Actual activation consent belongs
/// to the native manager before it calls negotiation/listing.
pub struct Endpoint {
    url: Url,
    loopback: Option<IpAddr>,
}
impl Endpoint {
    pub fn public_https(value: &str) -> Result<Self, DomainError> {
        let url = parse_endpoint(value)?;
        if url.scheme() != "https" {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            url,
            loopback: None,
        })
    }
    /// Explicit native development exception: literal loopback only, pinned to
    /// this exact URL/port. It cannot authorize localhost DNS or private LANs.
    pub fn loopback_development(value: &str) -> Result<Self, DomainError> {
        let url = parse_endpoint(value)?;
        let ip = hostname(&url)
            .parse::<IpAddr>()
            .map_err(|_| DomainError::PermissionDenied)?;
        if !ip.is_loopback() {
            return Err(DomainError::PermissionDenied);
        }
        Ok(Self {
            url,
            loopback: Some(ip),
        })
    }
}
fn parse_endpoint(value: &str) -> Result<Url, DomainError> {
    if value.len() > 8192
        || value
            .bytes()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'\\')
    {
        return Err(invalid("HTTP endpoint"));
    }
    let url = Url::parse(value).map_err(|_| invalid("HTTP endpoint"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
        || url.port_or_known_default() == Some(0)
    {
        return Err(invalid(
            "HTTP endpoint requires a credential-free URL without query or fragment",
        ));
    }
    Ok(url)
}
fn hostname(url: &Url) -> &str {
    url.host_str()
        .unwrap_or_default()
        .trim_start_matches('[')
        .trim_end_matches(']')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shutdown {
    LocalOnly,
    RemoteSessionDeleted,
    RemoteDeletionUnsupported,
}

/// No Debug/Serialize/Clone: legacy session identifiers stay private to this
/// native instance, endpoint and binding. Dropping an operation closes its HTTP
/// stream and releases its permit; it never starts asynchronous retry work.
pub struct HttpDiscovery<R = SystemResolver> {
    endpoint: Endpoint,
    resolver: R,
    binding: CatalogBinding,
    tools: CatalogSlot,
    limits: DiscoveryLimits,
    health: Health,
    server: Option<Value>,
    session: Option<HeaderValue>,
    certificates: Vec<reqwest::Certificate>,
    next_id: u64,
    in_flight: Option<String>,
    changes: [u64; 3],
    credential: Option<Arc<CredentialLease>>,
}
struct Operation<'a> {
    input_forms: bool,
    deadline: Instant,
    bytes: usize,
    messages: usize,
    send: Option<&'a (dyn Fn() -> Result<(), DomainError> + Sync)>,
}

impl HttpDiscovery<SystemResolver> {
    pub fn new(
        endpoint: Endpoint,
        binding: CatalogBinding,
        limits: DiscoveryLimits,
    ) -> Result<Self, DomainError> {
        Self::with_resolver(endpoint, binding, limits, SystemResolver)
    }
}
impl<R: WebResolver> HttpDiscovery<R> {
    pub fn with_resolver(
        endpoint: Endpoint,
        binding: CatalogBinding,
        limits: DiscoveryLimits,
        resolver: R,
    ) -> Result<Self, DomainError> {
        let tools = CatalogSlot::new(binding.clone())?;
        #[allow(unused_mut)]
        let mut certificates = vec![];
        #[cfg(feature = "mcp-fixtures")]
        if let Some(peer) = super::fixtures::TlsPeer::get().filter(|p| p.matches(&endpoint.url)) {
            certificates.push(
                reqwest::Certificate::from_pem(peer.certificate())
                    .map_err(|_| invalid("fixture TLS root"))?,
            );
        }
        Ok(Self {
            endpoint,
            resolver,
            binding,
            tools,
            limits,
            health: Health::Attached,
            server: None,
            session: None,
            certificates,
            next_id: 1,
            in_flight: None,
            changes: [0; 3],
            credential: None,
        })
    }
    /// Native vault authority only. Neither server JSON nor arbitrary HTTP
    /// headers can install credentials. Exact URL/profile/generation must match.
    pub fn with_credential(
        mut self,
        credential: Arc<CredentialLease>,
    ) -> Result<Self, DomainError> {
        let b = credential.binding();
        let endpoint = match &b.target {
            Target::ManualBearer {
                endpoint,
                loopback_development,
            } if *loopback_development == self.endpoint.loopback.is_some() => endpoint,
            Target::OAuth { endpoint, .. } if self.endpoint.loopback.is_none() => endpoint,
            _ => return Err(DomainError::PermissionDenied),
        };
        if self.health != Health::Attached
            || self.credential.is_some()
            || b.server_id != self.binding.server_id
            || b.profile_id != self.binding.auth_profile
            || b.configuration_revision != self.binding.config_revision
            || credential.generation() != self.binding.credential_generation
            || parse_endpoint(endpoint)? != self.endpoint.url
        {
            return Err(DomainError::PermissionDenied);
        }
        credential.check(credential_now()?)?;
        self.credential = Some(credential);
        Ok(self)
    }
    fn credential_check(&self) -> Result<(), DomainError> {
        match &self.credential {
            Some(c) => c.check(credential_now()?),
            None if self.binding.credential_generation == 0 => Ok(()),
            None => Err(DomainError::PermissionDenied),
        }
    }
    fn authorization(&self) -> Result<Option<HeaderValue>, DomainError> {
        self.credential_check()?;
        self.credential
            .as_ref()
            .map(|c| {
                c.with_material(credential_now()?, |material| {
                    let Material::Bearer { access, .. } = material else {
                        return Err(DomainError::PermissionDenied);
                    };
                    let mut bytes = Vec::with_capacity(7 + access.expose().len());
                    bytes.extend_from_slice(b"Bearer ");
                    bytes.extend_from_slice(access.expose());
                    let raw = SecretBytes::new(bytes);
                    let mut header = HeaderValue::from_bytes(raw.expose())
                        .map_err(|_| invalid("authorization header"))?;
                    header.set_sensitive(true);
                    Ok(header)
                })
            })
            .transpose()
    }
    /// Explicit native trust roots, as with AG1 web fixtures. Chain and hostname
    /// verification remain enabled. This does not install system certificates.
    pub fn with_certificates(mut self, roots: &[Vec<u8>]) -> Result<Self, DomainError> {
        if self.health != Health::Attached
            || roots.len() > 4
            || roots.iter().any(|r| r.len() > 65536)
        {
            return Err(invalid("HTTP trust roots"));
        }
        self.certificates = roots
            .iter()
            .map(|r| reqwest::Certificate::from_pem(r).map_err(|_| invalid("HTTP trust root")))
            .collect::<Result<_, _>>()?;
        Ok(self)
    }
    pub(crate) fn discovery_generation(&self) -> [u64; 3] {
        self.changes
    }
    pub fn health(&self) -> Health {
        self.health
    }
    pub fn server_metadata(&self) -> Option<&Value> {
        self.server.as_ref()
    }
    pub fn tools(&self) -> Option<&ToolsCatalog> {
        self.tools.current()
    }
    fn operation<'a>(&self) -> Operation<'a> {
        Operation {
            input_forms: false,            deadline: Instant::now() + self.limits.timeout(),
            bytes: 0,
            messages: 0,
            send: None,
        }
    }
    async fn retire<T>(&mut self, result: Result<T, DomainError>) -> Result<T, DomainError> {
        if matches!(&result, Err(DomainError::Cancelled | DomainError::TimedOut))
            && self.binding.protocol == Revision::Legacy20251125
            && let Some(id) = self.in_flight.take()
        {
            // Legacy peers require an explicit cancellation notification. Only
            // requests with received HTTP headers reach this state; do not open
            // a cleanup connection when cancellation occurred before contact.
            let (_source, mut cancel) = cancellation();
            let mut cleanup = Operation {
                input_forms: false,                deadline: Instant::now() + Duration::from_millis(100),
                bytes: 0,
                messages: 0,
                send: None,
            };
            let _ = self.control(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}), &mut cleanup, &mut cancel).await;
        }
        self.in_flight = None;
        if result.is_err() {
            self.tools.invalidate()?;
            self.server = None;
            self.session = None;
            self.health = Health::Failed;
        } else {
            self.health = Health::Ready;
        }
        result
    }
    fn require(&self, name: &str) -> Result<(), DomainError> {
        if self.health != Health::Ready
            || !self
                .server
                .as_ref()
                .is_some_and(|s| s["capabilities"][name].is_object())
        {
            return Err(DomainError::Unavailable(
                "MCP discovery capability unavailable".into(),
            ));
        }
        Ok(())
    }
    pub async fn negotiate(&mut self, cancel: &mut CancelToken) -> Result<(), DomainError> {
        if self.health != Health::Attached {
            return Err(invalid("HTTP connection already negotiated or retired"));
        }
        let _permit = OPERATIONS
            .try_acquire()
            .map_err(|_| DomainError::QueueFull)?;
        // A caller dropping this future must leave no usable negotiated state.
        self.health = Health::Failed;
        let mut operation = self.operation();
        let result = self.negotiate_inner(&mut operation, cancel).await;
        self.retire(result).await
    }
    async fn negotiate_inner(
        &mut self,
        operation: &mut Operation<'_>,
        cancel: &mut CancelToken,
    ) -> Result<(), DomainError> {
        let (method, params) = match self.binding.protocol {
            Revision::Legacy20251125 => (
                "initialize",
                json!({"protocolVersion":"2025-11-25","clientInfo":client_info(),"capabilities":{}}),
            ),
            Revision::Current20260728 => ("server/discover", json!({})),
        };
        let result = self.request(method, params, operation, cancel).await?;
        validate_server(self.binding.protocol, &result)?;
        if self.binding.protocol == Revision::Legacy20251125 {
            self.control(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                operation,
                cancel,
            )
            .await?;
        }
        self.server = Some(result);
        Ok(())
    }
    /// Native manager only; no retries and no credential/profile fallback.
    pub(crate) async fn call_selected(
        &mut self,
        identity: &ag1_core::tools::mcp::Identity,
        args: &Value,
        cancel: &mut CancelToken,
        send: &(dyn Fn() -> Result<(), DomainError> + Sync),
        continue_send: &(dyn Fn() -> Result<(), DomainError> + Sync),
        input: Option<&(dyn Fn(&Value,usize)->Result<Value,DomainError> + Sync)>,
    ) -> Result<Value, DomainError> {
        let params = if identity.kind == ag1_core::tools::mcp::Kind::Tool {
            self.refresh_tools(cancel).await?;
            super::operation::check_tool(self.tools(), identity, args)?;
            identity.protocol_params(args)?
        } else {
            let catalog = self
                .list(super::operation::discovery_kind(identity.kind)?, cancel)
                .await?;
            super::operation::check_capability(&catalog, identity, args)?
        };
        let _permit = OPERATIONS
            .try_acquire()
            .map_err(|_| DomainError::QueueFull)?;
        let mut operation = self.operation();
        operation.send = Some(send);
        operation.input_forms=input.is_some() && self.binding.protocol==Revision::Current20260728;
        if operation.input_forms {operation.deadline=Instant::now()+Duration::from_secs(super::input::MAX_SECONDS);}
        let result=async {
            let mut params=params;
            let mut round=0;
            loop {
                let response=self.request(identity.kind.method(),params,&mut operation,cancel).await?;
                if response["result"]["resultType"]!="input_required" || !operation.input_forms {return Ok(response);}
                round+=1;
                if round>super::input::MAX_ROUNDS {return Err(DomainError::LimitExceeded("MCP input rounds".into()));}
                let fields=input.ok_or(DomainError::PermissionDenied)?(&response,round)?;
                if cancel.cancelled(){return Err(DomainError::Cancelled);}
                if Instant::now()>=operation.deadline{return Err(DomainError::TimedOut);}
                params=if identity.kind==ag1_core::tools::mcp::Kind::Tool {
                    self.refresh_tools(cancel).await?;super::operation::check_tool(self.tools(),identity,args)?;identity.protocol_params(args)?
                }else{let catalog=self.list(super::operation::discovery_kind(identity.kind)?,cancel).await?;super::operation::check_capability(&catalog,identity,args)?};
                for (key,value) in fields.as_object().ok_or(DomainError::PermissionDenied)? {params[key]=value.clone();}
                operation.send=Some(continue_send);
            }
        }.await;
        self.retire(result).await
    }

    pub async fn refresh_tools(&mut self, cancel: &mut CancelToken) -> Result<(), DomainError> {
        self.require("tools")?;
        let _permit = OPERATIONS
            .try_acquire()
            .map_err(|_| DomainError::QueueFull)?;
        self.health = Health::Failed;
        let mut operation = self.operation();
        let result = self.refresh_inner(&mut operation, cancel).await;
        self.retire(result).await
    }
    async fn refresh_inner(
        &mut self,
        operation: &mut Operation<'_>,
        cancel: &mut CancelToken,
    ) -> Result<(), DomainError> {
        let generation = self.changes[0];
        let mut refresh = self.tools.begin_refresh()?;
        let mut cursor = None;
        loop {
            let result = self
                .request("tools/list", cursor_params(&cursor), operation, cancel)
                .await?;
            cursor = refresh.push_page(
                &serde_json::to_vec(&result).map_err(|_| invalid("catalog encoding"))?,
            )?;
            if self.changes[0] != generation {
                return Err(DomainError::Conflict);
            }
            if cursor.is_none() {
                return self.tools.publish(refresh);
            }
        }
    }
    pub async fn list(
        &mut self,
        kind: CatalogKind,
        cancel: &mut CancelToken,
    ) -> Result<DiscoveryCatalog, DomainError> {
        self.require(kind.capability())?;
        let _permit = OPERATIONS
            .try_acquire()
            .map_err(|_| DomainError::QueueFull)?;
        self.health = Health::Failed;
        // Previously published metadata is not a live lease while another
        // operation can be cancelled or report an asynchronous capability change.
        self.tools.invalidate()?;
        let mut operation = self.operation();
        let result = self.list_inner(kind, &mut operation, cancel).await;
        self.retire(result).await
    }
    async fn list_inner(
        &mut self,
        kind: CatalogKind,
        operation: &mut Operation<'_>,
        cancel: &mut CancelToken,
    ) -> Result<DiscoveryCatalog, DomainError> {
        let index = if kind == CatalogKind::Prompts { 2 } else { 1 };
        let generation = self.changes[index];
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut entries = vec![];
        for _ in 0..32 {
            let mut result = self
                .request(kind.method(), cursor_params(&cursor), operation, cancel)
                .await?;
            let rows = result
                .get_mut(kind.key())
                .and_then(Value::as_array_mut)
                .ok_or_else(|| invalid("discovery list shape"))?;
            if rows.len() > 1024usize.saturating_sub(entries.len()) {
                return Err(DomainError::LimitExceeded("MCP catalog entries".into()));
            }
            for row in std::mem::take(rows) {
                validate_entry(kind, &row)?;
                let key = match kind {
                    CatalogKind::Resources => "uri",
                    CatalogKind::ResourceTemplates => "uriTemplate",
                    CatalogKind::Prompts => "name",
                };
                if !names.insert(row[key].as_str().unwrap().to_owned()) {
                    return Err(invalid("duplicate discovery entry"));
                }
                entries.push(row);
            }
            if self.changes[index] != generation {
                return Err(DomainError::Conflict);
            }
            cursor = match result.get("nextCursor") {
                None => {
                    return Ok(DiscoveryCatalog {
                        binding: self.binding.clone(),
                        kind,
                        entries,
                    });
                }
                Some(Value::String(s)) if !s.is_empty() && s.len() <= 1024 => Some(s.clone()),
                _ => return Err(invalid("discovery cursor")),
            };
            if !cursors.insert(cursor.clone().unwrap()) {
                return Err(invalid("repeated discovery cursor"));
            }
        }
        Err(DomainError::LimitExceeded("MCP catalog pages".into()))
    }

    pub async fn shutdown(&mut self, cancel: &mut CancelToken) -> Result<Shutdown, DomainError> {
        self.tools.invalidate()?;
        self.server = None;
        self.health = Health::Stopped;
        self.in_flight = None;
        // Take before awaiting: even cancellation/drop cannot resurrect a session.
        let Some(session) = self.session.take() else {
            return Ok(Shutdown::LocalOnly);
        };
        let _permit = OPERATIONS
            .try_acquire()
            .map_err(|_| DomainError::QueueFull)?;
        let mut operation = self.operation();
        let response = self
            .send(
                Method::DELETE,
                None,
                vec![],
                Some(&session),
                &mut operation,
                cancel,
            )
            .await?;
        match response.status().as_u16() {
            200 | 202 | 204 => Ok(Shutdown::RemoteSessionDeleted),
            405 => Ok(Shutdown::RemoteDeletionUnsupported),
            _ => Err(DomainError::Unavailable(
                "MCP local disconnect complete; remote session deletion unconfirmed".into(),
            )),
        }
    }

    async fn request(
        &mut self,
        method: &str,
        mut params: Value,
        operation: &mut Operation<'_>,
        cancel: &mut CancelToken,
    ) -> Result<Value, DomainError> {
        check(operation.deadline, cancel)?;
        let id = format!("ag1-mcp-{}", self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(DomainError::Conflict)?;
        if self.binding.protocol == Revision::Current20260728 {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":client_info(),"io.modelcontextprotocol/clientCapabilities":if operation.input_forms {json!({"elicitation":{"form":{}}})}else{json!({})}});
        }
        let body =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
                .map_err(|_| invalid("request encoding"))?;
        let mut response = self
            .send(
                Method::POST,
                Some(method),
                body,
                self.session.as_ref(),
                operation,
                cancel,
            )
            .await?;
        self.in_flight = Some(id.clone());
        status(&response)?;
        let headers = response.headers();
        if let Some(value) = single(headers, "mcp-session-id")? {
            if self.binding.protocol != Revision::Legacy20251125
                || value.as_bytes().is_empty()
                || value.as_bytes().len() > 1024
                || value.as_bytes().iter().any(|b| !(0x21..=0x7e).contains(b))
            {
                return Err(invalid("unexpected or malformed MCP session header"));
            }
            if method == "initialize" {
                let mut value = value.clone();
                value.set_sensitive(true);
                self.session = Some(value);
            } else if self.session.as_ref() != Some(value) {
                return Err(invalid("MCP session substitution"));
            }
        }
        let content_type = single(headers, "content-type")?
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| invalid("HTTP content type"))?;
        let mime = content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let max = if mime == "application/json" {
            1024 * 1024
        } else if mime == "text/event-stream" {
            MAX_BYTES
        } else {
            return Err(invalid("unsupported HTTP content type"));
        };
        if response.content_length().is_some_and(|n| n > max as u64) {
            return Err(DomainError::LimitExceeded("MCP HTTP response bytes".into()));
        }
        if mime == "application/json" {
            let mut bytes = vec![];
            while let Some(chunk) = next_chunk(&mut response, operation, cancel).await? {
                if chunk.len() > max.saturating_sub(bytes.len()) {
                    return Err(DomainError::LimitExceeded("MCP JSON response".into()));
                }
                bytes.extend_from_slice(&chunk);
            }
            let result = response_result(
                decode_message(&bytes, Limits::default())?,
                &id,
                super::operation::is_execution_method(method),
            );
            self.in_flight = None;
            return result;
        }
        let mut decoder = sse::Decoder::default();
        while let Some(chunk) = next_chunk(&mut response, operation, cancel).await? {
            let messages = decoder.push(&chunk)?;
            let mut result = None;
            for message in messages {
                operation.messages += 1;
                if operation.messages > 128 {
                    return Err(DomainError::LimitExceeded(
                        "MCP HTTP operation messages".into(),
                    ));
                }
                if result.is_some() {
                    return Err(invalid("message after final SSE response"));
                }
                match message.kind {
                    MessageKind::Response | MessageKind::Error => {
                        result = Some(response_result(
                            message,
                            &id,
                            super::operation::is_execution_method(method),
                        )?)
                    }
                    MessageKind::Notification => self.notification(&message.body)?,
                    MessageKind::Request => {
                        if self.binding.protocol == Revision::Current20260728 {
                            return Err(invalid("server requests forbidden in current revision"));
                        }
                        let reply = if message.body["method"] == "ping" {
                            json!({"jsonrpc":"2.0","id":message.body["id"],"result":{}})
                        } else {
                            json!({"jsonrpc":"2.0","id":message.body["id"],"error":{"code":-32601,"message":"Client capability not supported"}})
                        };
                        self.control(reply, operation, cancel).await?;
                    }
                }
            }
            if let Some(result) = result {
                self.in_flight = None;
                return Ok(result);
            } // drop stream at the terminal response
        }
        Err(DomainError::Unavailable(
            "MCP stream ended before response; resumption is unsupported".into(),
        ))
    }
    fn notification(&mut self, value: &Value) -> Result<(), DomainError> {
        let index = match value["method"].as_str() {
            Some("notifications/tools/list_changed") => {
                self.tools.invalidate()?;
                0
            }
            Some("notifications/resources/list_changed") => 1,
            Some("notifications/prompts/list_changed") => 2,
            _ => return Ok(()),
        };
        self.changes[index] = self.changes[index]
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        Ok(())
    }
    async fn control(
        &self,
        value: Value,
        operation: &mut Operation<'_>,
        cancel: &mut CancelToken,
    ) -> Result<(), DomainError> {
        let body = serde_json::to_vec(&value).map_err(|_| invalid("control encoding"))?;
        let mut response = self
            .send(
                Method::POST,
                value["method"].as_str(),
                body,
                self.session.as_ref(),
                operation,
                cancel,
            )
            .await?;
        if response.status().as_u16() != 202 {
            return Err(invalid("HTTP notification/response acknowledgement"));
        }
        while let Some(chunk) = next_chunk(&mut response, operation, cancel).await? {
            if !chunk.is_empty() {
                return Err(invalid("HTTP acknowledgement must be empty"));
            }
        }
        Ok(())
    }
    async fn send(
        &self,
        method: Method,
        rpc_method: Option<&str>,
        body: Vec<u8>,
        session: Option<&HeaderValue>,
        operation: &mut Operation<'_>,
        cancel: &mut CancelToken,
    ) -> Result<Response, DomainError> {
        let tool_call = rpc_method.is_some_and(super::operation::is_execution_method);
        if body.len() > if tool_call { 272 * 1024 } else { 4096 } {
            return Err(DomainError::LimitExceeded("MCP discovery request".into()));
        }
        check(operation.deadline, cancel)?;
        self.credential_check()?;
        let url = &self.endpoint.url;
        let host = hostname(url);
        let port = url
            .port_or_known_default()
            .ok_or_else(|| invalid("HTTP endpoint port"))?;
        let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
            vec![ip]
        } else {
            within(
                operation
                    .deadline
                    .min(Instant::now() + Duration::from_secs(3)),
                cancel,
                self.resolver.resolve(host, port),
            )
            .await?
        };
        if let Some(ip) = self.endpoint.loopback {
            if addresses != vec![ip] {
                return Err(DomainError::PermissionDenied);
            }
        } else {
            #[cfg(feature = "mcp-fixtures")]
            let owned = super::fixtures::TlsPeer::get()
                .is_some_and(|p| p.matches(url) && p.addresses(&addresses));
            #[cfg(not(feature = "mcp-fixtures"))]
            let owned = false;
            if !owned {
                WebPolicy::default().validate(url.as_str(), &addresses)?;
            }
        }
        let sockets: Vec<_> = addresses
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
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
            .resolve_to_addrs(host, &sockets);
        for cert in &self.certificates {
            builder = builder.add_root_certificate(cert.clone());
        }
        let client = builder
            .build()
            .map_err(|_| invalid("HTTP/TLS client initialization"))?;
        let mut request = client
            .request(method, url.clone())
            .header("accept", "application/json, text/event-stream")
            .header("content-type", "application/json")
            .header("mcp-protocol-version", self.binding.protocol.as_str());
        if self.binding.protocol == Revision::Current20260728
            && let Some(method) = rpc_method
        {
            request = request.header("mcp-method", method);
        }
        if let Some(value) = session {
            request = request.header("mcp-session-id", value.clone());
        }
        if let Some(authorization) = self.authorization()? {
            request = request.header(reqwest::header::AUTHORIZATION, authorization);
        }
        check(operation.deadline, cancel)?;
        if tool_call {
            operation.send.ok_or(DomainError::PermissionDenied)?()?;
        }
        let response = within(operation.deadline, cancel, async {
            self.credential_check()?;
            request
                .body(body)
                .send()
                .await
                .map_err(|_| DomainError::Unavailable("MCP HTTP/TLS connection failed".into()))
        })
        .await?;
        let header_bytes: usize = response
            .headers()
            .iter()
            .map(|(k, v)| k.as_str().len() + v.as_bytes().len())
            .sum();
        if response.headers().len() > 64 || header_bytes > 16 * 1024 {
            return Err(DomainError::LimitExceeded("MCP HTTP headers".into()));
        }
        if single(response.headers(), "content-encoding")?.is_some_and(|v| v != "identity") {
            return Err(invalid("compressed MCP responses unsupported"));
        }
        if response.status().is_redirection() {
            return Err(DomainError::PermissionDenied);
        }
        Ok(response)
    }
}

pub(crate) fn credential_now() -> Result<u64, DomainError> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| DomainError::Unavailable("MCP credential clock".into()))?
            .as_millis(),
    )
    .map_err(|_| DomainError::Unavailable("MCP credential clock".into()))
}

fn response_result(message: Message, id: &str, tool_call: bool) -> Result<Value, DomainError> {
    if !matches!(message.kind, MessageKind::Response | MessageKind::Error)
        || message.body["id"] != id
    {
        return Err(invalid("unexpected HTTP response ID or message kind"));
    }
    if tool_call {
        return Ok(message.body);
    }
    if message.kind == MessageKind::Error {
        return Err(DomainError::Unavailable(
            "MCP server rejected discovery request".into(),
        ));
    }
    let result = message
        .body
        .get("result")
        .filter(|v| v.is_object())
        .ok_or_else(|| invalid("discovery result object"))?;
    if result.get("resultType").is_some_and(|v| v != "complete") {
        return Err(DomainError::Unavailable(
            "MCP discovery continuation unsupported".into(),
        ));
    }
    Ok(result.clone())
}
fn single<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a HeaderValue>, DomainError> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(invalid("duplicate HTTP protocol header"));
    }
    Ok(first)
}
fn status(response: &Response) -> Result<(), DomainError> {
    match response.status().as_u16() {
        200 => Ok(()),
        401 | 403 => Err(DomainError::Unavailable(
            "MCP authentication required or access denied".into(),
        )),
        404 => Err(DomainError::Unavailable(
            "MCP endpoint/method/session unavailable; reconnect requires review".into(),
        )),
        429 => Err(DomainError::Unavailable(
            "MCP service rate limited; retry later".into(),
        )),
        _ => Err(DomainError::Unavailable(
            "MCP unexpected HTTP status".into(),
        )),
    }
}
fn check(deadline: Instant, cancel: &CancelToken) -> Result<(), DomainError> {
    if cancel.cancelled() {
        return Err(DomainError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(DomainError::TimedOut);
    }
    Ok(())
}
async fn within<T>(
    deadline: Instant,
    cancel: &mut CancelToken,
    work: impl Future<Output = Result<T, DomainError>>,
) -> Result<T, DomainError> {
    check(deadline, cancel)?;
    tokio::select! { biased;
        _=cancel.wait()=>Err(DomainError::Cancelled),
        _=tokio::time::sleep_until(deadline)=>Err(DomainError::TimedOut),
        result=work=>result,
    }
}
async fn next_chunk(
    response: &mut Response,
    operation: &mut Operation<'_>,
    cancel: &mut CancelToken,
) -> Result<Option<Vec<u8>>, DomainError> {
    let chunk = within(
        operation
            .deadline
            .min(Instant::now() + Duration::from_secs(5)),
        cancel,
        async {
            response
                .chunk()
                .await
                .map_err(|_| DomainError::Unavailable("MCP HTTP body interrupted".into()))
        },
    )
    .await?;
    if let Some(chunk) = chunk {
        if chunk.len() > MAX_BYTES.saturating_sub(operation.bytes) {
            return Err(DomainError::LimitExceeded(
                "MCP operation response bytes".into(),
            ));
        }
        operation.bytes += chunk.len();
        Ok(Some(chunk.to_vec()))
    } else {
        Ok(None)
    }
}
