//! Durable, quiescent authority transfer. This module never controls an observed
//! process or grants ownership of an inference endpoint.
use ag1_core::DomainError;
use ag1_storage::{FrozenManifest, Store, digest};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, DomainError>;
fn io(e: std::io::Error) -> DomainError {
    crate::io_error(e)
}
fn json(e: serde_json::Error) -> DomainError {
    DomainError::InvalidInput(e.to_string())
}
fn conflict() -> DomainError {
    DomainError::Unavailable("Workspace is migrating; retry after recovery/resnapshot".into())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Owner {
    TypeScript,
    Native,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Active,
    Draining,
    Frozen,
    Imported,
    Validated,
    Revoked,
    RollbackFrozen,
    RollbackPrepared,
}

/// Private host metadata; never a client capability or a copied PID grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub version: u32,
    pub workspace: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub epoch: u64,
    pub owner: Owner,
    pub stage: Stage,
    pub state: Option<PathBuf>,
    pub isolated: bool,
    pub quiescent: bool,
    pub transfer: Option<Transfer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback: Option<rollback::Rollback>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transfer {
    pub id: String,
    pub original_state: PathBuf,
    pub archive: PathBuf,
    pub destination: PathBuf,
    pub manifest_sha256: String,
    pub session: String,
    pub config_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    sha256: String,
    record: Record,
}

/// All clients of a user share this registry, independently of their state dir.
#[derive(Clone)]
pub struct Registry {
    root: PathBuf,
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    // Reject symlinks before creating any descendants.
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.is_symlink() || !m.is_dir() => return Err(DomainError::PermissionDenied),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(io)?;
                fs::set_permissions(&current, fs::Permissions::from_mode(0o700)).map_err(io)?;
            }
            Err(e) => return Err(io(e)),
        }
    }
    let m = fs::symlink_metadata(path).map_err(io)?;
    if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err(DomainError::PermissionDenied);
    }
    Ok(())
}

fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(io)?;
    if !f.metadata().map_err(io)?.is_file() {
        return Err(DomainError::PermissionDenied);
    }
    let mut bytes = Vec::new();
    f.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(DomainError::LimitExceeded("cutover file".into()));
    }
    Ok(bytes)
}

fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(DomainError::PermissionDenied)?;
    let temp = parent.join(format!(".pending-{}", crate::client::nonce()?));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)
        .map_err(io)?;
    f.write_all(bytes).map_err(io)?;
    f.sync_all().map_err(io)?;
    fs::rename(temp, path).map_err(io)?;
    File::open(parent).and_then(|f| f.sync_all()).map_err(io)
}

fn identity(workspace: &Path) -> Result<(PathBuf, u64, u64)> {
    let path = fs::canonicalize(workspace).map_err(io)?;
    let m = fs::metadata(&path).map_err(io)?;
    if !m.is_dir() {
        return Err(DomainError::PermissionDenied);
    }
    Ok((path, m.dev(), m.ino()))
}

