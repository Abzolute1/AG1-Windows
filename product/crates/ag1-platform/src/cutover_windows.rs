//! Windows authority records for a native AG1W host.
//!
//! Linux uses flock and Unix inode metadata. Windows uses a named mutex for a
//! short-lived record lease and a volume/file index pair for the same identity
//! check. This deliberately owns AG1W state only; it does not adopt a process.

use ag1_core::DomainError;
use ag1_storage::digest;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileInformationByHandle, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING,
    },
    System::Threading::{
        CreateMutexW, CreateSemaphoreW, ReleaseMutex, ReleaseSemaphore, WaitForSingleObject,
    },
};

type Result<T> = std::result::Result<T, DomainError>;

fn io(error: std::io::Error) -> DomainError {
    crate::io_error(error)
}

fn json(error: serde_json::Error) -> DomainError {
    DomainError::InvalidInput(error.to_string())
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

// The Windows-native host does not support rollback to the old TypeScript
// worker yet. Keeping the durable shape lets it recognise old records and
// refuse an unsafe handoff instead of silently losing the marker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rollback {
    pub archive: PathBuf,
    pub archive_sha256: String,
    pub destination: PathBuf,
    pub staging: PathBuf,
    pub session: String,
    pub model: String,
    pub candidate_sha256: Option<String>,
    pub opened: bool,
}

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
    pub rollback: Option<Rollback>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
struct Identity {
    volume: u64,
    file: u64,
}

fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn reject_reparse_path(path: &Path) -> Result<()> {
    let mut current = Some(path);
    while let Some(part) = current {
        let metadata = fs::symlink_metadata(part).map_err(io)?;
        if metadata.file_type().is_symlink() || reparse(&metadata) {
            return Err(DomainError::PermissionDenied);
        }
        current = part.parent();
    }
    Ok(())
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(DomainError::PermissionDenied);
    }
    fs::create_dir_all(path).map_err(io)?;
    reject_reparse_path(path)?;
    if !fs::symlink_metadata(path).map_err(io)?.is_dir() {
        return Err(DomainError::PermissionDenied);
    }
    Ok(())
}

fn open_directory(path: &Path) -> Result<File> {
    let name = wide(path);
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn identity(path: &Path) -> Result<(PathBuf, Identity)> {
    reject_reparse_path(path)?;
    let path = fs::canonicalize(path).map_err(io)?;
    let metadata = fs::symlink_metadata(&path).map_err(io)?;
    if !metadata.is_dir() || reparse(&metadata) {
        return Err(DomainError::PermissionDenied);
    }
    let directory = open_directory(&path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(directory.as_raw_handle() as HANDLE, &mut info) } == 0 {
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok((
        path,
        Identity {
            volume: info.dwVolumeSerialNumber as u64,
            file: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        },
    ))
}

fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    reject_reparse_path(path)?;
    let file = File::open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(DomainError::PermissionDenied);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(DomainError::LimitExceeded("cutover file".into()));
    }
    Ok(bytes)
}

static TEMPORARY: AtomicU64 = AtomicU64::new(0);

fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or(DomainError::PermissionDenied)?;
    private_dir(parent)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(
        ".ag1w-pending-{}-{}-{}",
        std::process::id(),
        stamp,
        TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let write = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(|_| DomainError::DurabilityUnknown)
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let old = wide(path);
    let new = wide(&temporary);
    if unsafe {
        MoveFileExW(
            new.as_ptr(),
            old.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        let _ = fs::remove_file(&temporary);
        return Err(io(std::io::Error::last_os_error()));
    }
    Ok(())
}

static HELD_LOCKS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct NamedLock {
    handle: HANDLE,
    name: String,
}

// A runtime lease moves into the asynchronous model supervisor. Windows mutexes
// are thread-owned, so releasing one from the worker that later stops the model
// would be invalid. A binary semaphore has the same one-owner-at-a-time meaning
// here, but its release operation is explicitly not tied to the acquiring thread.
struct NamedSemaphore {
    handle: HANDLE,
    name: String,
}

// This owns a Windows kernel semaphore and performs only its documented
// cross-thread release operation. The local name set still prevents this one
// desktop process from recursively taking the same endpoint lease.
unsafe impl Send for NamedSemaphore {}

impl NamedSemaphore {
    fn acquire(name: String) -> Result<Self> {
        let held = HELD_LOCKS.get_or_init(|| Mutex::new(HashSet::new()));
        if !held
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .insert(name.clone())
        {
            return Err(DomainError::Unavailable(
                "AG1W authority is already owned by this desktop process".into(),
            ));
        }

        let wide_name = wide(&name);
        let handle = unsafe { CreateSemaphoreW(std::ptr::null(), 1, 1, wide_name.as_ptr()) };
        if handle.is_null() {
            let _ = held.lock().map(|mut held| held.remove(&name));
            return Err(io(std::io::Error::last_os_error()));
        }
        let result = unsafe { WaitForSingleObject(handle, 0) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
            return Ok(Self { handle, name });
        }
        unsafe { CloseHandle(handle) };
        let _ = held.lock().map(|mut held| held.remove(&name));
        Err(DomainError::Unavailable(
            "AG1W authority is already owned by another desktop process".into(),
        ))
    }
}

impl Drop for NamedSemaphore {
    fn drop(&mut self) {
        unsafe {
            ReleaseSemaphore(self.handle, 1, std::ptr::null_mut());
            CloseHandle(self.handle);
        }
        if let Some(held) = HELD_LOCKS.get()
            && let Ok(mut held) = held.lock()
        {
            held.remove(&self.name);
        }
    }
}

impl NamedLock {
    fn acquire(name: String) -> Result<Self> {
        // A named mutex is recursive for its owning thread. Track local names
        // too, so one desktop cannot accidentally take the same lease twice.
        let held = HELD_LOCKS.get_or_init(|| Mutex::new(HashSet::new()));
        if !held
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .insert(name.clone())
        {
            return Err(DomainError::Unavailable(
                "AG1W authority is already owned by this desktop process".into(),
            ));
        }

        let wide_name = wide(&name);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, wide_name.as_ptr()) };
        if handle.is_null() {
            let _ = held.lock().map(|mut held| held.remove(&name));
            return Err(io(std::io::Error::last_os_error()));
        }
        let result = unsafe { WaitForSingleObject(handle, 0) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED {
            return Ok(Self { handle, name });
        }
        unsafe { CloseHandle(handle) };
        let _ = held.lock().map(|mut held| held.remove(&name));
        Err(DomainError::Unavailable(
            "AG1W authority is already owned by another desktop process".into(),
        ))
    }
}

impl Drop for NamedLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
        if let Some(held) = HELD_LOCKS.get()
            && let Ok(mut held) = held.lock()
        {
            held.remove(&self.name);
        }
    }
}

#[derive(Clone)]
pub struct Registry {
    root: PathBuf,
}

