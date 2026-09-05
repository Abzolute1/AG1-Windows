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
        workspace.verify_root()?;
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
                cwd: workspace.root().to_path_buf(),
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
