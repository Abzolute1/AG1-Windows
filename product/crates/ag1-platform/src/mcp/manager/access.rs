//! Native fixed confinement recipe; no metadata-controlled launcher or fallback.
use super::*;
use crate::process::{PINNED_CWD_FD, READONLY_FILTER_FD};

pub(super) fn contained(
    mut spec: ProcessSpec,
    cwd: &str,
    helper: &Path,
) -> Result<ProcessSpec, DomainError> {
    let launcher = Path::new("/usr/bin/bwrap");
    let metadata = std::fs::metadata(launcher).map_err(|_| DomainError::Unavailable("Read-only MCP access requires /usr/bin/bwrap and Linux user namespaces. Choose host-account access explicitly only if intended.".into()))?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(DomainError::PermissionDenied);
    }
    // Resolve the destination itself, not the pinned source. A symlink alias may
    // otherwise overlay reserved runtime roots or expose unexpected directories.
    let canonical = Path::new(cwd)
        .canonicalize()
        .map_err(|_| invalid("MCP directory unavailable"))?;
    if canonical != Path::new(cwd) {
        return Err(invalid(
            "Read-only MCP working directory must use its canonical absolute path",
        ));
    }
    let mut args: Vec<String> = [
        "--die-with-parent",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-net",
        "--disable-userns",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/usr",
        "/usr",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    for path in ["/lib", "/lib64"] {
        if Path::new(path).exists() {
            args.extend(["--ro-bind".into(), path.into(), path.into()]);
        }
    }
    args.extend(
        [
            "--symlink",
            "usr/bin",
            "/bin",
            "--dir",
            "/ag1",
            "--perms",
            "0555",
            "--ro-bind-data",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    args.extend([
        format!("@AG1_MCP_PINNED_FILE@{}", spec.executable.display()),
        "/ag1/server".into(),
        "--perms".into(),
        "0555".into(),
        "--ro-bind-data".into(),
        format!("@AG1_MCP_PINNED_FILE@{}", helper.display()),
        "/ag1/entry".into(),
    ]);
    args.extend(
        [
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/tmp/ag1-home",
            "--dir",
            "/run/ag1",
            "--ro-bind-data",
            "@AG1_MCP_ENV_FD@",
            "/run/ag1/mcp-env",
            "--ro-bind-fd",
            PINNED_CWD_FD,
        ]
        .into_iter()
        .map(str::to_owned),
    );
    args.extend([
        cwd.into(),
        "--chdir".into(),
        cwd.into(),
        "--seccomp".into(),
        READONLY_FILTER_FD.into(),
        "--".into(),
        "/ag1/entry".into(),
        "--mcp-contained-entry".into(),
    ]);
    args.extend(spec.argv);
    spec.executable = launcher.into();
    spec.argv = args;
    Ok(spec)
}
