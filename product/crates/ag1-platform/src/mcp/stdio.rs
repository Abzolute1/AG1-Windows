//! Bounded discovery and manager-admitted tool calls over AG1's existing owned process capability.
//!
//! Run on an owned worker, never the UI/session owner. Attaching consumes an
//! already-authorized process; this module cannot launch, resolve credentials,
//! independently authorize tools, read resources or obtain prompts. Production composition still
//! requires the MCP launch/scope manager. Do not use the SDK convenience launcher.
use super::{
    Revision,
    catalog::{CatalogBinding, CatalogSlot, ToolsCatalog},
    invalid,
    wire::{Limits, Message, MessageKind, decode_message},
};
use crate::process::OwnedProcess;
use ag1_core::DomainError;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const FRAME_BYTES: usize = 1024 * 1024;
const OP_BYTES: usize = 8 * 1024 * 1024;
const OP_MESSAGES: usize = 128;
const CATALOG_PAGES: usize = 32;
const CATALOG_ENTRIES: usize = 1024;

pub use super::discovery::{CatalogKind, DiscoveryCatalog, DiscoveryLimits, Health};
use super::discovery::{client_info, cursor_params, validate_entry, validate_server};

/// One serial connection, one pending request, one bounded frame accumulator.
/// No Clone: ownership and shutdown stay with this native connection.
pub struct StdioDiscovery {
    process: OwnedProcess,
    binding: CatalogBinding,
    tools: CatalogSlot,
    health: Health,
    limits: DiscoveryLimits,
    next_id: u64,
    in_flight: Option<String>,
    server: Option<Value>,
    fragment: Vec<u8>,
    read_ahead: Vec<u8>,
    read_offset: usize,
    tool_changes: u64,
    resource_changes: u64,
    prompt_changes: u64,
}

struct Operation<'a> {
    input_forms: bool,
    deadline: Instant,
    cancel: &'a AtomicBool,
    bytes: usize,
    messages: usize,
    send: Option<&'a (dyn Fn() -> Result<(), DomainError> + Sync)>,
}
impl Operation<'_> {
    fn check(&self) -> Result<(), DomainError> {
        if self.cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(DomainError::TimedOut);
        }
        Ok(())
    }
}

