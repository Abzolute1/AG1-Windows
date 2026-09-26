use crate::{
    io_error,
    process::ProcessSupervisor,
    workspace::{MAX_FILES, MAX_READ, Workspace},
};
use ag1_core::{DomainError, FileRevision};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};

#[path = "git_read.rs"]
pub(crate) mod readonly;
const MAX_GIT_BYTES: usize = 1024 * 1024;
pub struct GitService {
    supervisor: ProcessSupervisor,
    executable: PathBuf,
    spools: PathBuf,
    bwrap: PathBuf,
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
            bwrap: "/usr/bin/bwrap".into(),
        })
    }
    pub fn with_bubblewrap(mut self, path: &Path) -> Self {
        self.bwrap = path.into();
        self
    }
    fn run(&self, workspace: &Workspace, args: &[String]) -> Result<Vec<u8>, DomainError> {
        let cancel = AtomicBool::new(false);
        let (output, ok) = readonly::run(
            &self.supervisor,
            &self.executable,
            &self.bwrap,
            &self.spools,
            workspace,
            args,
            &crate::resource_fs::ScanControl {
                cancel: &cancel,
                deadline: std::time::Instant::now() + Duration::from_secs(10),
            },
            MAX_GIT_BYTES,
        )?;
        if !ok {
            return Err(DomainError::Io("Contained Git command failed".into()));
        }
        Ok(output)
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
        self.files_bounded(workspace, MAX_FILES)
    }
    pub(crate) fn files_bounded(
        &self,
        workspace: &Workspace,
        limit: usize,
    ) -> Result<Vec<String>, DomainError> {
        if limit == 0 || limit > 10000 {
            return Err(DomainError::LimitExceeded("Git file inventory".into()));
        }
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
            if paths.len() >= limit {
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
        let (output, ok) = readonly::run(
            &self.supervisor,
            &self.executable,
            &self.bwrap,
            &self.spools,
            workspace,
            args,
            &crate::resource_fs::ScanControl { cancel, deadline },
            max_bytes,
        )?;
        Ok((
            String::from_utf8(output)
                .map_err(|_| DomainError::InvalidInput("UTF-8 Git output".into()))?,
            ok,
        ))
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