impl Registry {
    pub fn for_user() -> Result<Self> {
        let root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| {
                DomainError::Unavailable("Windows local app-data path unavailable".into())
            })?
            .join("AG1W")
            .join("authority-v1");
        Self::at(&root)
    }

    pub fn at(root: &Path) -> Result<Self> {
        private_dir(root)?;
        Ok(Self {
            root: fs::canonicalize(root).map_err(io)?,
        })
    }

    fn path(&self, workspace: &Path) -> PathBuf {
        self.root
            .join(digest(workspace.as_os_str().as_encoded_bytes()))
    }

    fn lock_name(&self, label: &str) -> String {
        format!(
            "Local\\AG1W-{label}-{}",
            digest(self.root.as_os_str().as_encoded_bytes())
        )
    }

    fn locked(&self, workspace: &Path) -> Result<Lease> {
        private_dir(&self.root)?;
        let (workspace, identity) = identity(workspace)?;
        let base = self.path(&workspace);
        let lock = NamedLock::acquire(self.lock_name(&format!(
            "record-{}",
            digest(workspace.as_os_str().as_encoded_bytes())
        )))?;
        let path = base.with_extension("json");
        let record = if path.exists() {
            let envelope: Envelope =
                serde_json::from_slice(&bounded(&path, 1024 * 1024)?).map_err(json)?;
            if envelope.sha256 != digest(&serde_json::to_vec(&envelope.record).map_err(json)?)
                || envelope.record.version != 1
                || envelope.record.workspace != workspace
                || envelope.record.device != identity.volume
                || envelope.record.inode != identity.file
            {
                return Err(DomainError::Conflict);
            }
            Some(envelope.record)
        } else {
            None
        };
        Ok(Lease {
            _lock: lock,
            path,
            workspace,
            identity,
            record,
        })
    }

    pub fn runtime_lease(&self) -> Result<EndpointLease> {
        Ok(EndpointLease(NamedSemaphore::acquire(
            self.lock_name("runtime"),
        )?))
    }

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
        keys.into_iter()
            .map(|key| {
                NamedSemaphore::acquire(
                    self.lock_name(&format!("endpoint-{}", digest(key.as_bytes()))),
                )
                .map(EndpointLease)
            })
            .collect()
    }

    pub fn inspect(&self, workspace: &Path) -> Result<Option<Record>> {
        let (workspace, identity) = identity(workspace)?;
        let path = self.path(&workspace).with_extension("json");
        if !path.exists() {
            return Ok(None);
        }
        let envelope: Envelope =
            serde_json::from_slice(&bounded(&path, 1024 * 1024)?).map_err(json)?;
        if envelope.sha256 != digest(&serde_json::to_vec(&envelope.record).map_err(json)?)
            || envelope.record.version != 1
            || envelope.record.workspace != workspace
            || envelope.record.device != identity.volume
            || envelope.record.inode != identity.file
        {
            return Err(DomainError::Conflict);
        }
        Ok(Some(envelope.record))
    }

    pub fn initialize_isolated(&self, workspace: &Path) -> Result<()> {
        self.initialize_native_record(workspace, true)
    }

    pub fn initialize_native(&self, workspace: &Path) -> Result<()> {
        self.initialize_native_record(workspace, false)
    }

    fn initialize_native_record(&self, workspace: &Path, isolated: bool) -> Result<()> {
        let mut lease = self.locked(workspace)?;
        if lease.record.is_some()
            || (isolated && fs::read_dir(&lease.workspace).map_err(io)?.next().is_some())
        {
            return Err(DomainError::Conflict);
        }
        lease.save(Record {
            version: 1,
            workspace: lease.workspace.clone(),
            device: lease.identity.volume,
            inode: lease.identity.file,
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

    // The Windows desktop does not run the legacy TypeScript worker. This is
    // kept as a clear refusal so imported records cannot accidentally wake it.
    pub fn open_typescript(&self, _workspace: &Path, _state: &Path) -> Result<Lease> {
        Err(DomainError::Unavailable(
            "The legacy TypeScript worker is unavailable in the Windows-native runtime".into(),
        ))
    }

    fn open_owner(&self, workspace: &Path, state: &Path, owner: Owner) -> Result<Lease> {
        let mut lease = self.locked(workspace)?;
        private_dir(state)?;
        let state = fs::canonicalize(state).map_err(io)?;
        if state.starts_with(&lease.workspace) || lease.workspace.starts_with(&state) {
            return Err(DomainError::PermissionDenied);
        }
        let mut record = match lease.record.clone() {
            Some(record) => record,
            None if owner == Owner::TypeScript => Record {
                version: 1,
                workspace: lease.workspace.clone(),
                device: lease.identity.volume,
                inode: lease.identity.file,
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
        if record.owner != owner
            || record.stage != Stage::Active
            || record.state.as_ref().is_some_and(|old| old != &state)
        {
            return Err(conflict());
        }
        if record.rollback.is_some() {
            return Err(DomainError::Unavailable(
                "This workspace has a legacy rollback marker; recover it with the Linux host first"
                    .into(),
            ));
        }
        record.state = Some(state);
        record.quiescent = false;
        lease.save(record)?;
        Ok(lease)
    }
}

pub struct EndpointLease(NamedSemaphore);

pub struct Lease {
    _lock: NamedLock,
    path: PathBuf,
    workspace: PathBuf,
    identity: Identity,
    record: Option<Record>,
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
        let envelope: Envelope =
            serde_json::from_slice(&bounded(&self.path, 1024 * 1024)?).map_err(json)?;
        let (_, identity) = identity(&self.workspace)?;
        if Some(&envelope.record) != self.record.as_ref()
            || envelope.sha256 != digest(&serde_json::to_vec(&envelope.record).map_err(json)?)
            || envelope.record.stage != Stage::Active
            || envelope.record.quiescent
            || identity != self.identity
        {
            return Err(conflict());
        }
        Ok(())
    }

    pub fn begin_drain(&mut self) -> Result<()> {
        let mut record = self.record()?.clone();
        if record.stage != Stage::Active && record.stage != Stage::Draining {
            return Err(conflict());
        }
        record.stage = Stage::Draining;
        self.save(record)
    }

    pub fn quiesced(&mut self) -> Result<()> {
        let mut record = self.record()?.clone();
        record.quiescent = true;
        self.save(record)
    }

    pub fn cancel_transfer(&mut self) -> Result<()> {
        let mut record = self.record()?.clone();
        if !record.quiescent || record.stage != Stage::Draining || record.transfer.is_some() {
            return Err(conflict());
        }
        record.stage = Stage::Active;
        self.save(record)
    }
}

#[path = "cutover/configuration_windows.rs"]
pub mod configuration;

#[cfg(test)]
mod tests {
    use super::*;

    fn directories() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let root = tempfile::Builder::new()
            .prefix("ag1w-authority-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let workspace = root.path().join("workspace");
        let state = root.path().join("state");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&state).unwrap();
        let authority = root.path().join("authority");
        (root, workspace, state, authority)
    }

    #[test]
    fn native_record_requires_clean_drain_before_reopen() {
        let (_temp, workspace, state, authority) = directories();
        let registry = Registry::at(&authority).unwrap();
        registry.initialize_native(&workspace).unwrap();
        let mut lease = registry.open_native(&workspace, &state).unwrap();
        assert!(lease.admit().is_ok());
        assert!(matches!(
            registry.open_native(&workspace, &state),
            Err(DomainError::Unavailable(_))
        ));
        // Normal host shutdown stays Active; Draining is reserved for a real
        // migration and must not be used merely because the app is closing.
        lease.quiesced().unwrap();
        drop(lease);
        let second = registry.open_native(&workspace, &state).unwrap();
        assert!(second.admit().is_ok());
    }

    #[test]
    fn endpoint_lease_only_locks_the_same_loopback_port() {
        let (_temp, workspace, _state, authority) = directories();
        let registry = Registry::at(&authority).unwrap();
        let first = registry
            .endpoint_leases(&["http://127.0.0.1:18082/v1/".into()])
            .unwrap();
        assert!(matches!(
            registry.endpoint_leases(&["http://localhost:18082/v1/".into()]),
            Err(DomainError::Unavailable(_))
        ));
        assert!(
            registry
                .endpoint_leases(&["https://example.invalid/v1/".into()])
                .unwrap()
                .is_empty()
        );
        drop(first);
        assert_eq!(
            registry
                .endpoint_leases(&["http://localhost:18082/v1/".into()])
                .unwrap()
                .len(),
            1
        );
        assert!(workspace.exists());
    }
}
