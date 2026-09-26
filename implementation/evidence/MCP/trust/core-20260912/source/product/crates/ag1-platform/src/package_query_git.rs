//! Explicit Git query capability for installed packages. Git's repository-local
//! config and remote helpers are trusted code with host-account privileges. The
//! native owner grants exact installation roots and a held executable identity.
use crate::{
    io_error,
    package_execution::{Binary, BinaryIdentity, binary_identity},
    package_queries::{Evidence, GitQueries},
    process::{ProcessCompletion, ProcessSpec, ProcessSupervisor},
    resource_fs::ScanControl,
    workspace::Workspace,
};
use ag1_core::{DomainError, PortFuture};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
static SERIAL: AtomicU64 = AtomicU64::new(1);
const CAPTURE: usize = 65536;
pub struct NativeGit<E> {
    processes: Arc<ProcessSupervisor>,
    roots: Vec<Arc<Workspace>>,
    spools: Arc<Workspace>,
    binary: Binary,
    identity: BinaryIdentity,
    executable: File,
    env: BTreeMap<String, String>,
    evidence: E,
}
impl<E: Evidence> NativeGit<E> {
    pub fn new(
        processes: Arc<ProcessSupervisor>,
        roots: Vec<Arc<Workspace>>,
        spools: Arc<Workspace>,
        mut binary: Binary,
        mut env: BTreeMap<String, String>,
        evidence: E,
    ) -> Result<Self, DomainError> {
        if roots.is_empty()
            || roots.len() > 64
            || binary.arguments.len() > 64
            || binary
                .arguments
                .iter()
                .any(|a| a.len() > 8192 || a.contains('\0'))
            || env.len() > 64
            || env.iter().any(|(k, v)| {
                k.len() > 256 || v.len() > 8192 || k.contains(['\0', '=']) || v.contains('\0')
            })
        {
            return Err(DomainError::InvalidInput("package Git capability".into()));
        }
        for root in &roots {
            root.verify_root()?;
        }
        spools.verify_root()?;
        binary.executable = binary.executable.canonicalize().map_err(io_error)?;
        let identity = binary_identity(&binary.executable)?;
        let executable = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&binary.executable)
            .map_err(io_error)?;
        let held = executable.metadata().map_err(io_error)?;
        if held.dev() != identity.0 || held.ino() != identity.1 {
            return Err(DomainError::Conflict);
        }
        env.extend([
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
            ("LC_ALL".into(), "C".into()),
        ]);
        Ok(Self {
            processes,
            roots,
            spools,
            binary,
            identity,
            executable,
            env,
            evidence,
        })
    }
}
impl<E: Evidence> GitQueries for NativeGit<E> {
    fn capture<'a>(
        &'a self,
        installed: &'a str,
        args: Vec<String>,
        remote: bool,
        deadline: Instant,
        cancel: &'a AtomicBool,
    ) -> PortFuture<'a, String> {
        Box::pin(async move {
            let control = ScanControl { cancel, deadline };
            control.check()?;
            // Revalidate here as well: callers can hold this capability directly.
            let allowed = if remote {
                args.len() == 3
                    && args[0] == "ls-remote"
                    && args[1] == "origin"
                    && (args[2] == "HEAD" || args[2].starts_with("refs/heads/"))
            } else {
                args == ["rev-parse", "HEAD"]
                    || args == ["rev-parse", "--abbrev-ref", "@{upstream}"]
            };
            if !allowed
                || args
                    .iter()
                    .any(|a| a.len() > 8192 || a.contains(['\0', '\n', '\r']))
            {
                return Err(DomainError::PermissionDenied);
            }
            let root = self
                .roots
                .iter()
                .find(|root| root.root().as_os_str() == installed)
                .ok_or(DomainError::PermissionDenied)?;
            root.verify_root()?;
            if binary_identity(&self.binary.executable)? != self.identity {
                return Err(DomainError::Conflict);
            }
            let pinned = root.pin_directory("")?;
            let spool_dir = self.spools.pin_directory("")?;
            let name = format!(
                "package-query-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            );
            let spool = spool_dir.process_cwd().join(format!("{name}.stderr"));
            let stdout_path = spool.with_extension("stdout");
            let metadata_path = spool.with_extension("json");
            let metadata = json!({"installed":installed,"args":args,"remote":remote,"stdout":stdout_path.file_name(),"stderr":spool.file_name()});
            let mut metadata_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&metadata_path)
                .map_err(io_error)?;
            metadata_file
                .write_all(metadata.to_string().as_bytes())
                .map_err(io_error)?;
            metadata_file.sync_all().map_err(io_error)?;
            let mut output_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&stdout_path)
                .map_err(io_error)?;
            let mut argv = self.binary.arguments.clone();
            argv.extend(args);
            let mut child = self.processes.spawn_protocol(
                ProcessSpec {
                    executable: PathBuf::from(format!(
                        "/proc/{}/fd/{}",
                        std::process::id(),
                        self.executable.as_raw_fd()
                    )),
                    argv,
                    cwd: pinned.process_cwd(),
                    env: self.env.clone(),
                    timeout_ms: deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .clamp(1, 10000) as u64,
                    grace_ms: 100,
                    resident: false,
                },
                &spool,
            )?;
            child.close_input();
            let mut stdout = Vec::new();
            let result = loop {
                let bytes = child.read_stdout(32768)?;
                let keep = bytes.len().min(CAPTURE - stdout.len());
                stdout.extend_from_slice(&bytes[..keep]);
                output_file.write_all(&bytes[..keep]).map_err(io_error)?;
                let view = child.tail(0)?;
                if keep != bytes.len() || view.truncated || view.total_seen > CAPTURE as u64 {
                    child.stop()?;
                    break Err(DomainError::LimitExceeded(
                        "package Git query output".into(),
                    ));
                }
                if let Err(error) = control.check() {
                    child.stop()?;
                    break Err(error);
                }
                if child.try_wait()?.is_some() {
                    loop {
                        let bytes = child.read_stdout(32768)?;
                        if bytes.is_empty() {
                            break;
                        }
                        let keep = bytes.len().min(CAPTURE - stdout.len());
                        stdout.extend_from_slice(&bytes[..keep]);
                        output_file.write_all(&bytes[..keep]).map_err(io_error)?;
                        if keep != bytes.len() {
                            return Err(DomainError::LimitExceeded(
                                "package Git query output".into(),
                            ));
                        }
                    }
                    break match child.completion() {
                        Some(ProcessCompletion::Exited(0)) => Ok(()),
                        Some(ProcessCompletion::TimedOut) => Err(DomainError::TimedOut),
                        _ => Err(DomainError::Unavailable("package Git query failed".into())),
                    };
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            };
            output_file.sync_all().map_err(io_error)?;
            let view = child.tail(0)?;
            if view.truncated || view.total_seen > CAPTURE as u64 {
                return Err(DomainError::LimitExceeded(
                    "package Git query diagnostics".into(),
                ));
            }
            let mut stderr = Vec::new();
            while stderr.len() < view.disk_bytes {
                let bytes = child.log_page(stderr.len() as u64, 32768)?;
                if bytes.is_empty() {
                    return Err(DomainError::DurabilityUnknown);
                }
                stderr.extend(bytes);
            }
            let metadata = json!({"request":metadata,"completion":format!("{result:?}")});
            self.evidence
                .capture("package_git_stdout", metadata.clone(), &stdout)
                .map_err(|_| DomainError::DurabilityUnknown)?;
            self.evidence
                .capture("package_git_stderr", metadata, &stderr)
                .map_err(|_| DomainError::DurabilityUnknown)?;
            pinned.verify_identity()?;
            spool_dir.verify_identity()?;
            drop(child);
            drop(output_file);
            drop(metadata_file);
            for path in [stdout_path, spool, metadata_path] {
                std::fs::remove_file(path).map_err(io_error)?;
            }
            result?;
            Ok(String::from_utf8_lossy(&stdout).into_owned())
        })
    }
}
