//! Owned shell execution. Host configuration is immutable and model arguments cannot choose
//! executables, environment, sandbox mode, quotas, or process identities.
use crate::{
    process::{MAX_LOG_MEMORY, OwnedProcess, ProcessSpec, ProcessSupervisor},
    workspace::{PinnedDirectory, Workspace},
};
use ag1_core::{
    DomainError,
    context::ContextArtifactPort,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Registry, Scope, ValidatedCall},
        shell::{self, Exit, FullOutput},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sandbox {
    Off,
    Workspace,
    Strict,
}
impl Sandbox {
    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value?.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "workspace" => Some(Self::Workspace),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }
}
#[derive(Clone, Serialize)]
pub struct ShellConfig {
    pub executable: PathBuf,
    pub env: BTreeMap<String, String>,
    pub command_prefix: Option<String>,
    pub command_template: Option<String>,
    pub sandbox: Sandbox,
    pub bubblewrap: Option<PathBuf>,
    pub output_bytes: usize,
    pub address_space: Option<u64>,
}
impl ShellConfig {
    /// Resolve only the environment already admitted by the native host. An
    /// explicit mode wins; no ambient process environment is consulted.
    pub fn with_sandbox_from_environment(mut self, explicit: Option<Sandbox>) -> Self {
        self.sandbox = explicit
            .or_else(|| Sandbox::parse(self.env.get("AG1_BASH_SANDBOX").map(String::as_str)))
            .unwrap_or(Sandbox::Off);
        if self.command_template.is_none() {
            self.command_template = self
                .env
                .get("AG1_BASH_SANDBOX_COMMAND")
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
        }
        self
    }
}
pub struct ShellService {
    workspace: Arc<Workspace>,
    processes: Arc<ProcessSupervisor>,
    spools: PathBuf,
    config: ShellConfig,
    network: bool,
    stamp: String,
}
pub struct PreparedShell {
    call: ValidatedCall,
    scope: Scope,
    pub(crate) command: String,
    timeout: f64,
    stamp: String,
}
impl PreparedShell {
    pub fn call(&self) -> &ValidatedCall {
        &self.call
    }
    pub fn target_revision(&self) -> &str {
        &self.stamp
    }
}
pub struct ShellExecution<'a, F: Fn() -> u64> {
    pub registry: &'a Registry,
    pub scope: &'a Scope,
    pub policy_revision: u64,
    pub now: F,
    pub cancel: &'a AtomicBool,
    pub deadline: Instant,
}
pub(crate) struct RunningShell {
    pub(crate) child: OwnedProcess,
    _pinned: PinnedDirectory,
    spool: PathBuf,
    command: String,
    deadline: Instant,
    timeout: f64,
}
impl Drop for RunningShell {
    fn drop(&mut self) {
        let _ = self.child.stop();
        let _ = std::fs::remove_file(&self.spool);
    }
}
impl ShellService {
    pub fn new(
        workspace: Arc<Workspace>,
        processes: Arc<ProcessSupervisor>,
        spools: &Path,
        config: ShellConfig,
    ) -> Result<Self, DomainError> {
        Self::new_with_network(workspace, processes, spools, config, true)
    }
    /// Strict mode always isolates networking, including when this flag is true.
    /// Workspace mode may additionally disable networking through host policy.
    pub fn new_with_network(
        workspace: Arc<Workspace>,
        processes: Arc<ProcessSupervisor>,
        spools: &Path,
        config: ShellConfig,
        network: bool,
    ) -> Result<Self, DomainError> {
        if !config.executable.is_absolute()
            || !spools.is_absolute()
            || !spools.is_dir()
            || config.env.len() > 128
            || !(1024..=64 * 1024 * 1024).contains(&config.output_bytes)
            || config.bubblewrap.as_ref().is_some_and(|p| !p.is_absolute())
        {
            return Err(DomainError::InvalidInput("host shell configuration".into()));
        }
        let bytes = serde_json::to_vec(&(&config, network))
            .map_err(|_| DomainError::InvalidInput("shell config".into()))?;
        if bytes.len() > 32 * 1024 {
            return Err(DomainError::LimitExceeded("shell configuration".into()));
        }
        let stamp = format!(
            "shell:{}:{:x}",
            workspace.id().as_str(),
            Sha256::digest(bytes)
        );
        Ok(Self {
            workspace,
            processes,
            spools: spools.into(),
            config,
            network,
            stamp,
        })
    }
    pub(crate) fn target_revision(&self) -> &str {
        &self.stamp
    }
    pub(crate) fn workspace(&self) -> &Arc<Workspace> {
        &self.workspace
    }
    pub fn prepare_job(
        &self,
        call: &ValidatedCall,
        registry: &Registry,
        scope: &Scope,
    ) -> Result<PreparedShell, DomainError> {
        if call.name() != "job" || call.args()["action"] != "start" {
            return Err(DomainError::PermissionDenied);
        }
        self.prepare_command(call, registry, scope, true)
    }
    /// Prefix/template changes are prepared BEFORE policy. Approvals bind the actual command.
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        registry: &Registry,
        scope: &Scope,
    ) -> Result<PreparedShell, DomainError> {
        if call.name() != "bash" {
            return Err(DomainError::PermissionDenied);
        }
        self.prepare_command(call, registry, scope, false)
    }
    fn prepare_command(
        &self,
        call: &ValidatedCall,
        registry: &Registry,
        scope: &Scope,
        job: bool,
    ) -> Result<PreparedShell, DomainError> {
        if scope.workspace != self.workspace.id().as_str() || call.origin() != Origin::BuiltIn {
            return Err(DomainError::PermissionDenied);
        }
        let input = call.args()["command"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("shell command".into()))?;
        shell::guard(input, job)?;
        if self.config.sandbox != Sandbox::Off
            && !self.config.bubblewrap.as_ref().is_some_and(|p| p.is_file())
        {
            return Err(DomainError::Unavailable(
                "sandbox requested but bubblewrap is unavailable".into(),
            ));
        }
        let mut command = if let Some(prefix) = &self.config.command_prefix {
            format!("{prefix}\n{input}")
        } else {
            input.into()
        };
        if let Some(template) = &self.config.command_template {
            command = shell::sandbox_template(
                template,
                &command,
                self.workspace
                    .root()
                    .to_str()
                    .ok_or(DomainError::PermissionDenied)?,
            )?;
        }
        shell::guard(&command, job)?;
        let timeout = call
            .args()
            .get("timeout")
            .map(|v| {
                v.as_f64()
                    .ok_or_else(|| DomainError::InvalidInput("shell timeout".into()))
            })
            .transpose()?
            .unwrap_or(300.);
        if !timeout.is_finite() || timeout <= 0. || timeout > 86400. {
            return Err(DomainError::InvalidInput(
                "shell timeout must be positive and at most one day".into(),
            ));
        }
        let mut args = call.args().clone();
        args["command"] = json!(command);
        let call = registry.prepare(call.operation(), call.call_id(), call.name(), &args, true)?;
        Ok(PreparedShell {
            call,
            scope: scope.clone(),
            command,
            timeout,
            stamp: self.stamp.clone(),
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedShell,
        authorization: AuthorizedCall,
        execution: ShellExecution<'_, F>,
        artifacts: &mut impl ContextArtifactPort,
        mut on_update: impl FnMut(Value),
    ) -> Result<AdapterOutcome, DomainError> {
        let ShellExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if scope != &plan.scope
            || plan.stamp != self.stamp
            || authorization.call().operation() != plan.call.operation()
            || authorization.call().args_hash() != plan.call.args_hash()
        {
            return Err(DomainError::Conflict);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &plan.stamp,
                now: now(),
            },
        )?;
        let mut running = self.start(&plan.command, plan.timeout, deadline)?;
        let mut next_update = Instant::now();
        let exit = loop {
            if let Some(exit) = running.poll(cancel)? {
                break exit;
            }
            if Instant::now() >= next_update {
                let tail = running.child.tail(MAX_LOG_MEMORY)?;
                let text = shell::sanitize(&tail.bytes)?;
                let view = ag1_core::tools::truncate::truncate(&text, Default::default(), true);
                on_update(
                    json!({"content":[{"type":"text","text":view.content}],"details":{"partial":true,"observedBytes":tail.total_seen}}),
                );
                next_update = Instant::now() + Duration::from_millis(100);
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut outcome = running.capture(exit, call.call_id(), "bash", artifacts)?;
        if matches!(exit, Exit::Code(0)) {
            let text = outcome.result["content"][0]["text"].as_str().unwrap_or("");
            let note = self.persist_note(&running.command, text);
            if !note.is_empty() {
                outcome.result["content"][0]["text"] = json!(format!("{text}\n\n{note}"));
            }
        }
        Ok(outcome)
    }
    fn persist_note(&self, command: &str, output: &str) -> String {
        static CUTOFF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?i)here-document(?: at line [0-9]+)? delimited by end-of-file")
                .expect("audited Bash diagnostic pattern")
        });
        let paths = crate::file_tools::FileService::new(self.workspace.clone());
        let mut notes = vec![];
        if command.contains("<<") && CUTOFF.is_match(output) {
            notes.push("Warning: here-document was cut off. Re-emit the write with the write tool and complete arguments instead of a bash heredoc.".into());
        }
        for target in shell::write_targets(command).into_iter().take(128) {
            let Ok(path) = paths.relative_path(&target) else {
                continue;
            };
            match self.workspace.snapshot(&path, crate::workspace::MAX_READ) {
                Ok(snapshot) => {
                    let lines = if snapshot.bytes.is_empty() {
                        0
                    } else {
                        snapshot.bytes.iter().filter(|b| **b == b'\n').count() + 1
                    };
                    notes.push(format!(
                        "Persisted {}B ({lines} line{}) at {target}.",
                        snapshot.bytes.len(),
                        if lines == 1 { "" } else { "s" }
                    ));
                }
                Err(DomainError::NotFound) => notes.push(format!("No file landed at {target}.")),
                _ => {}
            }
        }
        notes.join("\n")
    }
    pub(crate) fn start(
        &self,
        command: &str,
        timeout: f64,
        deadline: Instant,
    ) -> Result<RunningShell, DomainError> {
        let pinned = self.workspace.pin_directory("")?;
        let deadline = deadline.min(Instant::now() + Duration::from_secs_f64(timeout));
        let mut env = self.config.env.clone();
        // No interactive/login startup files or inherited environment are evaluated.
        env.remove("BASH_ENV");
        env.remove("ENV");
        let mut executable = self.config.executable.clone();
        let mut argv = vec![
            "--noprofile".into(),
            "--norc".into(),
            "-o".into(),
            "pipefail".into(),
            "-c".into(),
            command.into(),
        ];
        if self.config.sandbox != Sandbox::Off {
            executable = self
                .config
                .bubblewrap
                .clone()
                .filter(|p| p.is_file())
                .ok_or_else(|| {
                    DomainError::Unavailable(
                        "sandbox requested but bubblewrap is unavailable".into(),
                    )
                })?;
            let sandbox_path = sandbox_path(
                self.workspace.root(),
                self.config.env.get("PATH").map(String::as_str),
            );
            let cwd = self
                .workspace
                .root()
                .to_str()
                .ok_or(DomainError::PermissionDenied)?;
            let mut sandbox: Vec<String> = ["--unshare-ipc", "--unshare-pid", "--unshare-user"]
                .map(str::to_string)
                .into();
            if self.config.sandbox == Sandbox::Strict || !self.network {
                sandbox.push("--unshare-net".into());
            }
            // The supervisor passes an owned cwd descriptor through the private helper.
            // --bind-fd stays valid after user/pid namespaces hide the parent's /proc.
            sandbox.extend(
                [
                    "--die-with-parent",
                    "--ro-bind",
                    "/",
                    "/",
                    "--tmpfs",
                    "/tmp",
                    "--proc",
                    "/proc",
                    "--dev",
                    "/dev",
                    "--dir",
                    "/tmp/ag1-home",
                    "--bind-fd",
                    crate::process::PINNED_CWD_FD,
                    cwd,
                    "--setenv",
                    "HOME",
                    "/tmp/ag1-home",
                    "--setenv",
                    "PATH",
                    &sandbox_path,
                    "--chdir",
                    cwd,
                    "--",
                    self.config
                        .executable
                        .to_str()
                        .ok_or(DomainError::PermissionDenied)?,
                ]
                .map(str::to_string),
            );
            sandbox.append(&mut argv);
            argv = sandbox;
        }
        pinned.verify_identity()?;
        let spool = self.spools.join(format!(
            "shell-{}-{}.log",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let spec = ProcessSpec {
            executable,
            argv,
            cwd: pinned.process_cwd(),
            env,
            timeout_ms: deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .clamp(1, 86_400_000) as u64,
            grace_ms: 100,
            resident: false,
        };
        let mut child = if self.config.sandbox == Sandbox::Off {
            self.processes.spawn_tool(
                spec,
                &spool,
                self.config.output_bytes,
                self.config.address_space,
            )?
        } else {
            self.processes.spawn_tool_with_cwd(
                spec,
                &spool,
                self.config.output_bytes,
                self.config.address_space,
            )?
        };
        child.close_input();
        Ok(RunningShell {
            child,
            _pinned: pinned,
            spool,
            command: command.into(),
            deadline,
            timeout,
        })
    }
}
impl RunningShell {
    pub(crate) fn poll(&mut self, cancel: &AtomicBool) -> Result<Option<Exit>, DomainError> {
        if cancel.load(Ordering::Acquire) {
            self.child.stop()?;
            return Ok(Some(Exit::Cancelled));
        }
        if Instant::now() >= self.deadline {
            self.child.stop()?;
            return Ok(Some(Exit::Timeout(self.timeout)));
        }
        if self.child.tail(0)?.truncated {
            self.child.stop()?;
            return Ok(Some(Exit::OutputLimit));
        }
        if let Some(status) = self.child.try_wait()? {
            // A fast producer can cross its quota just before exit.
            if self.child.tail(0)?.truncated {
                return Ok(Some(Exit::OutputLimit));
            }
            return Ok(Some(match self.child.completion() {
                Some(crate::process::ProcessCompletion::Exited(code)) => Exit::Code(code),
                Some(crate::process::ProcessCompletion::Signaled(_)) => Exit::Signal,
                Some(crate::process::ProcessCompletion::TimedOut) => Exit::Timeout(self.timeout),
                None => status.code().map(Exit::Code).unwrap_or(Exit::Signal),
            }));
        }
        Ok(None)
    }
    pub(crate) fn capture(
        &mut self,
        exit: Exit,
        call_id: &str,
        tool: &str,
        artifacts: &mut impl ContextArtifactPort,
    ) -> Result<AdapterOutcome, DomainError> {
        let view = self.child.tail(0)?;
        let mut raw = Vec::with_capacity(view.disk_bytes);
        while raw.len() < view.disk_bytes {
            let chunk = self.child.log_page(raw.len() as u64, 32768)?;
            if chunk.is_empty() {
                return Err(DomainError::DurabilityUnknown);
            }
            raw.extend(chunk);
        }
        let complete = !view.truncated;
        let encoded = json!({"version":1,"encoding":"base64","bytes":raw.len(),"observedBytes":view.total_seen,"complete":complete,"data":STANDARD.encode(&raw)});
        let original = artifacts.put_evidence(
            &encoded.to_string(),
            "terminal-raw-output",
            &json!({"type":"tool","name":tool,"callId":call_id}),
            &[],
            None,
        )?;
        let sanitized = shell::sanitize(&raw)?;
        let full = if raw.len() > 50 * 1024 || sanitized.split('\n').count() > 40 {
            Some(artifacts.put_evidence(
                &sanitized,
                "terminal-output",
                &json!({"type":"tool","name":tool,"callId":call_id}),
                &[],
                Some(&original.id),
            )?)
        } else {
            None
        };
        let reference = full.as_ref().map(|a| FullOutput {
            id: &a.id,
            bytes: a.bytes,
            sha256: &a.sha256,
        });
        let mut result = shell::render(&self.command, &raw, exit, reference)?;
        if result.get("details").is_none() {
            result["details"] = json!({});
        }
        if tool == "job" {
            result["details"]["logTail"] = json!(
                ag1_core::tools::truncate::truncate(
                    &sanitized,
                    ag1_core::tools::truncate::Limits {
                        max_lines: 40,
                        max_bytes: 50 * 1024
                    },
                    true
                )
                .content
            );
        }
        result["details"]["rawOutputArtifact"] = json!(original);
        result["details"]["fullOutputRetained"] = json!(complete);
        result["details"]["observedBytes"] = json!(view.total_seen);
        result["details"]["termination"] = json!(match exit {
            Exit::Code(_) => "exit",
            Exit::Signal => "signal",
            Exit::Cancelled => "cancelled",
            Exit::Timeout(_) => "timeout",
            Exit::OutputLimit => "output_limit",
        });
        // Shell commands may have changed files even after an error or cancellation.
        Ok(AdapterOutcome {
            result,
            effect: Effect::Unknown,
            artifact_page: false,
        })
    }
}

/// Preserve explicit external toolchain directories while excluding workspace-controlled PATH entries.
pub fn sandbox_path(workspace: &Path, configured: Option<&str>) -> String {
    let mut paths = vec![];
    for raw in configured
        .unwrap_or("")
        .split(':')
        .chain("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".split(':'))
    {
        let path = Path::new(raw);
        if !path.is_absolute() {
            continue;
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                _ => normalized.push(component.as_os_str()),
            }
        }
        let canonical = normalized
            .canonicalize()
            .unwrap_or_else(|_| normalized.clone());
        if canonical.starts_with(workspace) {
            continue;
        }
        let Some(value) = normalized.to_str() else {
            continue;
        };
        if !paths.iter().any(|p| p == value) {
            paths.push(value.to_owned());
        }
    }
    paths.join(":")
}