impl Registry {
    /// Cross-process exclusion for the whole preserved single-GPU runtime bundle.
    /// A lock is not authority over any process that was started externally.
    pub fn runtime_lease(&self) -> Result<EndpointLease> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.root.join("preserved-runtime.lock"))
            .map_err(io)?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).map_err(
            |_| DomainError::Unavailable("Preserved runtime already has a host owner".into()),
        )?;
        Ok(EndpointLease(file))
    }
    /// A cooperative process lease, not ownership of an externally observed
    /// service. Aliased loopback hosts share a port key. Production callers must
    /// additionally hold a runtime-control capability before using a local model.
    pub fn endpoint_leases(&self, origins: &[String]) -> Result<Vec<EndpointLease>> {
        let mut keys = std::collections::BTreeSet::new();
        for origin in origins {
            let url = reqwest::Url::parse(origin)
                .map_err(|_| DomainError::InvalidInput("endpoint origin".into()))?;
            let host = url.host_str().ok_or(DomainError::PermissionDenied)?;
            let local = host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback());
            if local {
                keys.insert(format!(
                    "loopback:{}",
                    url.port_or_known_default()
                        .ok_or(DomainError::PermissionDenied)?
                ));
            }
        }
        let mut leases = Vec::new();
        for key in keys {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(
                    self.root
                        .join(format!("endpoint-{}.lock", digest(key.as_bytes()))),
                )
                .map_err(io)?;
            rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| {
                    DomainError::Unavailable(
                        "Local inference endpoint already has a host owner".into(),
                    )
                })?;
            leases.push(EndpointLease(file));
        }
        Ok(leases)
    }
    /// Use the OS account home, not inherited GUI/worker HOME or a caller's state
    /// directory. Changing those must not create a second ownership namespace.
    pub fn for_user() -> Result<Self> {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 65536];
        let rc = unsafe {
            libc::getpwuid_r(
                libc::geteuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if rc != 0 || result.is_null() {
            return Err(DomainError::Unavailable(
                "OS account home unavailable".into(),
            ));
        }
        let entry = unsafe { entry.assume_init() };
        let home = unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) }
            .to_str()
            .map_err(|_| DomainError::PermissionDenied)?;
        Self::at(&Path::new(home).join(".local/state/ag1/authority-v1"))
    }
    /// Explicit registry roots are for fixture harnesses and host embedding.
    /// Production desktop/CLI always use for_user.
    pub fn at(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err(DomainError::PermissionDenied);
        }
        private_dir(root)?;
        Ok(Self {
            root: fs::canonicalize(root).map_err(io)?,
        })
    }
    fn path(&self, workspace: &Path) -> PathBuf {
        self.root
            .join(digest(workspace.as_os_str().as_encoded_bytes()))
    }
    fn locked(&self, workspace: &Path) -> Result<Lease> {
        private_dir(&self.root)?;
        let (workspace, device, inode) = identity(workspace)?;
        let base = self.path(&workspace);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(base.with_extension("lock"))
            .map_err(io)?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| conflict())?;
        let path = base.with_extension("json");
        let record = if path.exists() {
            let e: Envelope =
                serde_json::from_slice(&bounded(&path, 1024 * 1024)?).map_err(json)?;
            if e.sha256 != digest(&serde_json::to_vec(&e.record).map_err(json)?)
                || e.record.version != 1
                || e.record.workspace != workspace
                || e.record.device != device
                || e.record.inode != inode
            {
                return Err(DomainError::Conflict);
            }
            Some(e.record)
        } else {
            None
        };
        Ok(Lease {
            lock,
            path,
            workspace,
            device,
            inode,
            record,
        })
    }
    pub fn inspect(&self, workspace: &Path) -> Result<Option<Record>> {
        // Read-only reports need no writer lock and can observe draining hosts.
        let (workspace, device, inode) = identity(workspace)?;
        let path = self.path(&workspace).with_extension("json");
        if !path.exists() {
            return Ok(None);
        }
        let e: Envelope = serde_json::from_slice(&bounded(&path, 1024 * 1024)?).map_err(json)?;
        if e.sha256 != digest(&serde_json::to_vec(&e.record).map_err(json)?)
            || e.record.version != 1
            || e.record.workspace != workspace
            || e.record.device != device
            || e.record.inode != inode
        {
            return Err(DomainError::Conflict);
        }
        Ok(Some(e.record))
    }
    pub fn initialize_isolated(&self, workspace: &Path) -> Result<()> {
        self.initialize_native_record(workspace, true)
    }
    /// Start fresh native agent state for a user-selected project. Existing
    /// ownership always requires its normal reopen or migration path.
    pub fn initialize_native(&self, workspace: &Path) -> Result<()> {
        self.initialize_native_record(workspace, false)
    }
    fn initialize_native_record(&self, workspace: &Path, isolated: bool) -> Result<()> {
        let mut lease = self.locked(workspace)?;
        if lease.record.is_some()
            || (isolated && fs::read_dir(workspace).map_err(io)?.next().is_some())
        {
            return Err(DomainError::Conflict);
        }
        lease.save(Record {
            version: 1,
            workspace: lease.workspace.clone(),
            device: lease.device,
            inode: lease.inode,
            epoch: 1,
            owner: Owner::Native,
            stage: Stage::Active,
            state: None,
            isolated,
            quiescent: true,
            transfer: None,
            rollback: None,
        })
    }
    pub fn open_native(&self, workspace: &Path, state: &Path) -> Result<Lease> {
        self.open_owner(workspace, state, Owner::Native)
    }
    pub fn open_typescript(&self, workspace: &Path, state: &Path) -> Result<Lease> {
        self.open_owner(workspace, state, Owner::TypeScript)
    }
    fn open_owner(&self, workspace: &Path, state: &Path, owner: Owner) -> Result<Lease> {
        let mut lease = self.locked(workspace)?;
        let state = fs::canonicalize(state).map_err(io)?;
        if state.starts_with(&lease.workspace) || lease.workspace.starts_with(&state) {
            return Err(DomainError::PermissionDenied);
        }
        let mut r = match lease.record.clone() {
            Some(r) => r,
            None if owner == Owner::TypeScript => Record {
                version: 1,
                workspace: lease.workspace.clone(),
                device: lease.device,
                inode: lease.inode,
                epoch: 1,
                owner: owner.clone(),
                stage: Stage::Active,
                state: Some(state.clone()),
                isolated: false,
                quiescent: true,
                transfer: None,
                rollback: None,
            },
            None => return Err(DomainError::PermissionDenied),
        };
        if r.owner != owner
            || r.stage != Stage::Active
            || r.state.as_ref().is_some_and(|old| old != &state)
        {
            return Err(conflict());
        }
        if owner == Owner::TypeScript && !r.quiescent {
            return Err(DomainError::Unavailable(
                "TypeScript owner exited without verified drain; retain its state for recovery"
                    .into(),
            ));
        }
        if owner == Owner::TypeScript
            && r.rollback.as_ref().is_some_and(|rollback| !rollback.opened)
        {
            rollback::admit_candidate(&r, &state)?;
            r.rollback.as_mut().unwrap().opened = true;
        }
        r.state = Some(state);
        r.quiescent = false;
        lease.save(r)?;
        Ok(lease)
    }
    /// Recovery/transfer can take this lock only after the old execution lease
    /// has been released. A copied PID or UI acknowledgement is never accepted.
    pub fn resume_transfer(&self, workspace: &Path) -> Result<Lease> {
        let lease = self.locked(workspace)?;
        let r = lease.record()?;
        if r.stage == Stage::Active || !r.quiescent {
            return Err(conflict());
        }
        Ok(lease)
    }
    /// Administrative drain of an already stopped, verified-quiescent owner.
    /// Acquiring this lock cannot turn an unclean exit into quiescence.
    pub fn begin_transfer(&self, workspace: &Path) -> Result<Lease> {
        let mut lease = self.locked(workspace)?;
        let mut record = lease.record()?.clone();
        if record.stage != Stage::Active || !record.quiescent || record.owner != Owner::TypeScript {
            return Err(conflict());
        }
        record.stage = Stage::Draining;
        lease.save(record)?;
        Ok(lease)
    }
}

