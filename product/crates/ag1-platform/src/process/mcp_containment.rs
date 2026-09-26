//! Target environment is injected only after bubblewrap installs isolation.
use super::*;
use std::io::{Seek, SeekFrom};

pub(super) fn environment_file(environment: &BTreeMap<String, String>) -> Result<File, DomainError> {
    let mut bytes = serde_json::to_vec(environment)
        .map_err(|_| DomainError::InvalidInput("MCP environment encoding".into()))?;
    if bytes.len() > 64 * 1024 { return Err(DomainError::LimitExceeded("MCP environment".into())); }
    let fd = unsafe { libc::memfd_create(c"ag1-mcp-environment".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 { return Err(io_error(std::io::Error::last_os_error())); }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let result = file.write_all(&bytes).and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()));
    bytes.fill(0);
    result.map_err(io_error)?;
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE) } < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(file)
}

pub(super) fn entry() -> Result<i32, DomainError> {
    let mut bytes = Vec::new();
    File::open("/run/ag1/mcp-env").map_err(io_error)?.take(64 * 1024 + 1).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() > 64 * 1024 { return Err(DomainError::LimitExceeded("MCP environment".into())); }
    let environment: BTreeMap<String, String> = serde_json::from_slice(&bytes)
        .map_err(|_| DomainError::PermissionDenied)?;
    bytes.fill(0);
    if environment.len() > 128 || environment.iter().any(|(k,v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0')) {
        return Err(DomainError::PermissionDenied);
    }
    // This branch runs inside the native-built mount/PID/network namespaces.
    // Never parse a renderer/peer-provided executable or environment-file path.
    let error = Command::new("/ag1/server").args(std::env::args_os().skip(2))
        .env_clear().env("PATH", "/usr/bin:/bin").env("HOME", "/tmp/ag1-home")
        .envs(environment).exec();
    Err(io_error(error))
}
