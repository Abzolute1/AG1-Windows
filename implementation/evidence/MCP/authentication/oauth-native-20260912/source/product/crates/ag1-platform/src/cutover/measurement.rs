//! Private command measurements. A start without a result means the outcome is
//! unknown; it is never permission to retry a migration or an external effect.
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
type ExecutableIdentity = (u64, u64, u64, i64, i64, i64, i64);
type ExecutableCapture = (PathBuf, String, ExecutableIdentity);

pub struct Measurement {
    directory: PathBuf,
    start_sha256: String,
    started: Instant,
}

pub(super) fn create(path: &Path, value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(super::json)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(io)?;
    file.write_all(&bytes).map_err(io)?;
    file.sync_all().map_err(io)?;
    File::open(path.parent().ok_or(DomainError::PermissionDenied)?)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    Ok(digest(&bytes))
}

impl Measurement {
    /// Reserve a new private directory and durably record intent before the
    /// caller executes anything. Existing paths fail without running a command.
    pub fn begin(directory: &Path, arguments: &[String]) -> Result<Self> {
        if arguments.is_empty() || arguments[0] == "measure" {
            return Err(DomainError::InvalidInput("Measured command".into()));
        }
        let directory = super::migration::new_path(directory)?;
        // Hash the executing inode, which remains stable even if a later build
        // replaces its pathname. Reuse only while its full metadata is unchanged.
        static EXECUTABLE: std::sync::OnceLock<ExecutableCapture> = std::sync::OnceLock::new();
        let executable = std::env::current_exe().map_err(io)?;
        let mut file = File::open("/proc/self/exe").map_err(io)?;
        let metadata = file.metadata().map_err(io)?;
        let identity = (
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        );
        if EXECUTABLE.get().is_none() {
            if !metadata.is_file() || metadata.len() > 1024 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("Measurement executable".into()));
            }
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            let mut remaining = metadata.len();
            while remaining > 0 {
                let n = file.read(&mut buffer).map_err(io)?;
                if n == 0 || n as u64 > remaining {
                    return Err(DomainError::Conflict);
                }
                remaining -= n as u64;
                hash.update(&buffer[..n]);
            }
            let after = file.metadata().map_err(io)?;
            if after.len() != metadata.len()
                || after.mtime() != metadata.mtime()
                || after.mtime_nsec() != metadata.mtime_nsec()
                || after.ctime() != metadata.ctime()
                || after.ctime_nsec() != metadata.ctime_nsec()
            {
                return Err(DomainError::Conflict);
            }
            let executable_sha256 = format!("{:x}", hash.finalize());
            let _ = EXECUTABLE.set((executable.clone(), executable_sha256, identity));
        }
        let (executable, executable_sha256, expected) =
            EXECUTABLE.get().ok_or(DomainError::Conflict)?;
        if &identity != expected {
            return Err(DomainError::Conflict);
        }
        fs::create_dir(&directory).map_err(io)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(io)?;
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DomainError::Conflict)?
            .as_millis();
        let start_sha256 = create(
            &directory.join("started.json"),
            &json!({"version":1,"status":"started",
            "arguments":arguments,"executable":executable,"executable_sha256":executable_sha256,
            "started_unix_ms":started_unix_ms,"process_observation":std::process::id(),
            "scope":"Host operation duration; excludes executable hashing and receipt writes",
            "automatic_retry_allowed":false,"runtime_control_granted":false}),
        )?;
        File::open(directory.parent().ok_or(DomainError::PermissionDenied)?)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        Ok(Self {
            directory,
            start_sha256,
            started: Instant::now(),
        })
    }

    /// The command's original result remains separate from receipt durability.
    /// Failure to publish requires authority inspection before any retry.
    pub fn finish(self, outcome: &std::result::Result<Value, String>) -> Result<()> {
        let elapsed_ms = self.started.elapsed().as_millis();
        let bytes = bounded(&self.directory.join("started.json"), 1024 * 1024)?;
        if digest(&bytes) != self.start_sha256 {
            return Err(DomainError::DurabilityUnknown);
        }
        create(
            &self.directory.join("result.json"),
            &json!({"version":1,
            "status":if outcome.is_ok() {"command_succeeded"} else {"command_failed"},
            "started_sha256":self.start_sha256,"elapsed_ms":elapsed_ms,
            "result":outcome.as_ref().ok(),"error":outcome.as_ref().err(),
            "automatic_retry_allowed":false,"phase_acceptance":false}),
        )?;
        Ok(())
    }
}