pub struct Lease {
    lock: File,
    path: PathBuf,
    workspace: PathBuf,
    device: u64,
    inode: u64,
    record: Option<Record>,
}
pub struct EndpointLease(File);
impl Drop for EndpointLease {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.lock, rustix::fs::FlockOperation::Unlock);
    }
}
impl Lease {
    pub fn record(&self) -> Result<&Record> {
        self.record.as_ref().ok_or(DomainError::NotFound)
    }
    fn save(&mut self, record: Record) -> Result<()> {
        if let Some(current) = self.record.as_ref() {
            let old: Envelope =
                serde_json::from_slice(&bounded(&self.path, 1024 * 1024)?).map_err(json)?;
            if &old.record != current
                || old.sha256 != digest(&serde_json::to_vec(current).map_err(json)?)
            {
                return Err(DomainError::Conflict);
            }
        }
        let envelope = Envelope {
            sha256: digest(&serde_json::to_vec(&record).map_err(json)?),
            record: record.clone(),
        };
        atomic(&self.path, &serde_json::to_vec(&envelope).map_err(json)?)?;
        self.record = Some(record);
        Ok(())
    }
    pub fn admit(&self) -> Result<()> {
        let e: Envelope =
            serde_json::from_slice(&bounded(&self.path, 1024 * 1024)?).map_err(json)?;
        if Some(&e.record) != self.record.as_ref()
            || e.sha256 != digest(&serde_json::to_vec(&e.record).map_err(json)?)
            || e.record.stage != Stage::Active
            || e.record.quiescent
            || identity(&self.workspace)? != (self.workspace.clone(), self.device, self.inode)
        {
            return Err(conflict());
        }
        Ok(())
    }
    /// Called by the executing host before cancel/settle. The lease remains held
    /// through shutdown, including owned tools, jobs, terminals and preview grants.
    pub fn begin_drain(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if r.stage != Stage::Active && r.stage != Stage::Draining {
            return Err(conflict());
        }
        r.stage = Stage::Draining;
        self.save(r)
    }
    /// Host-only acknowledgement, after its real shutdown has returned. It does
    /// not release the lease: dropping the host is a separate required boundary.
    pub fn quiesced(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        r.quiescent = true;
        self.save(r)
    }
    pub fn cancel_transfer(&mut self) -> Result<()> {
        let mut r = self.record()?.clone();
        if !r.quiescent || r.stage != Stage::Draining || r.transfer.is_some() {
            return Err(conflict());
        }
        r.stage = Stage::Active;
        self.save(r)
    }
}

mod audit;
mod legacy;
mod migration;
pub use migration::{FrozenBundle, Prepare};
mod source;
pub use source::SourceSnapshot;
mod recovery;
pub use recovery::{RecoveryManifest, restore_native_archive};
pub mod canary;
pub mod configuration;
pub mod measurement;
mod rollback;
pub mod workflow;
pub use rollback::Rollback;
