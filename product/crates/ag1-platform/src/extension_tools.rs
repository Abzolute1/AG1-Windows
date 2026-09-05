//! Durable tool adapter for an admitted optional host. Native registry, policy,
//! approvals and storage remain authoritative. Hook callbacks cannot request
//! parent actions; tool execution callbacks use the embedding core broker.
use crate::{
    extension_host::{Bridge, DenyBridge, ExtensionHost, NativeRequest, NativeStream},
    resource_fs::ScanControl,
};
use ag1_core::{
    DomainError,
    storage::StoragePort,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{
            self, AdapterOutcome, BeforeHook, Effect, Execution, ExecutionArtifacts, Hooks,
            Outcome, ToolEvent,
        },
        registry::{Origin, Registry, Scope, ValidatedCall},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{cell::RefCell, collections::BTreeSet};

pub struct ExtensionTools {
    host: RefCell<ExtensionHost>,
    scope: Scope,
    catalog: Value,
    revision: String,
}
pub struct PreparedExtensionTool {
    call: ValidatedCall,
    scope: Scope,
    target: String,
}
impl PreparedExtensionTool {
    pub fn call(&self) -> &ValidatedCall {
        &self.call
    }
    pub fn target_revision(&self) -> &str {
        &self.target
    }
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}
impl ExtensionTools {
    /// Bind a freshly admitted host. No tool is activated by this operation.
    pub fn bind(
        mut host: ExtensionHost,
        scope: &Scope,
        snapshot: &Value,
        control: &ScanControl<'_>,
        bridge: &mut impl Bridge,
    ) -> Result<Self, DomainError> {
        let catalog = host
            .call(
                scope,
                "bind",
                &json!({"snapshot":snapshot}),
                control,
                bridge,
            )?
            .ok_or_else(|| invalid("extension catalog missing"))?;
        let tools = catalog["tools"]
            .as_array()
            .ok_or_else(|| invalid("extension catalog tools"))?;
        if tools.len() > 256 {
            return Err(DomainError::LimitExceeded("extension tools".into()));
        }
        let revision = digest(&json!([host.identity(), catalog]));
        Ok(Self {
            host: RefCell::new(host),
            scope: scope.clone(),
            catalog,
            revision,
        })
    }
    pub fn catalog(&self) -> &Value {
        &self.catalog
    }
    pub fn register(&self, registry: &mut Registry) -> Result<(), DomainError> {
        registry.register_extensions(
            self.catalog["tools"]
                .as_array()
                .ok_or_else(|| invalid("extension catalog tools"))?,
        )
    }
    fn check_current(&self, scope: &Scope, control: &ScanControl<'_>) -> Result<(), DomainError> {
        control.check()?;
        if &self.scope != scope {
            return Err(DomainError::Conflict);
        }
        let mut host = self
            .host
            .try_borrow_mut()
            .map_err(|_| DomainError::Conflict)?;
        let catalog = host
            .call(scope, "catalog", &json!({}), control, &mut DenyBridge)?
            .ok_or_else(|| invalid("extension catalog missing"))?;
        // Error diagnostics may grow without changing definitions. Compare only the
        // registered API surface; refresh/reload requires a new native registration.
        for key in [
            "generation",
            "registrationRevision",
            "extensions",
            "tools",
            "commands",
            "flags",
            "shortcuts",
            "renderers",
            "handlers",
        ] {
            if catalog.get(key) != self.catalog.get(key) {
                return Err(DomainError::Conflict);
            }
        }
        Ok(())
    }
    fn target(&self, call: &ValidatedCall) -> String {
        digest(&json!([
            "extension-tool-v1",
            self.revision,
            call.name(),
            call.operation(),
            call.call_id(),
            call.args_hash()
        ]))
    }
    pub fn prepare(
        &self,
        registry: &Registry,
        scope: &Scope,
        invocation: executor::Invocation<'_>,
        snapshot: &Value,
        control: &ScanControl<'_>,
    ) -> Result<PreparedExtensionTool, DomainError> {
        if !invocation.complete {
            return registry
                .prepare(
                    invocation.operation,
                    invocation.call_id,
                    invocation.name,
                    invocation.arguments,
                    false,
                )
                .map(|_| unreachable!());
        }
        if !registry
            .active_names()
            .iter()
            .any(|name| name == invocation.name)
            || !self.catalog["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|t| t["name"] == invocation.name))
        {
            return Err(DomainError::Unavailable(
                "extension tool is not active".into(),
            ));
        }
        self.check_current(scope, control)?;
        let prepared=self.host.borrow_mut().call(scope,"tool.prepare",&json!({"name":invocation.name,"arguments":invocation.arguments,"snapshot":snapshot}),control,&mut DenyBridge)?.ok_or_else(||invalid("extension arguments missing"))?;
        let call = registry.prepare(
            invocation.operation,
            invocation.call_id,
            invocation.name,
            &prepared,
            true,
        )?;
        if call.origin() != Origin::Extension {
            return Err(DomainError::PermissionDenied);
        }
        let mut hooks = ToolHooks {
            tools: self,
            scope,
            snapshot,
            control,
        };
        let call = executor::prepare(registry, &call, &mut hooks)?;
        // A hook can register a new tool synchronously, even when all parent requests
        // are denied. Detect this before creating an approvable target revision.
        self.check_current(scope, control)?;
        Ok(PreparedExtensionTool {
            target: self.target(&call),
            call,
            scope: scope.clone(),
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn execute(
        &self,
        prepared: PreparedExtensionTool,
        authorization: AuthorizedCall,
        execution: Execution<'_>,
        store: &impl StoragePort,
        artifacts: &mut impl ExecutionArtifacts,
        snapshot: &Value,
        control: &ScanControl<'_>,
        bridge: &mut impl Bridge,
        on_event: impl FnMut(ToolEvent),
    ) -> Result<Outcome, DomainError> {
        let scope = execution.fence.scope;
        if prepared.scope != *scope
            || prepared.target != self.target(&prepared.call)
            || execution.fence.target_revision != prepared.target
            || authorization.call().operation() != prepared.call.operation()
            || authorization.call().call_id() != prepared.call.call_id()
            || authorization.call().name() != prepared.call.name()
            || authorization.call().args_hash() != prepared.call.args_hash()
        {
            return Err(DomainError::Conflict);
        }
        let registry = execution.registry;
        let fence = execution.fence;
        let clock = execution.clock;
        let mut hooks = ToolHooks {
            tools: self,
            scope,
            snapshot,
            control,
        };
        executor::execute(
            store,
            artifacts,
            authorization,
            execution,
            &mut hooks,
            |ticket| {
                // This closure runs only after the durable intent is acknowledged. Replays
                // return persisted evidence without contacting a dead/reloaded host.
                self.check_current(scope, control)?;
                let call = ticket.consume(
                    registry,
                    ExecutionFence {
                        now: clock.now_ms(),
                        ..fence
                    },
                )?;
                let result=self.host.borrow_mut().call(scope,"tool.execute",&json!({"name":call.name(),"callId":call.call_id(),"arguments":call.args(),"snapshot":snapshot}),control,bridge)?.ok_or_else(||invalid("extension tool result missing"))?;
                // Trusted code can perform its own I/O. A successful response cannot certify
                // a filesystem transaction, so retain Unknown rather than inventing a commit.
                Ok(AdapterOutcome {
                    result,
                    effect: Effect::Unknown,
                    artifact_page: false,
                })
            },
            on_event,
        )
    }
    /// Native command/session-event dispatch publishes dynamic tool changes at
    /// the operation boundary. Capability selection is an owner callback, never
    /// supplied by extension JSON. Preparation/execution use their durable path.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &mut self,
        registry: &mut Registry,
        method: &str,
        args: &Value,
        control: &ScanControl<'_>,
        bridge: &mut impl Bridge,
        capabilities: &mut impl FnMut(&Value) -> Result<BTreeSet<String>, DomainError>,
        activate_new: bool,
    ) -> Result<Option<Value>, DomainError> {
        if !["event", "command", "shortcut"].contains(&method) {
            return Err(DomainError::PermissionDenied);
        }
        let mut refresh = RefreshBridge { inner: bridge };
        let result = self
            .host
            .borrow_mut()
            .call(&self.scope, method, args, control, &mut refresh);
        if !self.host.borrow().is_alive() {
            return result;
        }
        let catalog = self
            .host
            .borrow_mut()
            .call(&self.scope, "catalog", &json!({}), control, &mut DenyBridge)?
            .ok_or_else(|| invalid("extension catalog missing"))?;
        if catalog["tools"] != self.catalog["tools"]
            || catalog["registrationRevision"] != self.catalog["registrationRevision"]
        {
            let previous = self.catalog["tools"]
                .as_array()
                .ok_or_else(|| invalid("extension catalog tools"))?
                .iter()
                .map(|t| {
                    t["name"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid("extension tool name"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let definitions = catalog["tools"]
                .as_array()
                .ok_or_else(|| invalid("extension catalog tools"))?;
            let available = capabilities(&catalog)?;
            registry.replace_extension_group(&previous, definitions, &available, activate_new)?;
        }
        self.revision = digest(&json!([self.host.borrow().identity(), catalog]));
        self.catalog = catalog;
        result
    }
    pub fn stop(&self) -> Result<(), DomainError> {
        self.host.borrow_mut().stop()
    }
}
struct ToolHooks<'a, 'b> {
    tools: &'a ExtensionTools,
    scope: &'a Scope,
    snapshot: &'a Value,
    control: &'a ScanControl<'b>,
}
impl Hooks for ToolHooks<'_, '_> {
    fn before(&mut self, call: &ValidatedCall) -> Result<BeforeHook, DomainError> {
        let result=self.tools.host.borrow_mut().call(self.scope,"event",&json!({"snapshot":self.snapshot,"event":{"type":"tool_call","toolName":call.name(),"toolCallId":call.call_id(),"input":call.args()}}),self.control,&mut DenyBridge)?;
        if let Some(result) = result
            && result["block"] == true
        {
            return Ok(BeforeHook::Block(
                result["reason"]
                    .as_str()
                    .unwrap_or("Extension blocked tool")
                    .into(),
            ));
        }
        Ok(BeforeHook::Continue)
    }
    fn after(
        &mut self,
        call: &ValidatedCall,
        native: &Value,
    ) -> Result<Option<Value>, DomainError> {
        let mut event = json!({"type":"tool_result","toolName":call.name(),"toolCallId":call.call_id(),"input":call.args(),"content":native["content"],"isError":native["isError"]==true});
        if let Some(details) = native.get("details") {
            event["details"] = details.clone();
        }
        let result = self.tools.host.borrow_mut().call(
            self.scope,
            "event",
            &json!({"snapshot":self.snapshot,"event":event}),
            self.control,
            &mut DenyBridge,
        )?;
        Ok(result)
    }
}

struct RefreshBridge<'a, B> {
    inner: &'a mut B,
}
impl<B: Bridge> Bridge for RefreshBridge<'_, B> {
    fn request(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        if r.method == "registry.refreshTools" {
            control.check()?;
            Ok(None)
        } else {
            self.inner.request(r, control)
        }
    }
    fn request_stream(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        if r.method == "registry.refreshTools" {
            self.request(r, control)
        } else {
            self.inner.request_stream(r, control, stream)
        }
    }
    fn event(&mut self, scope: &Scope, operation: &str, event: &Value) -> Result<(), DomainError> {
        self.inner.event(scope, operation, event)
    }
}
