//! Trusted package lifecycle with explicit writable roots and executable bindings.
//! This adapter is not an OS sandbox: approved package scripts have the host
//! account's privileges. It never inherits credentials or selects a system npm prefix.
use crate::{
    io_error,
    process::{ProcessCompletion, ProcessSpec, ProcessSupervisor},
    resource_fs::ScanControl,
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::{
    DomainError,
    context::ContextArtifactPort,
    resources::{
        package_lifecycle::{self as lifecycle, Manager, Port, Run},
        packages,
    },
    storage::StoragePort,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{self, AdapterOutcome, Effect, ExecutionArtifacts, Hooks},
        registry::{Origin, Registry, Scope, ValidatedCall},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
const OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_COMMANDS: usize = 32;

/// These are host capabilities, never extension/model argument fields. The host
/// must register dedicated cache roots; registering a project root for writing
/// would also grant deletion within that entire root.
pub struct Config {
    pub manager: lifecycle::Config,
    pub writable_roots: Vec<Arc<Workspace>>,
    pub readable_roots: Vec<Arc<Workspace>>,
    pub npm_prefix: Arc<Workspace>,
    pub binaries: BTreeMap<String, Binary>,
    pub env: BTreeMap<String, String>,
    pub spools: PathBuf,
}
pub struct Binary {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
}
struct BoundBinary {
    executable: PathBuf,
    file: std::fs::File,
    arguments: Vec<String>,
    identity: BinaryIdentity,
}
pub(crate) type BinaryIdentity = (u64, u64, u64, i64, i64, i64, i64);
pub(crate) fn binary_identity(path: &Path) -> Result<BinaryIdentity, DomainError> {
    let m = std::fs::symlink_metadata(path).map_err(io_error)?;
    if !m.is_file() || m.is_symlink() || m.mode() & 0o111 == 0 {
        return Err(DomainError::PermissionDenied);
    }
    Ok((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
fn normalized(path: &Path) -> bool {
    path.is_absolute()
        && path.as_os_str().len() <= 8192
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}
fn resolve(roots: &[Arc<Workspace>], path: &str) -> Result<(Arc<Workspace>, String), DomainError> {
    let path = Path::new(path);
    if !normalized(path) {
        return Err(DomainError::PermissionDenied);
    }
    for root in roots {
        if let Ok(relative) = path.strip_prefix(root.root()) {
            root.verify_root()?;
            return Ok((
                root.clone(),
                relative
                    .to_str()
                    .ok_or(DomainError::PermissionDenied)?
                    .into(),
            ));
        }
    }
    Err(DomainError::PermissionDenied)
}
struct State {
    manager: Manager,
    generation: u64,
}
pub struct PackageService {
    workspace: Arc<Workspace>,
    processes: Arc<ProcessSupervisor>,
    writable: Vec<Arc<Workspace>>,
    readable: Vec<Arc<Workspace>>,
    npm_prefix: Arc<Workspace>,
    binaries: BTreeMap<String, BoundBinary>,
    npm_command: String,
    env: BTreeMap<String, String>,
    spools: Arc<Workspace>,
    stamp: String,
    state: Mutex<State>,
}
pub struct PreparedPackage {
    scope: Scope,
    call: ValidatedCall,
    stamp: String,
    generation: u64,
    action: String,
    source: String,
    package_scope: lifecycle::Scope,
}
impl PreparedPackage {
    pub fn call(&self) -> &ValidatedCall {
        &self.call
    }
    pub fn target_revision(&self) -> &str {
        &self.stamp
    }
}
pub struct Execution<'a> {
    pub lifecycle: executor::Execution<'a>,
    pub control: ScanControl<'a>,
}
fn no_effect_failure(error: DomainError) -> AdapterOutcome {
    let error = format!("{error:?}");
    AdapterOutcome {
        result: json!({"content":[{"type":"text","text":format!("Package operation did not start: {error}")}],"isError":true,"details":{"error":error,"events":[],"commands":[]}}),
        effect: Effect::None,
        artifact_page: false,
    }
}
impl PackageService {
    /// Read-only handoff from an installed package to the resource loader. This
    /// is an observation, not an installation or trust grant. The loader captures
    /// and validates manifests/resources again before publishing its generation.
    pub fn installation_record(
        &self,
        source: &str,
        scope: lifecycle::Scope,
        control: ScanControl<'_>,
    ) -> Result<Option<crate::resources::InstalledPackage>, DomainError> {
        control.check()?;
        self.workspace.verify_root()?;
        let parsed = packages::parse(source)?;
        lifecycle::validate_execution_source(&parsed)?;
        let mut state = self.state.try_lock().map_err(|_| DomainError::Conflict)?;
        let mut port = InspectionPort {
            service: self,
            control,
        };
        let path = state.manager.installed_path(&mut port, &parsed, scope)?;
        let (root, relative) = resolve(&self.readable, &path)?;
        if !root.directory_exists(&relative)?
            || (matches!(parsed, packages::Source::Npm { pinned: true, .. })
                && !state.manager.pinned_matches(&mut port, &parsed, &path)?)
        {
            return Ok(None);
        }
        port.checkpoint()?;
        Ok(Some(crate::resources::InstalledPackage {
            source: source.into(),
            scope: match scope {
                lifecycle::Scope::User => "user",
                lifecycle::Scope::Project => "project",
                lifecycle::Scope::Temporary => "temporary",
            }
            .into(),
            identity: parsed.identity(&path),
            root: std::path::PathBuf::from(path),
        }))
    }
    /// Host-only discovery from captured settings. The owned npm prefix is
    /// already a capability; querying it never needs to launch an npm process.
    pub fn update_check_queries(
        &self,
        entries: &[lifecycle::ConfiguredPackage],
        control: ScanControl<'_>,
    ) -> Result<Vec<ag1_core::resources::package_queries::Query>, DomainError> {
        control.check()?;
        self.workspace.verify_root()?;
        let mut state = self.state.try_lock().map_err(|_| DomainError::Conflict)?;
        state.manager.prepare_update_checks(
            &mut InspectionPort {
                service: self,
                control,
            },
            entries,
        )
    }
    pub fn needs_install(
        &self,
        source: &str,
        scope: lifecycle::Scope,
        control: ScanControl<'_>,
    ) -> Result<bool, DomainError> {
        control.check()?;
        self.workspace.verify_root()?;
        let parsed = packages::parse(source)?;
        lifecycle::validate_execution_source(&parsed)?;
        let mut state = self.state.try_lock().map_err(|_| DomainError::Conflict)?;
        state.manager.needs_install(
            &mut InspectionPort {
                service: self,
                control,
            },
            &parsed,
            scope,
        )
    }
    pub fn new(
        workspace: Arc<Workspace>,
        processes: Arc<ProcessSupervisor>,
        mut config: Config,
    ) -> Result<Self, DomainError> {
        workspace.verify_root()?;
        config.npm_prefix.verify_root()?;
        if config.writable_roots.is_empty()
            || config.writable_roots.len() > 16
            || config.readable_roots.len() > 64
            || !normalized(&config.spools)
            || !config.spools.is_dir()
            || config.env.len() > 96
            || config.binaries.len() != 2
            || config.manager.npm.command == "git"
        {
            return Err(DomainError::InvalidInput(
                "package host configuration".into(),
            ));
        }
        for root in &config.writable_roots {
            root.verify_root()?;
        }
        if !config
            .writable_roots
            .iter()
            .any(|r| Arc::ptr_eq(r, &config.npm_prefix))
        {
            return Err(DomainError::PermissionDenied);
        }
        config
            .writable_roots
            .sort_by_key(|r| std::cmp::Reverse(r.root().as_os_str().len()));
        config
            .readable_roots
            .extend(config.writable_roots.iter().cloned());
        config.readable_roots.push(workspace.clone());
        config
            .readable_roots
            .sort_by_key(|r| std::cmp::Reverse(r.root().as_os_str().len()));
        let mut binaries = BTreeMap::new();
        for (name, binary) in config.binaries {
            if (name != "git" && name != config.manager.npm.command)
                || !normalized(&binary.executable)
                || binary.arguments.len() > 32
                || binary
                    .arguments
                    .iter()
                    .any(|a| a.len() > 8192 || a.contains('\0'))
            {
                return Err(DomainError::InvalidInput(
                    "package executable binding".into(),
                ));
            }
            let executable = binary.executable.canonicalize().map_err(io_error)?;
            let identity = binary_identity(&executable)?;
            use std::os::unix::fs::OpenOptionsExt;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&executable)
                .map_err(io_error)?;
            let held = file.metadata().map_err(io_error)?;
            if held.dev() != identity.0 || held.ino() != identity.1 {
                return Err(DomainError::Conflict);
            }
            binaries.insert(
                name,
                BoundBinary {
                    executable,
                    file,
                    arguments: binary.arguments,
                    identity,
                },
            );
        }
        let prefix = config
            .npm_prefix
            .root()
            .to_str()
            .ok_or(DomainError::PermissionDenied)?
            .to_owned();
        for (key, value) in [
            ("NPM_CONFIG_PREFIX", prefix.clone()),
            ("npm_config_prefix", prefix.clone()),
            ("NPM_CONFIG_USERCONFIG", "/dev/null".into()),
            ("npm_config_userconfig", "/dev/null".into()),
            ("NPM_CONFIG_GLOBALCONFIG", "/dev/null".into()),
            ("npm_config_globalconfig", "/dev/null".into()),
            ("NPM_CONFIG_CACHE", format!("{prefix}/.cache")),
            ("npm_config_cache", format!("{prefix}/.cache")),
            ("GIT_CONFIG_NOSYSTEM", "1".into()),
            ("GIT_CONFIG_GLOBAL", "/dev/null".into()),
            ("GIT_TERMINAL_PROMPT", "0".into()),
        ] {
            config.env.insert(key.into(), value);
        }
        if serde_json::to_vec(&config.env)
            .map_err(|_| DomainError::InvalidInput("package environment".into()))?
            .len()
            > 16 * 1024
        {
            return Err(DomainError::LimitExceeded("package environment".into()));
        }
        let stamp = format!(
            "package:{}:{}",
            workspace.id().as_str(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let npm_command = config.manager.npm.command.clone();
        Ok(Self {
            workspace,
            processes,
            writable: config.writable_roots,
            readable: config.readable_roots,
            npm_prefix: config.npm_prefix,
            binaries,
            npm_command,
            env: config.env,
            spools: WorkspaceService::new()?.register(&config.spools)?,
            stamp,
            state: Mutex::new(State {
                manager: Manager::new(config.manager)?,
                generation: 0,
            }),
        })
    }
    pub fn register(registry: &mut Registry) -> Result<(), DomainError> {
        registry.register(json!({"name":"managePackage","description":"Install, remove or update a trusted resource package in a host-selected cache. Package scripts execute with host privileges.","parameters":{"type":"object","properties":{"action":{"type":"string","enum":["install","remove","update"]},"source":{"type":"string"},"scope":{"type":"string","enum":["user","project","temporary"]}},"required":["action","source","scope"],"additionalProperties":false}}), Origin::TrustedHost)
    }
    fn target(&self, generation: u64, call: &ValidatedCall) -> String {
        format!(
            "{:x}",
            Sha256::digest(json!([self.stamp, generation, call.args_hash()]).to_string())
        )
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
    ) -> Result<PreparedPackage, DomainError> {
        if call.origin() != Origin::TrustedHost
            || call.name() != "managePackage"
            || scope.workspace != self.workspace.id().as_str()
        {
            return Err(DomainError::PermissionDenied);
        }
        self.workspace.verify_root()?;
        let source = call.args()["source"]
            .as_str()
            .ok_or(DomainError::PermissionDenied)?;
        let parsed = packages::parse(source)?;
        lifecycle::validate_execution_source(&parsed)?;
        let action = call.args()["action"]
            .as_str()
            .ok_or(DomainError::PermissionDenied)?
            .to_owned();
        if !matches!(action.as_str(), "install" | "remove" | "update") {
            return Err(DomainError::PermissionDenied);
        }
        let package_scope = match call.args()["scope"].as_str() {
            Some("user") => lifecycle::Scope::User,
            Some("project") => lifecycle::Scope::Project,
            Some("temporary") => lifecycle::Scope::Temporary,
            _ => return Err(DomainError::PermissionDenied),
        };
        let state = self.state.try_lock().map_err(|_| DomainError::Conflict)?;
        Ok(PreparedPackage {
            scope: scope.clone(),
            call: call.clone(),
            stamp: self.target(state.generation, call),
            generation: state.generation,
            action,
            source: source.into(),
            package_scope,
        })
    }
    /// The durable intent and native output artifacts are mandatory. Recovery of
    /// an unfinished operation returns uncertainty and never reruns npm/git.
    pub fn execute<A: ExecutionArtifacts>(
        &self,
        store: &impl StoragePort,
        artifacts: &mut A,
        prepared: PreparedPackage,
        authorization: AuthorizedCall,
        execution: Execution<'_>,
        hooks: &mut impl Hooks,
    ) -> Result<executor::Outcome, DomainError> {
        if self.target(prepared.generation, &prepared.call) != prepared.stamp
            || prepared.scope != *execution.lifecycle.fence.scope
            || prepared.stamp != execution.lifecycle.fence.target_revision
            || authorization.call().operation() != prepared.call.operation()
            || authorization.call().call_id() != prepared.call.call_id()
            || authorization.call().args_hash() != prepared.call.args_hash()
        {
            return Err(DomainError::PermissionDenied);
        }
        let registry = execution.lifecycle.registry;
        let fence = execution.lifecycle.fence;
        let clock = execution.lifecycle.clock;
        executor::execute_with_artifacts(
            store,
            artifacts,
            authorization,
            execution.lifecycle,
            hooks,
            |ticket, artifacts| {
                let mut state = loop {
                    if let Err(error) = execution.control.check() {
                        return Ok(no_effect_failure(error));
                    }
                    match self.state.try_lock() {
                        Ok(state) => break state,
                        Err(TryLockError::Poisoned(_)) => {
                            return Err(DomainError::DurabilityUnknown);
                        }
                        Err(TryLockError::WouldBlock) => thread::sleep(Duration::from_millis(5)),
                    }
                };
                if state.generation != prepared.generation
                    || self.target(state.generation, &prepared.call) != prepared.stamp
                {
                    return Ok(no_effect_failure(DomainError::Conflict));
                }
                if let Err(error) = self.workspace.verify_root() {
                    return Ok(no_effect_failure(error));
                }
                if let Err(error) = ticket.consume(
                    registry,
                    ExecutionFence {
                        now: clock.now_ms(),
                        ..fence
                    },
                ) {
                    return Ok(no_effect_failure(error));
                }
                let mut port = NativePort {
                    service: self,
                    control: execution.control,
                    artifacts,
                    events: vec![],
                    commands: vec![],
                    effect: false,
                    pending_output: false,
                    operation: prepared.call.operation().into(),
                };
                let result = match prepared.action.as_str() {
                    "install" => {
                        state
                            .manager
                            .install(&mut port, &prepared.source, prepared.package_scope)
                    }
                    "remove" => {
                        state
                            .manager
                            .remove(&mut port, &prepared.source, prepared.package_scope)
                    }
                    "update" => state.manager.update_source(
                        &mut port,
                        &prepared.source,
                        prepared.package_scope,
                    ),
                    _ => unreachable!(),
                };
                state.generation = state
                    .generation
                    .checked_add(1)
                    .ok_or(DomainError::DurabilityUnknown)?;
                if port.pending_output {
                    return Err(DomainError::DurabilityUnknown);
                }
                let effect = if !port.effect {
                    Effect::None
                } else if result.is_ok() {
                    Effect::Committed
                } else {
                    Effect::Unknown
                };
                let error = result.err().map(|e| format!("{e:?}"));
                Ok(AdapterOutcome {
                    result: json!({"content":[{"type":"text","text":error.as_ref().map_or_else(||format!("Package {} completed.", prepared.action), |e|format!("Package {} failed: {e}", prepared.action))}],"isError":error.is_some(),"details":{"action":prepared.action,"source":prepared.source,"scope":prepared.package_scope,"error":error,"events":port.events,"commands":port.commands}}),
                    effect,
                    artifact_page: false,
                })
            },
            |_| {},
        )
    }
}
/// No subprocess or mutation capability exists in this manager inspection port.
struct InspectionPort<'a> {
    service: &'a PackageService,
    control: ScanControl<'a>,
}
impl Port for InspectionPort<'_> {
    fn checkpoint(&mut self) -> Result<(), DomainError> {
        self.control.check()
    }
    fn exists(&mut self, path: &str) -> Result<bool, DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.readable, path)?;
        if name.is_empty() {
            Ok(true)
        } else {
            root.exists(&name)
        }
    }
    fn read_json(&mut self, path: &str) -> Result<Option<Value>, DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.readable, path)?;
        match root.snapshot(&name, 1024 * 1024) {
            Ok(snapshot) => {
                let value = serde_json::from_slice(&snapshot.bytes)
                    .map_err(|_| DomainError::InvalidInput("package JSON".into()))?;
                packages::validate_settings_document(&value)?;
                self.checkpoint()?;
                Ok(Some(value))
            }
            Err(DomainError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn create_directory(&mut self, _: &str) -> Result<(), DomainError> {
        Err(DomainError::PermissionDenied)
    }
    fn write_absent(&mut self, _: &str, _: &[u8]) -> Result<(), DomainError> {
        Err(DomainError::PermissionDenied)
    }
    fn remove_owned_tree(&mut self, _: &str, _: Option<&str>) -> Result<(), DomainError> {
        Err(DomainError::PermissionDenied)
    }
    fn progress(&mut self, _: Value) -> Result<(), DomainError> {
        Err(DomainError::PermissionDenied)
    }
    fn run(&mut self, command: Run) -> Result<String, DomainError> {
        self.checkpoint()?;
        if command.command != self.service.npm_command
            || command.kind != "sync"
            || command.options.is_some()
            || command.args.len() < 2
            || command.args.len() > 66
            || command.args[command.args.len() - 2..] != ["root", "-g"]
        {
            return Err(DomainError::PermissionDenied);
        }
        self.service.npm_prefix.verify_root()?;
        self.service
            .npm_prefix
            .root()
            .join("lib/node_modules")
            .to_str()
            .map(str::to_owned)
            .ok_or(DomainError::PermissionDenied)
    }
}
struct NativePort<'a, A: ContextArtifactPort> {
    service: &'a PackageService,
    control: ScanControl<'a>,
    artifacts: &'a mut A,
    events: Vec<Value>,
    commands: Vec<Value>,
    effect: bool,
    pending_output: bool,
    operation: String,
}
impl<A: ContextArtifactPort> Port for NativePort<'_, A> {
    fn checkpoint(&mut self) -> Result<(), DomainError> {
        self.control.check()
    }
    fn exists(&mut self, path: &str) -> Result<bool, DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.readable, path)?;
        if name.is_empty() {
            Ok(true)
        } else {
            root.exists(&name)
        }
    }
    fn read_json(&mut self, path: &str) -> Result<Option<Value>, DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.readable, path)?;
        match root.snapshot(&name, 1024 * 1024) {
            Ok(s) => {
                let value = serde_json::from_slice(&s.bytes)
                    .map_err(|_| DomainError::InvalidInput("package JSON".into()))?;
                packages::validate_settings_document(&value)?;
                Ok(Some(value))
            }
            Err(DomainError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn create_directory(&mut self, path: &str) -> Result<(), DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.writable, path)?;
        self.effect |= !name.is_empty();
        root.create_directories_checked(&name, || self.control.check())
    }
    fn write_absent(&mut self, path: &str, bytes: &[u8]) -> Result<(), DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.writable, path)?;
        if bytes.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("package document".into()));
        }
        self.effect = true;
        root.create_checked(&name, bytes, false, || self.control.check())
            .map(|_| ())
    }
    fn remove_owned_tree(
        &mut self,
        path: &str,
        prune_root: Option<&str>,
    ) -> Result<(), DomainError> {
        self.checkpoint()?;
        let (root, name) = resolve(&self.service.writable, path)?;
        if name.is_empty() {
            return Err(DomainError::PermissionDenied);
        }
        let stop = prune_root.unwrap_or(root.root().to_str().ok_or(DomainError::PermissionDenied)?);
        let (stop_root, _) = resolve(&self.service.writable, stop)?;
        if !Arc::ptr_eq(&root, &stop_root) || !Path::new(path).starts_with(stop) || path == stop {
            return Err(DomainError::PermissionDenied);
        }
        let pinned = root.pin_directory(&name)?;
        self.effect = true;
        root.remove_directory_tree_checked(pinned, || self.control.check())?;
        let mut parent = Path::new(path).parent();
        while let Some(path) = parent.filter(|p| *p != Path::new(stop) && p.starts_with(stop)) {
            self.checkpoint()?;
            let name = path
                .strip_prefix(root.root())
                .map_err(|_| DomainError::PermissionDenied)?
                .to_str()
                .ok_or(DomainError::PermissionDenied)?;
            if name.is_empty()
                || !root.remove_empty_directory_checked(root.pin_directory(name)?, || {
                    self.control.check()
                })?
            {
                break;
            }
            parent = path.parent();
        }
        Ok(())
    }
    fn progress(&mut self, event: Value) -> Result<(), DomainError> {
        // Terminal progress remains recordable after cancellation. It cannot
        // dispatch another operation or modify the parent session.
        if self.events.len() >= 128 || event.to_string().len() > 32768 {
            return Err(DomainError::LimitExceeded("package progress".into()));
        }
        self.events.push(event);
        Ok(())
    }
    fn run(&mut self, command: Run) -> Result<String, DomainError> {
        self.checkpoint()?;
        if self.commands.len() >= MAX_COMMANDS
            || command.args.len() > 128
            || command
                .args
                .iter()
                .any(|s| s.len() > 8192 || s.contains('\0'))
            || !matches!(command.kind.as_str(), "run" | "capture" | "sync")
            || command.options.as_ref().is_some_and(|o| o.env.is_some())
        {
            return Err(DomainError::LimitExceeded("package command".into()));
        }
        let binary = self
            .service
            .binaries
            .get(&command.command)
            .ok_or(DomainError::PermissionDenied)?;
        if binary_identity(&binary.executable)? != binary.identity {
            return Err(DomainError::Conflict);
        }
        let cwd = command.options.as_ref().map(|o| o.cwd.as_str()).unwrap_or(
            self.service
                .npm_prefix
                .root()
                .to_str()
                .ok_or(DomainError::PermissionDenied)?,
        );
        let (root, name) = resolve(&self.service.writable, cwd)?;
        let pinned = root.pin_directory(&name)?;
        let mut argv = binary.arguments.clone();
        argv.extend(command.args.clone());
        if command.command == self.service.npm_command
            && command.args.iter().any(|a| a == "-g" || a == "--global")
        {
            argv.extend([
                "--prefix".into(),
                self.service
                    .npm_prefix
                    .root()
                    .to_str()
                    .ok_or(DomainError::PermissionDenied)?
                    .into(),
            ]);
        }
        let timeout = command
            .options
            .as_ref()
            .and_then(|o| o.timeout_ms)
            .unwrap_or(if command.kind == "sync" {
                10000
            } else {
                300000
            });
        let deadline = self
            .control
            .deadline
            .min(Instant::now() + Duration::from_millis(timeout.min(300000)));
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        let spool_directory = self.service.spools.pin_directory("")?;
        let spool = spool_directory.process_cwd().join(format!(
            "package-{}-{}.log",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        use std::os::unix::fs::OpenOptionsExt;
        let stdout_path = spool.with_extension("stdout");
        let metadata_path = spool.with_extension("json");
        let mut output_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&stdout_path)
            .map_err(io_error)?;
        let mut metadata_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&metadata_path)
            .map_err(io_error)?;
        metadata_file.write_all(json!({"operation":self.operation,"workspace":self.service.workspace.id().as_str(),"command":command}).to_string().as_bytes()).map_err(io_error)?;
        metadata_file.sync_all().map_err(io_error)?;
        let mut child = self.service.processes.spawn_protocol(
            ProcessSpec {
                executable: PathBuf::from(format!(
                    "/proc/{}/fd/{}",
                    std::process::id(),
                    binary.file.as_raw_fd()
                )),
                argv,
                cwd: pinned.process_cwd(),
                env: self.service.env.clone(),
                timeout_ms: deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .max(1) as u64,
                grace_ms: 100,
                resident: false,
            },
            &spool,
        )?;
        // Arbitrary trusted package code can mutate even on exit failure. A
        // capture command is read-only by the manager contract, not a sandbox.
        self.effect = true;
        self.pending_output = true;
        child.close_input();
        let mut stdout = vec![];
        let mut truncated = false;
        let capture = command.kind != "run";
        let limit = if capture { 65536 } else { OUTPUT_BYTES };
        let mut terminal = loop {
            let bytes = child.read_stdout(32768)?;
            let keep = bytes.len().min(limit - stdout.len());
            stdout.extend_from_slice(&bytes[..keep]);
            output_file.write_all(&bytes[..keep]).map_err(io_error)?;
            truncated |= keep != bytes.len();
            if child.tail(0)?.truncated {
                truncated = true;
            }
            let stop = self.control.check().and_then(|_| {
                if Instant::now() >= deadline {
                    Err(DomainError::TimedOut)
                } else {
                    Ok(())
                }
            });
            if truncated || stop.is_err() {
                child.stop()?;
                break if truncated {
                    Err(DomainError::LimitExceeded("package output".into()))
                } else {
                    stop
                };
            }
            if child.try_wait()?.is_some() {
                // Drain bytes already in stdout after the owned child receipt.
                loop {
                    let bytes = child.read_stdout(32768)?;
                    if bytes.is_empty() {
                        break;
                    }
                    let keep = bytes.len().min(limit - stdout.len());
                    stdout.extend_from_slice(&bytes[..keep]);
                    output_file.write_all(&bytes[..keep]).map_err(io_error)?;
                    truncated |= keep != bytes.len();
                }
                break match child.completion() {
                    Some(ProcessCompletion::Exited(0)) => Ok(()),
                    Some(ProcessCompletion::TimedOut) => Err(DomainError::TimedOut),
                    value => Err(DomainError::Io(format!(
                        "package command failed: {value:?}"
                    ))),
                };
            }
            thread::sleep(Duration::from_millis(5));
        };
        let log = child.tail(0)?;
        truncated |= log.truncated;
        let mut stderr = Vec::with_capacity(log.disk_bytes);
        while stderr.len() < log.disk_bytes {
            let bytes = child.log_page(stderr.len() as u64, 32768)?;
            if bytes.is_empty() {
                return Err(DomainError::DurabilityUnknown);
            }
            stderr.extend_from_slice(&bytes);
        }
        if truncated {
            terminal = Err(DomainError::LimitExceeded("package output".into()));
        }
        let output = json!({"version":1,"command":command,"stdoutBase64":STANDARD.encode(&stdout),"stderrBase64":STANDARD.encode(&stderr),"truncated":truncated,"termination":terminal.as_ref().err().map(|e|format!("{e:?}"))});
        output_file.sync_all().map_err(io_error)?;
        let artifact = self.artifacts.put_evidence(
            &output.to_string(),
            "package-command-output",
            &json!({"type":"tool","name":"managePackage","operation":self.operation}),
            &[],
            None,
        )?;
        self.commands.push(json!({"artifact":artifact,"command":command.command,"kind":command.kind,"truncated":truncated,"succeeded":terminal.is_ok()}));
        self.pending_output = false;
        drop(child);
        // Retain the spool if output persistence failed; clean only acknowledged output.
        std::fs::remove_file(&spool).map_err(io_error)?;
        std::fs::remove_file(&stdout_path).map_err(io_error)?;
        std::fs::remove_file(&metadata_path).map_err(io_error)?;
        pinned.verify_identity()?;
        terminal?;
        if capture {
            let value = String::from_utf8(stdout)
                .map_err(|_| DomainError::InvalidInput("package command UTF-8".into()))?;
            if command.command == self.service.npm_command
                && command.args.windows(2).any(|a| a == ["root", "-g"])
                && Path::new(value.trim())
                    != self.service.npm_prefix.root().join("lib/node_modules")
            {
                return Err(DomainError::PermissionDenied);
            }
            Ok(value)
        } else {
            Ok(String::new())
        }
    }
}
