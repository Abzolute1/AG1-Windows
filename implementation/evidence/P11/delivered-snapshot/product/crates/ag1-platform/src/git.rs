use crate::{
    io_error,
    process::{ProcessSpec, ProcessSupervisor},
    workspace::{MAX_FILES, MAX_READ, Workspace},
};
use ag1_core::{DomainError, FileRevision};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static SERIAL: AtomicU64 = AtomicU64::new(1);
const MAX_GIT_BYTES: usize = 1024 * 1024;
pub struct GitService {
    supervisor: ProcessSupervisor,
    executable: PathBuf,
    spools: PathBuf,
}
#[derive(Clone, Debug)]
pub struct GitView {
    pub head: String,
    pub worktree: String,
    pub status: Vec<u8>,
    pub diff: Vec<u8>,
    pub conflicted: bool,
}
impl GitService {
    pub fn new(helper: &Path, git: &Path, spools: &Path) -> Result<Self, DomainError> {
        if !spools.is_dir() {
            return Err(DomainError::NotFound);
        }
        Ok(Self {
            supervisor: ProcessSupervisor::new(helper)?,
            executable: git.canonicalize().map_err(io_error)?,
            spools: spools.canonicalize().map_err(io_error)?,
        })
    }
    fn run(&self, workspace: &Workspace, args: &[String]) -> Result<Vec<u8>, DomainError> {
        let pinned = workspace.pin_directory("")?;
        let spool = self.spools.join(format!(
            "git-{}-{}.log",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let mut argv: Vec<String> = [
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "--no-pager",
        ]
        .map(String::from)
        .to_vec();
        argv.extend_from_slice(args);
        let env = BTreeMap::from([
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
            ("LC_ALL".into(), "C".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ]);
        let mut child = self.supervisor.spawn(
            ProcessSpec {
                resident: false,
                executable: self.executable.clone(),
                argv,
                cwd: pinned.process_cwd(),
                env,
                timeout_ms: 10_000,
                grace_ms: 100,
            },
            &spool,
        )?;
        let status = child.wait(Duration::from_secs(12));
        let result = (|| {
            if !status?.success() {
                return Err(DomainError::Io(
                    "Git command failed; isolated spool retained".into(),
                ));
            }
            let view = child.tail(0)?;
            if view.truncated || view.total_seen > MAX_GIT_BYTES as u64 {
                return Err(DomainError::LimitExceeded("Git output".into()));
            }
            pinned.verify_identity()?;
            let mut bytes = vec![];
            while bytes.len() < view.disk_bytes {
                bytes.extend(child.log_page(bytes.len() as u64, 32 * 1024)?);
            }
            Ok(bytes)
        })();
        drop(child);
        if result.is_ok() {
            std::fs::remove_file(spool).map_err(io_error)?;
        }
        result
    }
    fn head_unlocked(&self, workspace: &Workspace) -> Result<String, DomainError> {
        let bytes = self.run(
            workspace,
            &["rev-parse".into(), "--verify".into(), "HEAD".into()],
        )?;
        let head = String::from_utf8(bytes)
            .map_err(|_| DomainError::InvalidInput("Git HEAD".into()))?
            .trim()
            .to_owned();
        validate_oid(&head)?;
        Ok(head)
    }
    pub fn status_diff(
        &self,
        workspace: &Workspace,
        expected_head: Option<&str>,
    ) -> Result<GitView, DomainError> {
        let _claim = workspace.claim()?;
        self.status_diff_unlocked(workspace, expected_head)
    }
    fn status_diff_unlocked(
        &self,
        workspace: &Workspace,
        expected_head: Option<&str>,
    ) -> Result<GitView, DomainError> {
        let head = self.head_unlocked(workspace)?;
        if expected_head.is_some_and(|expected| expected != head) {
            return Err(DomainError::Conflict);
        }
        let status = self.run(
            workspace,
            &[
                "status".into(),
                "--porcelain=v1".into(),
                "-z".into(),
                "--untracked-files=all".into(),
            ],
        )?;
        let diff = self.run(
            workspace,
            &[
                "diff".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                "HEAD".into(),
                "--".into(),
            ],
        )?;
        let git_dir = self.run(
            workspace,
            &["rev-parse".into(), "--absolute-git-dir".into()],
        )?;
        let conflicted = status.split(|b| *b == 0).any(|line| {
            line.len() >= 2
                && ([b"DD", b"AU", b"UD", b"UA", b"DU", b"AA", b"UU"]
                    .iter()
                    .any(|code| line.starts_with(*code)))
        });
        if self.head_unlocked(workspace)? != head {
            return Err(DomainError::Conflict);
        }
        let worktree = format!("git-{:x}", Sha256::digest(git_dir));
        Ok(GitView {
            head,
            worktree,
            status,
            diff,
            conflicted,
        })
    }
    pub fn files(&self, workspace: &Workspace) -> Result<Vec<String>, DomainError> {
        let _claim = workspace.claim()?;
        let bytes = self.run(
            workspace,
            &[
                "ls-files".into(),
                "-z".into(),
                "--cached".into(),
                "--others".into(),
                "--exclude-standard".into(),
            ],
        )?;
        let mut paths = vec![];
        for bytes in bytes
            .split(|byte| *byte == 0)
            .filter(|bytes| !bytes.is_empty())
        {
            if paths.len() >= MAX_FILES {
                return Err(DomainError::LimitExceeded("Git file count".into()));
            }
            paths.push(
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| DomainError::InvalidInput("non-UTF8 Git path".into()))?,
            );
        }
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
    /// Read a restore candidate only from a clean repository and a pinned commit. Publication still uses CAS.
    pub fn restore_candidate(
        &self,
        workspace: &Workspace,
        path: &str,
        expected_head: &str,
        expected: &FileRevision,
        source_commit: &str,
    ) -> Result<Vec<u8>, DomainError> {
        validate_oid(source_commit)?;
        let view = self.status_diff(workspace, Some(expected_head))?;
        if view.conflicted
            || !view.status.is_empty()
            || workspace.snapshot(path, MAX_READ)?.revision != *expected
        {
            return Err(DomainError::Conflict);
        }
        if path.contains(['\0', '\n', ':'])
            || path.starts_with('/')
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
        {
            return Err(DomainError::InvalidInput("restore path".into()));
        }
        let bytes = self.run(
            workspace,
            &["show".into(), format!("{source_commit}:{path}")],
        )?;
        // No git reset/checkout/restore mutation is hidden in this read operation.
        if self.head_unlocked(workspace)? != expected_head {
            return Err(DomainError::Conflict);
        }
        Ok(bytes)
    }
}
fn validate_oid(value: &str) -> Result<(), DomainError> {
    if ![40, 64].contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Err(DomainError::InvalidInput(
            "full Git object ID required".into(),
        ))
    } else {
        Ok(())
    }
}

/// P09 read-only diff transport. Arguments are assembled by the native tool adapter;
/// no shell or repository-configured external diff/text conversion is executed.
impl GitService {
    pub(crate) fn tool_diff(
        &self,
        workspace: &Workspace,
        tail: &[String],
        cancel: &std::sync::atomic::AtomicBool,
        deadline: std::time::Instant,
    ) -> Result<(String, bool), DomainError> {
        let mut args = [
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--relative",
        ]
        .map(str::to_owned)
        .to_vec();
        args.extend_from_slice(tail);
        self.tool_command(workspace, &args, cancel, deadline, 5 * 1024 * 1024)
    }
    /// Host-assembled read-only commands, sharing owned process drain and bounds.
    pub(crate) fn tool_command(
        &self,
        workspace: &Workspace,
        args: &[String],
        cancel: &std::sync::atomic::AtomicBool,
        deadline: std::time::Instant,
        max_bytes: usize,
    ) -> Result<(String, bool), DomainError> {
        let _claim = workspace.claim()?;
        let pinned = workspace.pin_directory("")?;
        let check = || {
            if cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if std::time::Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                Ok(())
            }
        };
        check()?;
        let mut argv: Vec<String> = [
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "--no-pager",
            "--literal-pathspecs",
        ]
        .map(str::to_string)
        .into();
        argv.extend_from_slice(args);
        let spool = self.spools.join(format!(
            "tool-git-{}-{}.log",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let mut child = self.supervisor.spawn_protocol_limited(
            ProcessSpec {
                executable: self.executable.clone(),
                argv,
                cwd: pinned.process_cwd(),
                env: BTreeMap::from([
                    ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
                    ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
                    ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
                    ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                    ("LC_ALL".into(), "C".into()),
                ]),
                timeout_ms: deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .as_millis()
                    .clamp(1, 86_400_000) as u64,
                grace_ms: 100,
                resident: false,
            },
            &spool,
            1024 * 1024 * 1024,
        )?;
        // Spool ownership is obtained only after successful create_new/spawn.
        struct SpoolCleanup(PathBuf);
        impl Drop for SpoolCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = SpoolCleanup(spool);
        child.close_input();
        let mut output = vec![];
        let status;
        loop {
            check()?;
            let bytes = child.read_stdout(32768)?;
            let empty = bytes.is_empty();
            output.extend(bytes);
            if output.len() > max_bytes {
                return Err(DomainError::LimitExceeded("Git command output".into()));
            }
            if empty && let Some(exit) = child.try_wait()? {
                status = exit;
                break;
            }
            if empty {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        pinned.verify_identity()?;
        check()?;
        if status.success() {
            Ok((
                String::from_utf8(output)
                    .map_err(|_| DomainError::InvalidInput("UTF-8 Git diff".into()))?,
                true,
            ))
        } else {
            Ok((
                String::from_utf8_lossy(&child.tail(65536)?.bytes)
                    .trim()
                    .into(),
                false,
            ))
        }
    }
}

/// A reviewed file-version preview. This cannot reset HEAD, session state or a
/// database. Source bytes and expected Git/file identities are bound together.
pub struct VersionPreview {
    pub path: String,
    pub head: String,
    pub source: String,
    pub before: Vec<u8>,
    pub candidate: Vec<u8>,
    expected: FileRevision,
}
impl GitService {
    pub fn history(
        &self,
        workspace: &Workspace,
        limit: usize,
    ) -> Result<Vec<(String, String)>, DomainError> {
        if limit == 0 || limit > 100 {
            return Err(DomainError::LimitExceeded("Git history".into()));
        }
        let _claim = workspace.claim()?;
        let bytes = self.run(
            workspace,
            &[
                "log".into(),
                format!("-{limit}"),
                "--format=%H%x00%s".into(),
            ],
        )?;
        let text = String::from_utf8(bytes)
            .map_err(|_| DomainError::InvalidInput("Git history UTF-8".into()))?;
        text.lines()
            .map(|l| {
                let (id, title) = l
                    .split_once('\0')
                    .ok_or_else(|| DomainError::InvalidInput("Git history record".into()))?;
                validate_oid(id)?;
                Ok((id.into(), title.chars().take(512).collect()))
            })
            .collect()
    }
    pub fn preview_version(
        &self,
        workspace: &Workspace,
        path: &str,
        head: &str,
        source: &str,
    ) -> Result<VersionPreview, DomainError> {
        let before = workspace.snapshot(path, MAX_READ)?;
        let candidate = self.restore_candidate(workspace, path, head, &before.revision, source)?;
        Ok(VersionPreview {
            path: path.into(),
            head: head.into(),
            source: source.into(),
            before: before.bytes,
            candidate,
            expected: before.revision,
        })
    }
    /// Explicit host-approved publication, with a durable caller-owned recovery
    /// record required before entry. All AG1 writers share Workspace's claim.
    pub fn apply_version(
        &self,
        workspace: &Workspace,
        preview: &VersionPreview,
    ) -> Result<FileRevision, DomainError> {
        let check = |temporary: Option<&str>| {
            let view = self.status_diff_unlocked(workspace, Some(&preview.head))?;
            // The atomic writer's own temporary file is not user dirt.
            let dirty = view
                .status
                .split(|b| *b == 0)
                .filter(|l| !l.is_empty())
                .any(|l| temporary.is_none_or(|path| l != format!("?? {path}").as_bytes()));
            if view.conflicted || dirty {
                return Err(DomainError::Conflict);
            }
            Ok(())
        };
        let result = workspace.replace_guarded(
            &preview.path,
            &preview.expected,
            &preview.candidate,
            check,
        )?;
        if self.head_unlocked(workspace)? != preview.head {
            return Err(DomainError::DurabilityUnknown);
        }
        Ok(result)
    }
}
