//! Owned optional JavaScript process. Trust admission and scope fences are native;
//! every callback is a request to the core broker, never a parent-state mutation.
use crate::{
    process::{OwnedProcess, ProcessSpec, ProcessSupervisor},
    resource_fs::{ResourceFs, ScanControl},
    workspace::{PinnedDirectory, Workspace, WorkspaceService},
};
use ag1_core::{
    DomainError,
    resources::{catalog::ExtensionResource, packages, trust},
    tools::registry::Scope,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
const FRAME_BYTES: usize = 1024 * 1024;
pub struct Config {
    pub node: PathBuf,
    pub host_script: PathBuf,
    pub spools: PathBuf,
    pub env: BTreeMap<String, String>,
}
pub struct ExtensionService {
    workspace: Arc<Workspace>,
    resources: ResourceFs,
    processes: Arc<ProcessSupervisor>,
    spools: Arc<Workspace>,
    config: Config,
    stamp: u64,
}
pub struct PreparedExtensions {
    scope: Scope,
    stamp: u64,
    entries: Vec<Value>,
    selection: trust::Selection,
}
impl PreparedExtensions {
    pub fn selection(&self) -> &trust::Selection {
        &self.selection
    }
}
/// A trusted embedding port. Implementations prepare/schema-check/policy-authorize
/// each parent action. The request's strings and JSON are not capabilities. Long
/// native work must observe control cancellation/deadlines (P06/P10 adapters).
pub trait NativeStream {
    fn emit(&mut self, event: &Value) -> Result<(), DomainError>;
    /// Observe helper death and cancellation even while provider output stalls.
    fn cancelled(&mut self) -> Result<bool, DomainError>;
    fn invoke_provider(
        &mut self,
        _invocation: ProviderInvocation<'_>,
        _bridge: &mut dyn Bridge,
    ) -> Result<Option<Value>, DomainError> {
        Err(DomainError::Unavailable(
            "nested provider invocation is unavailable".into(),
        ))
    }
}
pub struct ProviderInvocation<'a> {
    pub deadline: Option<Instant>,
    pub name: &'a str,
    pub registration: &'a str,
    pub hook: &'a str,
    pub arguments: &'a Value,
}
pub trait Bridge {
    fn request(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError>;
    /// Streaming callbacks use the same admitted operation and bounded transport.
    fn request_stream(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
        _stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        self.request(request, control)
    }
    fn event(
        &mut self,
        _scope: &Scope,
        _operation: &str,
        _event: &Value,
    ) -> Result<(), DomainError> {
        Ok(())
    }
    /// Nested provider observers may return one validated SDK event for immediate
    /// forwarding to the waiting native stream callback.
    fn provider_event(
        &mut self,
        scope: &Scope,
        operation: &str,
        event: &Value,
    ) -> Result<Option<Value>, DomainError> {
        self.event(scope, operation, event)?;
        Ok(None)
    }
}
pub struct NativeRequest<'a> {
    pub scope: &'a Scope,
    pub operation: &'a str,
    pub phase: &'a str,
    pub extension: &'a str,
    pub method: &'a str,
    pub arguments: &'a Value,
}
pub struct DenyBridge;
impl Bridge for DenyBridge {
    fn request(
        &mut self,
        _: NativeRequest<'_>,
        _: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        Err(DomainError::PermissionDenied)
    }
}
pub struct ExtensionHost {
    child: OwnedProcess,
    workspace: Arc<Workspace>,
    scope: Scope,
    entries: BTreeSet<String>,
    buffer: Vec<u8>,
    deferred: VecDeque<Value>,
    deferred_bytes: usize,
    parked_bytes: usize,
    parked_count: usize,
    serial: u64,
    incarnation: u64,
    alive: bool,
    _cwd: PinnedDirectory,
    _spools: PinnedDirectory,
    spool: PathBuf,
}
fn invalid(message: &str) -> DomainError {
    DomainError::InvalidInput(message.into())
}
impl ExtensionService {
    pub fn new(
        workspace: Arc<Workspace>,
        resources: ResourceFs,
        processes: Arc<ProcessSupervisor>,
        config: Config,
    ) -> Result<Self, DomainError> {
        workspace.verify_root()?;
        if !config.node.is_absolute()
            || !config.node.is_file()
            || !config.host_script.is_absolute()
            || !config.host_script.is_file()
            || !config.spools.is_absolute()
            || config.env.len() > 64
        {
            return Err(invalid("extension host configuration"));
        }
        if serde_json::to_vec(&config.env)
            .map_err(|_| invalid("extension environment"))?
            .len()
            > 16384
        {
            return Err(DomainError::LimitExceeded("extension environment".into()));
        }
        let spools = WorkspaceService::new()?.register(&config.spools)?;
        Ok(Self {
            workspace,
            resources,
            processes,
            config,
            spools,
            stamp: SERIAL.fetch_add(1, Ordering::Relaxed),
        })
    }
    /// Policy inputs come from explicit host options/environment/USER settings.
    /// The renderer/extension RPC has no method that constructs this capability.
    pub fn prepare(
        &self,
        scope: &Scope,
        entries: &[ExtensionResource],
        policy: trust::Policy<'_>,
        control: &ScanControl<'_>,
    ) -> Result<PreparedExtensions, DomainError> {
        control.check()?;
        if scope.workspace != self.workspace.id().as_str() || entries.len() > 64 {
            return Err(DomainError::PermissionDenied);
        }
        let mut captured = vec![];
        let mut paths = vec![];
        for entry in entries {
            let (bytes, revision, canonical) =
                self.resources
                    .read(Path::new(&entry.path), FRAME_BYTES, control)?;
            if revision.sha256 != entry.sha256 {
                return Err(DomainError::Conflict);
            }
            let canonical = canonical
                .to_str()
                .ok_or_else(|| invalid("extension path encoding"))?
                .to_owned();
            paths.push(trust::ExtensionPath {
                path: entry.path.clone(),
                canonical: canonical.clone(),
            });
            captured.push(json!({"path":canonical,"sha256":format!("{:x}",Sha256::digest(bytes)),"sourceInfo":entry.source_info}));
        }
        let selection = trust::select(&paths, policy)?;
        let entries = captured
            .into_iter()
            .zip(entries)
            .filter(|(_, entry)| selection.allowed.contains(&entry.path))
            .map(|(entry, _)| entry)
            .collect::<Vec<_>>();
        packages::validate_settings_document(&json!(entries))?;
        Ok(PreparedExtensions {
            scope: scope.clone(),
            stamp: self.stamp,
            entries,
            selection,
        })
    }
    pub fn start(
        &self,
        prepared: PreparedExtensions,
        scope: &Scope,
        control: &ScanControl<'_>,
    ) -> Result<ExtensionHost, DomainError> {
        control.check()?;
        if self.stamp != prepared.stamp || prepared.scope != *scope {
            return Err(DomainError::Conflict);
        }
        if prepared.entries.is_empty() {
            return Err(DomainError::Unavailable("no admitted extensions".into()));
        }
        self.workspace.verify_root()?;
        for entry in &prepared.entries {
            let (_, revision, canonical) = self.resources.read(
                Path::new(
                    entry["path"]
                        .as_str()
                        .ok_or(DomainError::PermissionDenied)?,
                ),
                FRAME_BYTES,
                control,
            )?;
            if revision.sha256 != entry["sha256"].as_str().unwrap_or("")
                || canonical.to_str() != entry["path"].as_str()
            {
                return Err(DomainError::Conflict);
            }
        }
        let cwd = self.workspace.pin_directory("")?;
        let spools = self.spools.pin_directory("")?;
        let spool = spools.process_cwd().join(format!(
            "extension-{}-{}.log",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let child = self.processes.spawn_protocol(
            ProcessSpec {
                executable: self.config.node.clone(),
                argv: vec![
                    "--max-old-space-size=128".into(),
                    self.config
                        .host_script
                        .to_str()
                        .ok_or_else(|| invalid("extension host path"))?
                        .into(),
                ],
                cwd: cwd.process_cwd(),
                env: self.config.env.clone(),
                timeout_ms: 86400000,
                grace_ms: 100,
                resident: false,
            },
            &spool,
        )?;
        let entries = prepared
            .entries
            .iter()
            .map(|e| e["path"].as_str().unwrap().to_owned())
            .collect();
        let mut host = ExtensionHost {
            child,
            workspace: self.workspace.clone(),
            scope: scope.clone(),
            entries,
            buffer: vec![],
            deferred: VecDeque::new(),
            deferred_bytes: 0,
            parked_bytes: 0,
            parked_count: 0,
            serial: 0,
            incarnation: SERIAL.fetch_add(1, Ordering::Relaxed),
            alive: true,
            _cwd: cwd,
            _spools: spools,
            spool,
        };
        host.call(
            scope,
            "initialize",
            &json!({"cwd":self.workspace.root()}),
            control,
            &mut DenyBridge,
        )?;
        host.call(
            scope,
            "load",
            &json!({"entries":prepared.entries}),
            control,
            &mut DenyBridge,
        )?;
        Ok(host)
    }
}
impl ExtensionHost {
    pub(crate) fn verify_workspace(&self, cwd: &Path) -> Result<(), DomainError> {
        if self.workspace.root() != cwd {
            return Err(DomainError::PermissionDenied);
        }
        self.workspace.verify_root()?;
        self._cwd.verify_identity()
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn is_alive(&self) -> bool {
        self.alive
    }
    pub fn process_id(&self) -> u32 {
        self.child.identity().pid
    }
    fn write(&mut self, frame: &Value, control: &ScanControl<'_>) -> Result<(), DomainError> {
        packages::validate_settings_document(frame)?;
        let mut bytes =
            serde_json::to_vec(frame).map_err(|_| invalid("extension protocol frame"))?;
        if bytes.len() > FRAME_BYTES {
            return Err(DomainError::LimitExceeded(
                "extension protocol frame".into(),
            ));
        }
        bytes.push(b'\n');
        let mut offset = 0;
        while offset < bytes.len() {
            control.check()?;
            if self.child.try_wait()?.is_some() {
                return Err(DomainError::Unavailable("extension host exited".into()));
            }
            match self
                .child
                .try_write_input(&bytes[offset..bytes.len().min(offset + 32768)])?
            {
                Some(0) => return Err(DomainError::Unavailable("extension input closed".into())),
                Some(n) => offset += n,
                None => thread::sleep(Duration::from_millis(5)),
            }
        }
        Ok(())
    }
    fn frame(&mut self, control: &ScanControl<'_>) -> Result<Value, DomainError> {
        if let Some(frame) = self.deferred.pop_front() {
            self.deferred_bytes = self.deferred_bytes.saturating_sub(frame.to_string().len());
            return Ok(frame);
        }
        loop {
            control.check()?;
            if let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
                if end > FRAME_BYTES {
                    return Err(DomainError::LimitExceeded(
                        "extension protocol frame".into(),
                    ));
                }
                let bytes = self.buffer.drain(..=end).collect::<Vec<_>>();
                let frame = serde_json::from_slice(&bytes)
                    .map_err(|_| invalid("extension protocol JSON"))?;
                packages::validate_settings_document(&frame)?;
                return Ok(frame);
            }
            if self.buffer.len() > FRAME_BYTES {
                return Err(DomainError::LimitExceeded(
                    "extension protocol frame".into(),
                ));
            }
            let bytes = self.child.read_stdout(32768)?;
            if bytes.is_empty() {
                if self.child.try_wait()?.is_some() {
                    let final_bytes = self.child.read_stdout(32768)?;
                    if !final_bytes.is_empty() {
                        self.buffer.extend(final_bytes);
                        continue;
                    }
                    return Err(DomainError::Unavailable("extension host exited".into()));
                }
                if self.child.tail(0)?.truncated {
                    return Err(DomainError::LimitExceeded(
                        "extension diagnostic output".into(),
                    ));
                }
                thread::sleep(Duration::from_millis(5));
            } else {
                self.buffer.extend(bytes);
            }
        }
    }
    /// One active request per host. Scope is checked against native ownership,
    /// and callback operation/entry/generation are checked before reaching Bridge.
    /// Any protocol/timeout/cancel failure stops this owned process. A later call
    /// requires a fresh host and cannot replay a half-executed JS callback.
    pub fn call(
        &mut self,
        scope: &Scope,
        method: &str,
        args: &Value,
        control: &ScanControl<'_>,
        bridge: &mut dyn Bridge,
    ) -> Result<Option<Value>, DomainError> {
        control.check()?;
        let control = &ScanControl {
            cancel: control.cancel,
            deadline: control
                .deadline
                .min(Instant::now() + Duration::from_secs(300)),
        };
        if scope != &self.scope {
            return Err(DomainError::Conflict);
        }
        if !self.alive {
            return Err(DomainError::Unavailable("extension host is closed".into()));
        }
        if !matches!(
            method,
            "initialize"
                | "load"
                | "bind"
                | "catalog"
                | "models.update"
                | "flags.set"
                | "flags.values"
                | "shortcuts.resolve"
                | "shortcut"
                | "event"
                | "tool.prepare"
                | "tool.execute"
                | "command"
                | "provider"
        ) {
            return Err(DomainError::Unavailable("extension host method".into()));
        }
        self.serial = self.serial.checked_add(1).ok_or(DomainError::Conflict)?;
        let id = format!("extension-{}-{}", self.incarnation, self.serial);
        let mut callback_error = false;
        let result = (|| {
            self.write(
                &json!({"v":1,"kind":"request","id":id,"method":method,"args":args}),
                control,
            )?;
            let mut seen = BTreeSet::new();
            let mut events = 0usize;
            loop {
                let frame = self.frame(control)?;
                if frame["v"] != 1 {
                    return Err(invalid("extension protocol version"));
                }
                match frame["kind"].as_str() {
                    Some("response") => {
                        if frame["id"] != id {
                            return Err(DomainError::Conflict);
                        }
                        if let Some(error) = frame.get("error") {
                            callback_error = true;
                            return Err(DomainError::Unavailable(format!(
                                "extension callback: {}",
                                error["message"].as_str().unwrap_or("failed")
                            )));
                        }
                        return match frame["result"]["defined"].as_bool() {
                            Some(false) => Ok(None),
                            Some(true) => Ok(Some(frame["result"]["value"].clone())),
                            None => Err(invalid("extension result packet")),
                        };
                    }
                    Some("event") => {
                        if frame["id"] != id || frame.get("invocation").is_some() {
                            return Err(DomainError::Conflict);
                        }
                        events += 1;
                        if events > 10000 {
                            return Err(DomainError::LimitExceeded("extension events".into()));
                        }
                        bridge.event(scope, &id, &frame["event"])?;
                    }
                    Some("call_cancel") => {
                        if frame["v"] != 1
                            || !frame["id"].as_str().is_some_and(|v| seen.contains(v))
                        {
                            return Err(DomainError::Conflict);
                        }
                        // A cancellation racing a completed callback is harmless.
                    }
                    Some("call") => {
                        let callback = frame["id"]
                            .as_str()
                            .filter(|v| !v.is_empty() && v.len() <= 256)
                            .ok_or_else(|| invalid("extension callback id"))?;
                        let extension = frame["extension"]
                            .as_str()
                            .ok_or(DomainError::PermissionDenied)?;
                        if frame["operation"] != id
                            || frame.get("invocation").is_some()
                            || frame["generation"] != 0
                            || !self.entries.contains(extension)
                            || seen.len() >= 256
                            || !seen.insert(callback.to_owned())
                        {
                            return Err(DomainError::PermissionDenied);
                        }
                        let native_method = frame["method"]
                            .as_str()
                            .filter(|v| v.len() <= 256)
                            .ok_or(DomainError::PermissionDenied)?;
                        // Preparation hooks can transform data, but cannot use a
                        // native callback to bypass the tool's subsequent policy.
                        let answer = if method == "tool.prepare" {
                            Err(DomainError::PermissionDenied)
                        } else {
                            bridge.request_stream(
                                NativeRequest {
                                    scope,
                                    operation: &id,
                                    phase: method,
                                    extension,
                                    method: native_method,
                                    arguments: &frame["args"],
                                },
                                control,
                                &mut CallbackStream {
                                    host: self,
                                    callback,
                                    control,
                                    cancelled: false,
                                    operation: &id,
                                    seen: &mut seen,
                                    events: &mut events,
                                    depth: 0,
                                },
                            )
                        };
                        let response = match answer {
                            Ok(Some(value)) => {
                                json!({"v":1,"kind":"response","id":callback,"result":{"defined":true,"value":value}})
                            }
                            Ok(None) => {
                                json!({"v":1,"kind":"response","id":callback,"result":{"defined":false}})
                            }
                            Err(error) => {
                                json!({"v":1,"kind":"response","id":callback,"error":{"message":format!("{error:?}")}})
                            }
                        };
                        self.write(&response, control)?;
                    }
                    _ => return Err(invalid("extension protocol packet kind")),
                }
            }
        })();
        if result.is_err() && !callback_error {
            self.alive = false;
            let _ = self.child.stop();
        }
        result
    }
    /// Cancellation uses bounded teardown of this owned process;
    /// the native durable executor decides whether any parent effect is uncertain.
    pub fn stop(&mut self) -> Result<(), DomainError> {
        self.alive = false;
        self.child.stop()
    }
    pub fn diagnostics(&self) -> Result<Vec<u8>, DomainError> {
        Ok(self.child.tail(65536)?.bytes)
    }
    pub(crate) fn identity(&self) -> u64 {
        self.incarnation
    }
    pub(crate) fn admits_extension(&self, path: &str) -> bool {
        self.entries.contains(path)
    }
    pub fn spool_path(&self) -> &Path {
        &self.spool
    }
}
impl Drop for ExtensionHost {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

struct CallbackStream<'a, 'b> {
    host: &'a mut ExtensionHost,
    callback: &'a str,
    control: &'a ScanControl<'b>,
    cancelled: bool,
    operation: &'a str,
    seen: &'a mut BTreeSet<String>,
    events: &'a mut usize,
    depth: usize,
}
impl CallbackStream<'_, '_> {
    fn invoke_provider_unbounded(
        &mut self,
        invocation: ProviderInvocation<'_>,
        bridge: &mut dyn Bridge,
    ) -> Result<Option<Value>, DomainError> {
        if self.depth >= 4
            || invocation.name.len() > 256
            || invocation.registration.len() > 256
            || invocation.name.is_empty()
            || invocation.registration.is_empty()
            || !matches!(
                invocation.hook,
                "streamSimple"
                    | "oauth.login"
                    | "oauth.refreshToken"
                    | "oauth.getApiKey"
                    | "oauth.modifyModels"
            )
        {
            return Err(DomainError::PermissionDenied);
        }
        if self.cancelled()? {
            return Err(DomainError::Cancelled);
        }
        self.host.serial = self
            .host
            .serial
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let id = format!("provider-{}-{}", self.host.incarnation, self.host.serial);
        let scope = self.host.scope.clone();
        self.host.write(&json!({"v":1,"kind":"invoke","id":id,"parent":self.callback,"method":"provider",
            "args":{"name":invocation.name,"registration":invocation.registration,"hook":invocation.hook,"arguments":invocation.arguments}}), self.control)?;
        let mut waiting = std::mem::take(&mut self.host.deferred);
        let mut waiting_bytes = std::mem::take(&mut self.host.deferred_bytes);
        self.host.parked_bytes += waiting_bytes;
        self.host.parked_count += waiting.len();
        let mut callback_error = false;
        let result = (|| {
            loop {
                let frame = self.host.frame(self.control)?;
                if frame["v"] != 1 {
                    return Err(invalid("nested provider protocol version"));
                }
                match frame["kind"].as_str() {
                    Some("invoke_response") => {
                        if frame["id"] != id {
                            return Err(DomainError::Conflict);
                        }
                        if let Some(error) = frame.get("error") {
                            callback_error = true;
                            return Err(DomainError::Unavailable(format!(
                                "extension provider callback: {}",
                                error["message"].as_str().unwrap_or("failed")
                            )));
                        }
                        return match frame["result"]["defined"].as_bool() {
                            Some(false) => Ok(None),
                            Some(true) => Ok(Some(frame["result"]["value"].clone())),
                            None => Err(invalid("nested provider result packet")),
                        };
                    }
                    Some("call_cancel") if frame["id"] == self.callback => {
                        self.cancelled = true;
                        return Err(DomainError::Cancelled);
                    }
                    Some("call_cancel")
                        if frame["id"]
                            .as_str()
                            .is_some_and(|id| self.seen.contains(id)) => {}
                    Some("event") if frame["invocation"] == id => {
                        if frame["id"] != self.operation {
                            return Err(DomainError::Conflict);
                        }
                        *self.events += 1;
                        if *self.events > 10000 {
                            return Err(DomainError::LimitExceeded("extension events".into()));
                        }
                        if let Some(event) =
                            bridge.provider_event(&scope, self.operation, &frame["event"])?
                        {
                            self.emit(&event)?;
                        }
                    }
                    Some("call") if frame["invocation"] == id => {
                        let callback = frame["id"]
                            .as_str()
                            .filter(|v| !v.is_empty() && v.len() <= 256)
                            .ok_or_else(|| invalid("nested callback id"))?;
                        let extension = frame["extension"]
                            .as_str()
                            .ok_or(DomainError::PermissionDenied)?;
                        if frame["operation"] != self.operation
                            || frame["generation"] != 0
                            || !self.host.entries.contains(extension)
                            || self.seen.len() >= 256
                            || !self.seen.insert(callback.to_owned())
                        {
                            return Err(DomainError::PermissionDenied);
                        }
                        let method = frame["method"]
                            .as_str()
                            .filter(|v| v.len() <= 256)
                            .ok_or(DomainError::PermissionDenied)?;
                        let answer = bridge.request_stream(
                            NativeRequest {
                                scope: &scope,
                                operation: self.operation,
                                phase: "provider",
                                extension,
                                method,
                                arguments: &frame["args"],
                            },
                            self.control,
                            &mut CallbackStream {
                                host: self.host,
                                callback,
                                control: self.control,
                                cancelled: false,
                                operation: self.operation,
                                seen: self.seen,
                                events: self.events,
                                depth: self.depth + 1,
                            },
                        );
                        let response = match answer {
                            Ok(Some(value)) => {
                                json!({"v":1,"kind":"response","id":callback,"result":{"defined":true,"value":value}})
                            }
                            Ok(None) => {
                                json!({"v":1,"kind":"response","id":callback,"result":{"defined":false}})
                            }
                            Err(error) => {
                                json!({"v":1,"kind":"response","id":callback,"error":{"message":format!("{error:?}")}})
                            }
                        };
                        self.host.write(&response, self.control)?;
                    }
                    Some("call" | "event" | "call_cancel")
                        if frame.get("invocation").is_none() || frame["invocation"] != id =>
                    {
                        let bytes = frame.to_string().len();
                        if self.host.parked_count + self.host.deferred.len() >= 256
                            || self.host.parked_bytes + self.host.deferred_bytes + bytes
                                > FRAME_BYTES
                        {
                            return Err(DomainError::LimitExceeded(
                                "extension deferred callbacks".into(),
                            ));
                        }
                        waiting_bytes += bytes;
                        self.host.parked_bytes += bytes;
                        self.host.parked_count += 1;
                        waiting.push_back(frame);
                    }
                    _ => return Err(invalid("nested provider protocol packet")),
                }
            }
        })();
        self.host.parked_bytes = self.host.parked_bytes.saturating_sub(waiting_bytes);
        self.host.parked_count = self.host.parked_count.saturating_sub(waiting.len());
        waiting.append(&mut self.host.deferred);
        self.host.deferred = waiting;
        self.host.deferred_bytes += waiting_bytes;
        if result.is_err() && !callback_error {
            self.host.alive = false;
            let _ = self.host.child.stop();
        }
        result
    }
}
impl NativeStream for CallbackStream<'_, '_> {
    fn invoke_provider(
        &mut self,
        invocation: ProviderInvocation<'_>,
        bridge: &mut dyn Bridge,
    ) -> Result<Option<Value>, DomainError> {
        let control = ScanControl {
            cancel: self.control.cancel,
            deadline: self
                .control
                .deadline
                .min(invocation.deadline.unwrap_or(self.control.deadline)),
        };
        control.check()?;
        let mut bounded = CallbackStream {
            host: self.host,
            callback: self.callback,
            control: &control,
            cancelled: self.cancelled,
            operation: self.operation,
            seen: self.seen,
            events: self.events,
            depth: self.depth,
        };
        let result = bounded.invoke_provider_unbounded(invocation, bridge);
        self.cancelled = bounded.cancelled;
        result
    }
    fn emit(&mut self, event: &Value) -> Result<(), DomainError> {
        self.host.write(
            &json!({"v":1,"kind":"stream","id":self.callback,"event":event}),
            self.control,
        )
    }
    fn cancelled(&mut self) -> Result<bool, DomainError> {
        self.control.check()?;
        let mut i = 0;
        while i < self.host.deferred.len() {
            let frame = &self.host.deferred[i];
            if frame["v"] == 1 && frame["kind"] == "call_cancel" && frame["id"] == self.callback {
                let frame = self.host.deferred.remove(i).ok_or(DomainError::Conflict)?;
                self.host.deferred_bytes = self
                    .host
                    .deferred_bytes
                    .saturating_sub(frame.to_string().len());
                self.cancelled = true;
            } else {
                i += 1;
            }
        }
        if self.host.child.try_wait()?.is_some() {
            return Err(DomainError::Unavailable("extension host exited".into()));
        }
        if self.host.child.tail(0)?.truncated {
            return Err(DomainError::LimitExceeded(
                "extension diagnostic output".into(),
            ));
        }
        let bytes = self.host.child.read_stdout(32768)?;
        self.host.buffer.extend(bytes);
        while let Some(end) = self.host.buffer.iter().position(|b| *b == b'\n') {
            if end > FRAME_BYTES {
                return Err(DomainError::LimitExceeded(
                    "extension protocol frame".into(),
                ));
            }
            let bytes = self.host.buffer.drain(..=end).collect::<Vec<_>>();
            let frame: Value =
                serde_json::from_slice(&bytes).map_err(|_| invalid("extension protocol JSON"))?;
            packages::validate_settings_document(&frame)?;
            if frame["v"] == 1 && frame["kind"] == "call_cancel" && frame["id"] == self.callback {
                self.cancelled = true;
            } else {
                let n = frame.to_string().len();
                if self.host.deferred.len() + self.host.parked_count >= 256
                    || self.host.deferred_bytes + self.host.parked_bytes + n > FRAME_BYTES
                {
                    return Err(DomainError::LimitExceeded(
                        "extension deferred callbacks".into(),
                    ));
                }
                self.host.deferred_bytes += n;
                self.host.deferred.push_back(frame);
            }
        }
        if self.host.buffer.len() > FRAME_BYTES {
            return Err(DomainError::LimitExceeded(
                "extension protocol frame".into(),
            ));
        }
        Ok(self.cancelled)
    }
}
