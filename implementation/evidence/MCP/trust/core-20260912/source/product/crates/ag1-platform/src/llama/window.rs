//! Read-only Linux resource observation. PIDs from this module are never used
//! as signal targets or as a source of process custody.
use ag1_core::DomainError;
use std::{
    fs,
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

struct Probe(std::process::Child);
impl Drop for Probe {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
pub fn validate(owned: Option<u32>, servers: &[u32], compute: &[u32]) -> Result<(), DomainError> {
    if servers.iter().chain(compute).any(|pid| Some(*pid) != owned) {
        return Err(DomainError::Unavailable(
            "An external model/GPU process owns the runtime window; only its owner may release it"
                .into(),
        ));
    }
    Ok(())
}
pub fn verify(owned: Option<u32>) -> Result<(), DomainError> {
    let mut servers = Vec::new();
    for entry in fs::read_dir("/proc").map_err(crate::io_error)? {
        let entry = entry.map_err(crate::io_error)?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        match fs::read_to_string(entry.path().join("comm")) {
            Ok(name) if name.trim() == "llama-server" => servers.push(pid),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => {
                return Err(DomainError::Unavailable(
                    "Cannot verify the process runtime window".into(),
                ));
            }
        }
    }
    // Fail before probing any endpoint or invoking a GPU query if an external
    // tuned process is already observed.
    validate(owned, &servers, &[])?;
    let text = query("--query-compute-apps=pid")?;
    let compute = text
        .lines()
        .map(|line| {
            line.trim()
                .parse::<u32>()
                .map_err(|_| DomainError::Conflict)
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate(owned, &servers, &compute)
}
pub(crate) fn gpu_memory() -> Result<serde_json::Value, DomainError> {
    let text = query("--query-gpu=memory.total,memory.used,memory.free")?;
    if text.lines().count() != 1 {
        return Err(DomainError::Conflict);
    }
    let values = text
        .trim()
        .split(',')
        .map(|s| s.trim().parse::<u64>().map_err(|_| DomainError::Conflict))
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != 3 || values[0] == 0 {
        return Err(DomainError::Conflict);
    }
    Ok(
        serde_json::json!({"total_mib":values[0],"used_mib":values[1],"free_mib":values[2],"scope":"GPU totals include graphics; observation, not a measured peak"}),
    )
}
fn query(argument: &str) -> Result<String, DomainError> {
    let child = Command::new("/usr/bin/nvidia-smi")
        .args([argument, "--format=csv,noheader,nounits"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| DomainError::Unavailable("GPU ownership observation is unavailable".into()))?;
    let mut probe = Probe(child);
    let child = &mut probe.0;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(crate::io_error)? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(5) {
            // This is the probe child just spawned above, never an observed PID.
            let _ = child.kill();
            let _ = child.wait();
            return Err(DomainError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        return Err(DomainError::Unavailable(
            "Cannot verify GPU compute ownership".into(),
        ));
    }
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or(DomainError::Conflict)?
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(crate::io_error)?;
    if bytes.len() > 65536 {
        return Err(DomainError::LimitExceeded("GPU observation".into()));
    }
    String::from_utf8(bytes).map_err(|_| DomainError::Conflict)
}