impl StdioDiscovery {
    /// `process` must have been explicitly admitted by the native owner, with
    /// bounded lifetime and safe stderr capture (e.g. spawn_mcp_protocol).
    /// Server metadata cannot construct an OwnedProcess or set this binding.
    pub fn from_owned(
        process: OwnedProcess,
        binding: CatalogBinding,
        limits: DiscoveryLimits,
    ) -> Result<Self, DomainError> {
        let tools = CatalogSlot::new(binding.clone())?;
        Ok(Self {
            process,
            binding,
            tools,
            health: Health::Attached,
            limits,
            next_id: 1,
            in_flight: None,
            server: None,
            fragment: Vec::new(),
            read_ahead: Vec::new(),
            read_offset: 0,
            tool_changes: 0,
            resource_changes: 0,
            prompt_changes: 0,
        })
    }
    /// Native worker health probe; no protocol requests or automatic reconnect.
    pub fn is_running(&mut self) -> Result<bool, DomainError> {
        if self.process.try_wait()?.is_some() {
            self.health = Health::Stopped;
            self.tools.invalidate()?;
            self.server = None;
            return Ok(false);
        }
        Ok(true)
    }
    pub(crate) fn discovery_generation(&self) -> [u64; 3] {
        [
            self.tool_changes,
            self.resource_changes,
            self.prompt_changes,
        ]
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

    fn operation<'a>(&self, cancel: &'a AtomicBool) -> Operation<'a> {
        Operation {
            input_forms: false,
            deadline: Instant::now() + self.limits.timeout(),
            cancel,
            // Charge buffered bytes to the new operation too; read-ahead from
            // the previous response must not evade the aggregate ceiling.
            bytes: self.read_ahead.len().saturating_sub(self.read_offset) + self.fragment.len(),
            messages: 0,
            send: None,
        }
    }
    fn settle<T>(&mut self, result: Result<T, DomainError>) -> Result<T, DomainError> {
        if result.is_err() {
            if matches!(&result, Err(DomainError::Cancelled | DomainError::TimedOut))
                && let Some(id) = self.in_flight.take()
            {
                let cancel = AtomicBool::new(false);
                let cleanup = Operation {
                    input_forms: false,
                    deadline: Instant::now() + Duration::from_millis(100),
                    cancel: &cancel,
                    bytes: 0,
                    messages: 0,
                    send: None,
                };
                // Cancellation is best effort. Process custody below does not
                // depend on the peer reading or honoring the notification.
                let _ = self.write(&json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":id}}), &cleanup);
            }
            self.in_flight = None;
            self.health = Health::Failed;
            let _ = self.tools.invalidate();
            self.server = None;
            self.fragment.clear();
            self.read_ahead.clear();
            // No replay or reconnect. Retain the OwnedProcess even if stopping
            // fails, so a caller can retry shutdown and Drop retains custody.
            if self.process.stop().is_err() {
                return Err(DomainError::Unavailable(
                    "MCP failed; owned process cleanup incomplete".into(),
                ));
            }
        }
        result
    }
    pub fn shutdown(&mut self) -> Result<(), DomainError> {
        self.tools.invalidate()?;
        self.server = None;
        self.fragment.clear();
        self.read_ahead.clear();
        self.process.close_input();
        match self.process.stop() {
            Ok(()) => {
                self.health = Health::Stopped;
                Ok(())
            }
            Err(error) => {
                self.health = Health::Failed;
                Err(error)
            }
        }
    }

    pub fn negotiate(&mut self, cancel: &AtomicBool) -> Result<(), DomainError> {
        if self.health != Health::Attached {
            return Err(invalid("connection already negotiated or retired"));
        }
        self.health = Health::Negotiating;
        let mut operation = self.operation(cancel);
        let result = self.negotiate_inner(&mut operation);
        self.settle(result)
    }
    fn negotiate_inner(&mut self, operation: &mut Operation<'_>) -> Result<(), DomainError> {
        let result = match self.binding.protocol {
            Revision::Legacy20251125 => self.request(
                "initialize",
                json!({
                    "protocolVersion":"2025-11-25", "capabilities":{}, "clientInfo":client_info()
                }),
                operation,
            )?,
            Revision::Current20260728 => self.request("server/discover", json!({}), operation)?,
        };
        validate_server(self.binding.protocol, &result)?;
        if self.binding.protocol == Revision::Legacy20251125 {
            self.write(
                &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                operation,
            )?;
        }
        self.server = Some(result);
        self.health = Health::Ready;
        Ok(())
    }

    /// Native manager only, after durable intent. Refresh is mandatory even when
    /// the peer omits list-change notifications. No tool call is retried.
    pub(crate) fn call_selected(
        &mut self,
        identity: &ag1_core::tools::mcp::Identity,
        args: &Value,
        cancel: &AtomicBool,
        send: &(dyn Fn() -> Result<(), DomainError> + Sync),
        continue_send: &(dyn Fn() -> Result<(), DomainError> + Sync),
        input: Option<&(dyn Fn(&Value, usize) -> Result<Value, DomainError> + Sync)>,
    ) -> Result<Value, DomainError> {
        let params = if identity.kind == ag1_core::tools::mcp::Kind::Tool {
            self.refresh_tools(cancel)?;
            super::operation::check_tool(self.tools(), identity, args)?;
            identity.protocol_params(args)?
        } else {
            let catalog = self.list(super::operation::discovery_kind(identity.kind)?, cancel)?;
            super::operation::check_capability(&catalog, identity, args)?
        };
        let mut operation = self.operation(cancel);
        operation.send = Some(send);
        operation.input_forms =
            input.is_some() && self.binding.protocol == Revision::Current20260728;
        if operation.input_forms {
            operation.deadline = Instant::now() + Duration::from_secs(super::input::MAX_SECONDS);
        }
        let result = (|| {
            let mut params = params;
            let mut round = 0;
            loop {
                let response = self.request(identity.kind.method(), params, &mut operation)?;
                if response["result"]["resultType"] != "input_required" || !operation.input_forms {
                    return Ok(response);
                }
                round += 1;
                if round > super::input::MAX_ROUNDS {
                    return Err(DomainError::LimitExceeded("MCP input rounds".into()));
                }
                let fields = input.ok_or(DomainError::PermissionDenied)?(&response, round)?;
                operation.check()?;
                params = if identity.kind == ag1_core::tools::mcp::Kind::Tool {
                    self.refresh_tools(cancel)?;
                    super::operation::check_tool(self.tools(), identity, args)?;
                    identity.protocol_params(args)?
                } else {
                    let catalog =
                        self.list(super::operation::discovery_kind(identity.kind)?, cancel)?;
                    super::operation::check_capability(&catalog, identity, args)?
                };
                for (key, value) in fields.as_object().ok_or(DomainError::PermissionDenied)? {
                    params[key] = value.clone();
                }
                operation.send = Some(continue_send);
            }
        })();
        self.settle(result)
    }

    pub fn refresh_tools(&mut self, cancel: &AtomicBool) -> Result<(), DomainError> {
        self.require_capability("tools")?;
        let mut operation = self.operation(cancel);
        let result = self.refresh_tools_inner(&mut operation);
        self.settle(result)
    }
    fn refresh_tools_inner(&mut self, operation: &mut Operation<'_>) -> Result<(), DomainError> {
        let generation = self.tool_changes;
        let mut refresh = self.tools.begin_refresh()?;
        let mut cursor = None;
        loop {
            let page = self.request("tools/list", cursor_params(&cursor), operation)?;
            let encoded =
                serde_json::to_vec(&page).map_err(|_| invalid("tools/list serialization"))?;
            cursor = refresh.push_page(&encoded)?;
            if self.tool_changes != generation {
                return Err(DomainError::Conflict);
            }
            if cursor.is_none() {
                return self.tools.publish(refresh);
            }
        }
    }

    pub fn list(
        &mut self,
        kind: CatalogKind,
        cancel: &AtomicBool,
    ) -> Result<DiscoveryCatalog, DomainError> {
        self.require_capability(kind.capability())?;
        let mut operation = self.operation(cancel);
        let result = self.list_inner(kind, &mut operation);
        self.settle(result)
    }
    fn list_inner(
        &mut self,
        kind: CatalogKind,
        operation: &mut Operation<'_>,
    ) -> Result<DiscoveryCatalog, DomainError> {
        let generation = self.catalog_generation(kind);
        let mut cursor = None;
        let mut cursors = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut entries = Vec::new();
        for _ in 0..CATALOG_PAGES {
            let mut page = self.request(kind.method(), cursor_params(&cursor), operation)?;
            let rows = page
                .get_mut(kind.key())
                .and_then(Value::as_array_mut)
                .ok_or_else(|| invalid("discovery list shape"))?;
            if rows.len() > CATALOG_ENTRIES.saturating_sub(entries.len()) {
                return Err(DomainError::LimitExceeded("MCP discovery entries".into()));
            }
            for row in std::mem::take(rows) {
                validate_entry(kind, &row)?;
                let identity = match kind {
                    CatalogKind::Resources => &row["uri"],
                    CatalogKind::ResourceTemplates => &row["uriTemplate"],
                    CatalogKind::Prompts => &row["name"],
                };
                if !names.insert(identity.as_str().unwrap().to_owned()) {
                    return Err(invalid("duplicate discovery entry"));
                }
                entries.push(row);
            }
            if self.catalog_generation(kind) != generation {
                return Err(DomainError::Conflict);
            }
            cursor = match page.get("nextCursor") {
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
        Err(DomainError::LimitExceeded("MCP discovery pages".into()))
    }
    fn catalog_generation(&self, kind: CatalogKind) -> u64 {
        match kind {
            CatalogKind::Prompts => self.prompt_changes,
            _ => self.resource_changes,
        }
    }
    fn require_capability(&self, name: &str) -> Result<(), DomainError> {
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

    fn request(
        &mut self,
        method: &str,
        mut params: Value,
        operation: &mut Operation<'_>,
    ) -> Result<Value, DomainError> {
        operation.check()?;
        let id = format!("ag1-mcp-{}", self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(DomainError::Conflict)?;
        if self.binding.protocol == Revision::Current20260728 {
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion":"2026-07-28",
                "io.modelcontextprotocol/clientInfo":client_info(),
                "io.modelcontextprotocol/clientCapabilities":if operation.input_forms {json!({"elicitation":{"form":{}}})}else{json!({})}
            });
        }
        self.write(
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            operation,
        )?;
        self.in_flight = Some(id.clone());
        loop {
            let message = self.receive(operation)?;
            match message.kind {
                MessageKind::Response | MessageKind::Error => {
                    if message.body["id"] != id {
                        return Err(invalid("unexpected or duplicate response ID"));
                    }
                    self.in_flight = None;
                    if super::operation::is_execution_method(method) {
                        return Ok(message.body);
                    }
                    if message.kind == MessageKind::Error {
                        // No server-controlled message/data in logs or diagnostic error.
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
                    return Ok(result.clone());
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
                    self.write(&reply, operation)?;
                }
            }
        }
    }
    fn notification(&mut self, body: &Value) -> Result<(), DomainError> {
        let counter = match body["method"].as_str() {
            Some("notifications/tools/list_changed") => {
                self.tools.invalidate()?;
                &mut self.tool_changes
            }
            Some("notifications/resources/list_changed") => &mut self.resource_changes,
            Some("notifications/prompts/list_changed") => &mut self.prompt_changes,
            // Unsupported progress/log/subscription content is bounded and ignored.
            _ => return Ok(()),
        };
        *counter = counter.checked_add(1).ok_or(DomainError::Conflict)?;
        Ok(())
    }
    fn write(&mut self, value: &Value, operation: &Operation<'_>) -> Result<(), DomainError> {
        let mut bytes = serde_json::to_vec(value).map_err(|_| invalid("request serialization"))?;
        let tool_call = value["method"]
            .as_str()
            .is_some_and(super::operation::is_execution_method);
        let max = if tool_call { 272 * 1024 } else { 4096 };
        if bytes.len() > max {
            return Err(DomainError::LimitExceeded("MCP discovery request".into()));
        }
        if tool_call {
            operation.send.ok_or(DomainError::PermissionDenied)?()?;
        }
        bytes.push(b'\n');
        let mut offset = 0;
        while offset < bytes.len() {
            operation.check()?;
            match self.process.try_write_input(&bytes[offset..])? {
                Some(0) => return Err(invalid("closed process input")),
                Some(count) => offset += count,
                None => thread::sleep(Duration::from_millis(2)),
            }
        }
        Ok(())
    }
    fn receive(&mut self, operation: &mut Operation<'_>) -> Result<Message, DomainError> {
        loop {
            operation.check()?;
            if self.read_offset < self.read_ahead.len() {
                let remaining = &self.read_ahead[self.read_offset..];
                let newline = remaining.iter().position(|b| *b == b'\n');
                let take = newline.unwrap_or(remaining.len());
                if take > FRAME_BYTES.saturating_sub(self.fragment.len()) {
                    return Err(DomainError::LimitExceeded("MCP frame bytes".into()));
                }
                self.fragment.extend_from_slice(&remaining[..take]);
                self.read_offset += take + usize::from(newline.is_some());
                if newline.is_some() {
                    operation.messages += 1;
                    if operation.messages > OP_MESSAGES {
                        return Err(DomainError::LimitExceeded("MCP operation messages".into()));
                    }
                    let message = decode_message(&self.fragment, Limits::default())?;
                    self.fragment.clear();
                    return Ok(message);
                }
            }
            let (chunk, status) = self.process.poll_stdout(32 * 1024)?;
            operation.bytes = operation.bytes.saturating_add(chunk.len());
            if operation.bytes > OP_BYTES {
                return Err(DomainError::LimitExceeded("MCP operation bytes".into()));
            }
            self.read_ahead = chunk;
            self.read_offset = 0;
            if self.read_ahead.is_empty() {
                if status.is_some() {
                    return Err(invalid("MCP process exited or truncated a frame"));
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
