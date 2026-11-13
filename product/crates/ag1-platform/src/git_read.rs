//! Read-only Git in an owned networkless namespace. Linked worktree metadata
//! is accepted only with its native reciprocal back-pointer, never as a generic
//! grant to the directory containing another repository.
use crate::{
    io_error,
    process::{
        PINNED_CWD_FD, PINNED_DIRECTORY_PREFIX, ProcessSpec, ProcessSupervisor, READONLY_FILTER_FD,
    },
    resource_fs::ScanControl,
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::{ByteSnapshot, DomainError};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, DomainError>;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contained_git_starts_with_only_owned_workspace_metadata_and_system_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let status = std::process::Command::new("/usr/bin/git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap();
        assert!(status.success());
        let helper = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ag1-process-host");
        let supervisor = ProcessSupervisor::new(&helper).unwrap();
        let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let (bytes, ok) = run(
            &supervisor,
            Path::new("/usr/bin/git"),
            Path::new("/usr/bin/bwrap"),
            dir.path(),
            &workspace,
            &["status".into(), "--porcelain".into()],
            &ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(10),
            },
            65536,
        )
        .unwrap();
        assert!(ok, "{}", String::from_utf8_lossy(&bytes));
        assert!(bytes.is_empty());
    }
}
pub(crate) struct Metadata {
    pub(crate) git: Arc<Workspace>,
    pub(crate) common: Arc<Workspace>,
    pub(crate) links: Vec<(Arc<Workspace>, String, ByteSnapshot)>,
}
impl Metadata {
    pub(crate) fn namespace_git_dir(&self) -> String {
        if self.git.root() == self.common.root() {
            "/gitcommon".into()
        } else {
            format!(
                "/gitcommon/{}",
                self.git
                    .root()
                    .strip_prefix(self.common.root())
                    .expect("validated dedicated slot")
                    .display()
            )
        }
    }
}
fn text(snapshot: &ByteSnapshot) -> Result<&str> {
    std::str::from_utf8(&snapshot.bytes)
        .map(str::trim_end)
        .map_err(|_| DomainError::PermissionDenied)
}
pub(crate) fn metadata(workspace: &Workspace) -> Result<Metadata> {
    let service = WorkspaceService::new()?;
    if workspace.directory_exists(".git")? {
        let git = service.register(&workspace.root().join(".git"))?;
        if git.root() != workspace.root().join(".git") {
            return Err(DomainError::PermissionDenied);
        }
        return Ok(Metadata {
            common: git.clone(),
            git,
            links: vec![],
        });
    }
    let link = workspace.snapshot(".git", 4096)?;
    let target = text(&link)?
        .strip_prefix("gitdir: ")
        .filter(|s| !s.is_empty())
        .ok_or(DomainError::PermissionDenied)?;
    let git = service.register(&workspace.root().join(target))?;
    let common_link = git.snapshot("commondir", 4096)?;
    let common = service.register(&git.root().join(text(&common_link)?))?;
    // Git-created worktrees have a dedicated slot in this exact common store.
    if git.root().parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("worktrees"))
        || git.root().parent().and_then(Path::parent) != Some(common.root())
    {
        return Err(DomainError::PermissionDenied);
    }
    let back = git.snapshot("gitdir", 4096)?;
    let back_path = git
        .root()
        .join(text(&back)?)
        .canonicalize()
        .map_err(io_error)?;
    if back_path != workspace.root().join(".git") {
        return Err(DomainError::PermissionDenied);
    }
    let owner = workspace.for_conversation(workspace.id())?;
    let links = vec![
        (owner, ".git".into(), link),
        (git.clone(), "commondir".into(), common_link),
        (git.clone(), "gitdir".into(), back),
    ];
    Ok(Metadata { git, common, links })
}
pub(super) fn run(
    supervisor: &ProcessSupervisor,
    executable: &Path,
    bwrap: &Path,
    spools: &Path,
    workspace: &Workspace,
    args: &[String],
    control: &ScanControl<'_>,
    limit: usize,
) -> Result<(Vec<u8>, bool)> {
    control.check()?;
    if !executable.starts_with("/usr/") {
        return Err(DomainError::Unavailable(
            "Contained review needs a configured system Git executable".into(),
        ));
    }
    let metadata = metadata(workspace)?;
    let root = workspace.pin_directory("")?;
    let git = metadata.git.pin_directory("")?;
    let common = metadata.common.pin_directory("")?;
    let objects = metadata.common.pin_directory("objects")?;
    let optional_pin = |name| match metadata.common.pin_directory(name) {
        Ok(pin) => Ok(Some(pin)),
        Err(DomainError::NotFound) => Ok(None),
        Err(error) => Err(error),
    };
    let original_info = optional_pin("info")?;
    let original_object_info = optional_pin("objects/info")?;
    let temp = tempfile::Builder::new()
        .prefix("git-read-")
        .tempdir_in(spools)
        .map_err(io_error)?;
    std::fs::write(
        temp.path().join("attributes"),
        b"* -filter -text -ident -working-tree-encoding\n",
    )
    .map_err(io_error)?;
    for name in ["exclude", "sparse-checkout"] {
        match metadata
            .common
            .snapshot(&format!("info/{name}"), 1024 * 1024)
        {
            Ok(value) => std::fs::write(temp.path().join(name), value.bytes).map_err(io_error)?,
            Err(DomainError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    let overlay = WorkspaceService::new()?.register(temp.path())?;
    let info = overlay.pin_directory("")?;
    // Missing optional directories need mount points without modifying the host
    // repository. A read-only union supplies only these empty directories; safe
    // info mounts below still hide attributes/alternates from either lower layer.
    let skeleton = if original_info.is_none() || original_object_info.is_none() {
        std::fs::create_dir_all(temp.path().join("skeleton/info")).map_err(io_error)?;
        std::fs::create_dir_all(temp.path().join("skeleton/objects/info")).map_err(io_error)?;
        Some(overlay.pin_directory("skeleton")?)
    } else {
        None
    };
    let mut argv = [
        "--die-with-parent",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-net",
        "--ro-bind",
        "/usr",
        "/usr",
        "--symlink",
        "usr/bin",
        "/bin",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/tmp/ag1-home",
        "--ro-bind-fd",
        PINNED_CWD_FD,
        "/workspace",
    ]
    .map(str::to_string)
    .to_vec();
    for lib in ["/lib", "/lib64"] {
        if Path::new(lib).exists() {
            argv.extend(["--ro-bind".into(), lib.into(), lib.into()]);
        }
    }
    let git_dir = metadata.namespace_git_dir();
    if let Some(skeleton) = &skeleton {
        for pin in [&common, skeleton] {
            argv.extend([
                "--overlay-src".into(),
                format!("{PINNED_DIRECTORY_PREFIX}{}", pin.process_cwd().display()),
            ]);
        }
        argv.extend(["--ro-overlay".into(), "/gitcommon".into()]);
    } else {
        argv.extend([
            "--ro-bind-fd".into(),
            format!(
                "{PINNED_DIRECTORY_PREFIX}{}",
                common.process_cwd().display()
            ),
            "/gitcommon".into(),
        ]);
    }
    // Rebinding the common root here would hide the synthesized mount points.
    if metadata.git.root() != metadata.common.root() {
        argv.extend([
            "--ro-bind-fd".into(),
            format!("{PINNED_DIRECTORY_PREFIX}{}", git.process_cwd().display()),
            git_dir.clone(),
        ]);
    }
    for (pin, target) in [
        (&info, "/gitcommon/info"),
        (&info, "/gitcommon/objects/info"),
    ] {
        argv.extend([
            "--ro-bind-fd".into(),
            format!("{PINNED_DIRECTORY_PREFIX}{}", pin.process_cwd().display()),
            target.into(),
        ]);
    }
    argv.extend(
        [
            "--seccomp",
            READONLY_FILTER_FD,
            "--chdir",
            "/workspace",
            "--",
            executable.to_str().ok_or(DomainError::PermissionDenied)?,
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
            "-c",
            "core.attributesFile=/dev/null",
            "-c",
            "diff.ignoreSubmodules=all",
            "-c",
            "status.submoduleSummary=false",
            "-c",
            "log.showSignature=false",
            "-c",
            "protocol.allow=never",
        ]
        .map(str::to_string),
    );
    argv.extend_from_slice(args);
    let spool = temp.path().join("stderr.log");
    let env = BTreeMap::from([
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("HOME".into(), "/tmp/ag1-home".into()),
        ("GIT_DIR".into(), git_dir),
        ("GIT_COMMON_DIR".into(), "/gitcommon".into()),
        ("GIT_WORK_TREE".into(), "/workspace".into()),
        ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
        ("GIT_TERMINAL_PROMPT".into(), "0".into()),
        ("LC_ALL".into(), "C".into()),
        ("GIT_NO_LAZY_FETCH".into(), "1".into()),
    ]);
    let mut child = supervisor.spawn_readonly_protocol(
        ProcessSpec {
            executable: bwrap.into(),
            argv,
            cwd: root.process_cwd(),
            env,
            timeout_ms: control
                .deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .clamp(1, 300000) as u64,
            grace_ms: 100,
            resident: false,
        },
        &spool,
    )?;
    child.close_input();
    let mut output = Vec::new();
    let success = loop {
        control.check()?;
        let (bytes, done) = child.poll_stdout(32768)?;
        output.extend(bytes);
        if output.len() > limit {
            return Err(DomainError::LimitExceeded("Git output".into()));
        }
        if let Some(status) = done {
            break status.success();
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    for pin in [&root, &git, &common, &objects, &info]
        .into_iter()
        .chain(original_info.as_ref())
        .chain(original_object_info.as_ref())
        .chain(skeleton.as_ref())
    {
        pin.verify_identity()?;
    }
    for (name, before) in [
        ("info", &original_info),
        ("objects/info", &original_object_info),
    ] {
        if before.is_none()
            && !matches!(
                metadata.common.pin_directory(name),
                Err(DomainError::NotFound)
            )
        {
            return Err(DomainError::Conflict);
        }
    }
    for (root, name, before) in &metadata.links {
        if root.snapshot(name, 4096)?.revision != before.revision {
            return Err(DomainError::Conflict);
        }
    }
    control.check()?;
    if !success {
        output = child.tail(65536)?.bytes;
    }
    // These host-assembled queries describe the logical project, not guest paths.
    if success && args == ["rev-parse", "--absolute-git-dir"] {
        output = format!("{}\n", metadata.git.root().display()).into_bytes();
    }
    if success && args == ["rev-parse", "--show-toplevel"] {
        output = format!("{}\n", workspace.root().display()).into_bytes();
    }
    Ok((output, success))
}
