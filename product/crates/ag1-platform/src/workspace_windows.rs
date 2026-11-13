//! Windows workspace capability foundation.
//! The Linux backend uses `openat2`; Windows needs a separate implementation
//! because file descriptors and Unix mount semantics do not exist here.

use crate::{executor::BlockingPool, io_error};
use ag1_core::{
    ByteSnapshot, DomainError, FileRevision, MutationPort, PortFuture, WorkspaceId, WorkspacePort,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::{
        fs::MetadataExt,
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, GetFileInformationByHandle, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING,
    },
    System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
};

pub const MAX_READ: usize = 8 * 1024 * 1024;
pub const MAX_STREAM_READ: u64 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 1000;
pub const MAX_INDEX_BYTES: usize = 16 * 1024 * 1024;
static UNDO_SERIAL: AtomicU64 = AtomicU64::new(0);

type Result<T> = std::result::Result<T, DomainError>;

/// Each point where Undo checks that the session and workspace are still
/// eligible. The host supplies the policy; the workspace owns the file moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoStage {
    Prepare,
    BeforeMove,
    AfterMove,
    BeforePublish,
    AfterPublish,
}

/// An Undo that crosses its first rename never reports a retryable failure.
/// The recovery file instead gives the user a concrete, bounded next step.
#[derive(Clone, Debug)]
pub struct UndoOutcome {
    pub state: &'static str,
    pub recovery: Option<String>,
    pub revision: Option<FileRevision>,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    volume: u64,
    file: u64,
}

fn relative(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 4096
        || value.contains('\0')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(DomainError::InvalidInput(
            "contained relative path required".into(),
        ));
    }
    Ok(path)
}

fn identity(file: &File) -> Result<Identity> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    let handle = file.as_raw_handle() as HANDLE;
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(Identity {
        volume: info.dwVolumeSerialNumber as u64,
        file: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

fn revision(file: &File, sha256: &str) -> Result<FileRevision> {
    let metadata = file.metadata().map_err(io_error)?;
    let id = identity(file)?;
    Ok(FileRevision {
        sha256: sha256.into(),
        device: id.volume,
        inode: id.file,
        size: metadata.file_size(),
        // Windows reports these FILETIME values in 100ns ticks. Keep the
        // original field names so the portable core has one revision shape.
        modified_ns: i128::from(metadata.last_write_time()) * 100,
        // Rust exposes Windows change time experimentally. Last-write time is
        // stable, so use it for both fields until the Windows journal supplies
        // an independently observed change timestamp.
        changed_ns: i128::from(metadata.last_write_time()) * 100,
        mode: if metadata.permissions().readonly() {
            0o444
        } else {
            0o666
        },
    })
}

fn has_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Move without replacement. `MOVEFILE_WRITE_THROUGH` asks Windows to flush
/// the rename before returning; it is the strongest filesystem boundary this
/// backend can use without pretending it has Unix directory fsync semantics.
fn move_no_replace(from: &Path, to: &Path) -> Result<()> {
    let from = wide(from);
    let to = wide(to);
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        let error = std::io::Error::last_os_error();
        return if error.kind() == std::io::ErrorKind::AlreadyExists {
            Err(DomainError::Conflict)
        } else {
            Err(io_error(error))
        };
    }
    Ok(())
}

fn display_path(path: PathBuf) -> PathBuf {
    // std::fs::canonicalize adds the extended `\\?\` prefix on Windows.
    // Keep roots in the spelling a user enters; the handle-derived identity is
    // still the authority boundary, not this display string.
    let Some(text) = path.to_str() else {
        return path;
    };
    let Some(stripped) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    if let Some(unc) = stripped.strip_prefix("UNC\\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else {
        PathBuf::from(stripped)
    }
}

fn open_directory(path: &Path) -> Result<File> {
    let path = wide(path);
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            FILE_GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

pub struct PinnedDirectory {
    root: PathBuf,
    root_identity: Identity,
    relative: PathBuf,
    path: PathBuf,
    identity: Identity,
}

impl PinnedDirectory {
    pub(crate) fn relative_path(&self) -> &Path {
        &self.relative
    }

    pub fn process_cwd(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn verify_identity(&self) -> Result<()> {
        let root = open_directory(&self.root)?;
        if identity(&root)? != self.root_identity {
            return Err(DomainError::Conflict);
        }
        let current = open_directory(&self.path)?;
        if identity(&current)? != self.identity {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

pub struct Workspace {
    id: WorkspaceId,
    root: PathBuf,
    root_identity: Identity,
    claim: Mutex<()>,
}

impl Workspace {
    /// A conversation keeps the same verified directory capability while its
    /// journal gets a conversation-specific identity in storage.
    pub(crate) fn for_conversation(&self, id: &WorkspaceId) -> Result<Arc<Self>> {
        self.verify_root()?;
        Ok(Arc::new(Self {
            id: id.clone(),
            root: self.root.clone(),
            root_identity: self.root_identity,
            claim: Mutex::new(()),
        }))
    }

    pub(crate) fn directory_identity(&self) -> (u64, u64) {
        (self.root_identity.volume, self.root_identity.file)
    }

    pub fn id(&self) -> &WorkspaceId {
        &self.id
    }

    pub(crate) fn canonical_id(&self) -> Result<WorkspaceId> {
        self.verify_root()?;
        WorkspaceId::new(format!(
            "ws-{:x}",
            Sha256::digest(
                format!(
                    "{}:{}:{}",
                    self.root.display(),
                    self.root_identity.volume,
                    self.root_identity.file
                )
                .as_bytes()
            )
        ))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn verify_root(&self) -> Result<()> {
        let root = open_directory(&self.root)?;
        if identity(&root)? != self.root_identity {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }

    fn path(&self, value: &str) -> Result<PathBuf> {
        self.verify_root()?;
        let relative = relative(value)?;
        let mut current = self.root.clone();
        for part in relative.components() {
            current.push(part.as_os_str());
            if let Ok(metadata) = fs::symlink_metadata(&current) {
                // Junctions and other reparse points can leave the granted root
                // just as a Unix symlink can, so reject them before every open.
                if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
                    return Err(DomainError::PermissionDenied);
                }
            }
        }
        Ok(current)
    }

    pub fn pin_directory(&self, value: &str) -> Result<PinnedDirectory> {
        let relative = if value.is_empty() {
            PathBuf::new()
        } else {
            relative(value)?.to_path_buf()
        };
        let path = if relative.as_os_str().is_empty() {
            self.root.clone()
        } else {
            self.path(value)?
        };
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
            return Err(DomainError::PermissionDenied);
        }
        let directory = open_directory(&path)?;
        Ok(PinnedDirectory {
            root: self.root.clone(),
            root_identity: self.root_identity,
            relative,
            path,
            identity: identity(&directory)?,
        })
    }

    pub fn exists(&self, value: &str) -> Result<bool> {
        match self.path(value) {
            Ok(path) => Ok(path.exists()),
            Err(DomainError::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn directory_exists(&self, value: &str) -> Result<bool> {
        Ok(self.path(value).ok().is_some_and(|path| path.is_dir()))
    }

    pub(crate) fn regular_file(&self, value: &str) -> Result<bool> {
        self.verify_root()?;
        let metadata = fs::symlink_metadata(self.path(value)?).map_err(io_error)?;
        if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
            return Err(DomainError::PermissionDenied);
        }
        self.verify_root()?;
        Ok(metadata.is_file())
    }

    pub fn directory_entries(&self, value: &str) -> Result<Vec<ag1_core::tools::list::Entry>> {
        self.verify_root()?;
        let path = if value.is_empty() {
            self.root.clone()
        } else {
            self.path(value)?
        };
        let directory = open_directory(&path)?;
        let before = (
            identity(&directory)?,
            directory.metadata().map_err(io_error)?.last_write_time(),
        );
        let mut entries = Vec::new();
        let mut bytes = 0usize;
        for entry in fs::read_dir(&path).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry
                .file_name()
                .to_str()
                .ok_or_else(|| DomainError::InvalidInput("non-UTF8 filename".into()))?
                .to_owned();
            if entries.len() >= 10_000 {
                return Err(DomainError::LimitExceeded("directory entries".into()));
            }
            bytes = bytes.saturating_add(name.len());
            if bytes > 4 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("directory names".into()));
            }
            let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
            entries.push(ag1_core::tools::list::Entry {
                name,
                directory: metadata.is_dir() && !has_reparse_point(&metadata),
                symlink: metadata.file_type().is_symlink() || has_reparse_point(&metadata),
            });
        }
        let after = (
            identity(&directory)?,
            directory.metadata().map_err(io_error)?.last_write_time(),
        );
        let current = open_directory(&path)?;
        let reopened = (
            identity(&current)?,
            current.metadata().map_err(io_error)?.last_write_time(),
        );
        if before != after || after != reopened {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(entries)
    }

    pub fn snapshot(&self, value: &str, limit: usize) -> Result<ByteSnapshot> {
        self.snapshot_with_barrier(value, limit, || {})
    }

    pub fn snapshot_with_barrier(
        &self,
        value: &str,
        limit: usize,
        after_read: impl FnOnce(),
    ) -> Result<ByteSnapshot> {
        if limit == 0 || limit > MAX_READ {
            return Err(DomainError::LimitExceeded("read bytes".into()));
        }
        let path = self.path(value)?;
        let mut file = File::open(&path).map_err(io_error)?;
        let before = revision(&file, "")?;
        if !file.metadata().map_err(io_error)?.is_file() || before.size > limit as u64 {
            return Err(DomainError::LimitExceeded("regular file read".into()));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > limit {
            return Err(DomainError::LimitExceeded("read bytes".into()));
        }
        after_read();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let after = revision(&file, &digest)?;
        if before.device != after.device || before.inode != after.inode || before.size != after.size
        {
            return Err(DomainError::Conflict);
        }
        let current = File::open(self.path(value)?).map_err(io_error)?;
        if revision(&current, &digest)? != after {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(ByteSnapshot {
            bytes,
            revision: after,
        })
    }

    pub fn stream(
        &self,
        value: &str,
        limit: u64,
        mut consume: impl FnMut(&[u8]) -> Result<()>,
    ) -> Result<FileRevision> {
        if limit == 0 || limit > MAX_STREAM_READ {
            return Err(DomainError::LimitExceeded("streamed file bytes".into()));
        }
        let path = self.path(value)?;
        let mut file = File::open(&path).map_err(io_error)?;
        let before = revision(&file, "")?;
        if !file.metadata().map_err(io_error)?.is_file() || before.size > limit {
            return Err(DomainError::LimitExceeded("regular streamed file".into()));
        }
        let mut hash = Sha256::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let count = file.read(&mut buffer).map_err(io_error)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > limit {
                return Err(DomainError::LimitExceeded("streamed file bytes".into()));
            }
            hash.update(&buffer[..count]);
            consume(&buffer[..count])?;
        }
        let digest = format!("{:x}", hash.finalize());
        let after = revision(&file, &digest)?;
        if before.device != after.device || before.inode != after.inode || bytes != after.size {
            return Err(DomainError::Conflict);
        }
        let current = File::open(self.path(value)?).map_err(io_error)?;
        if revision(&current, &digest)? != after {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(after)
    }

    fn claim(&self) -> Result<WorkspaceClaim<'_>> {
        let guard = self.claim.try_lock().map_err(|_| DomainError::Conflict)?;
        let mut name: Vec<u16> = format!("Local\\AG1-workspace-{}", self.id.as_str())
            .encode_utf16()
            .collect();
        name.push(0);
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let result = unsafe { WaitForSingleObject(handle, 0) };
        if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
            unsafe { CloseHandle(handle) };
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(WorkspaceClaim {
            _guard: guard,
            handle,
        })
    }

    pub fn replace(
        &self,
        value: &str,
        expected: &FileRevision,
        bytes: &[u8],
    ) -> Result<FileRevision> {
        self.replace_checked(value, expected, bytes, || Ok(()))
    }

    /// Check cancellation before every point where a replacement can become
    /// visible. A renderer callback never reaches this method; callers supply
    /// only the host-owned operation fence.
    pub fn replace_checked(
        &self,
        value: &str,
        expected: &FileRevision,
        bytes: &[u8],
        check: impl Fn() -> Result<()>,
    ) -> Result<FileRevision> {
        check()?;
        if bytes.len() > MAX_READ {
            return Err(DomainError::LimitExceeded("mutation bytes".into()));
        }
        let _claim = self.claim()?;
        if self.snapshot(value, MAX_READ)?.revision != *expected {
            return Err(DomainError::Conflict);
        }
        let path = self.path(value)?;
        let parent = path.parent().ok_or(DomainError::PermissionDenied)?;
        let temporary = parent.join(format!(
            ".ag1-write-{}-{}",
            std::process::id(),
            expected.inode
        ));
        if temporary.exists() {
            return Err(DomainError::Conflict);
        }
        let write = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(io_error)?;
            file.write_all(bytes).map_err(io_error)?;
            check()?;
            // Windows exposes only a readonly flag, so preserve that portable
            // part of the prior mode instead of silently making it writable.
            if expected.mode & 0o222 == 0 {
                let mut permissions = file.metadata().map_err(io_error)?.permissions();
                permissions.set_readonly(true);
                file.set_permissions(permissions).map_err(io_error)?;
            }
            file.sync_all().map_err(|_| DomainError::DurabilityUnknown)
        })();
        if let Err(error) = write {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        check()?;
        if self.snapshot(value, MAX_READ)?.revision != *expected {
            let _ = fs::remove_file(&temporary);
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        let old = wide(&path);
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
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let snapshot = self
            .snapshot(value, MAX_READ)
            .map_err(|_| DomainError::DurabilityUnknown)?;
        if snapshot.bytes != bytes {
            return Err(DomainError::DurabilityUnknown);
        }
        Ok(snapshot.revision)
    }

    /// Publish an absent file without replacing another creator's result.
    /// Missing parent directories are created only after an authorized caller
    /// explicitly asks for them.
    pub fn create_checked(
        &self,
        value: &str,
        bytes: &[u8],
        parents: bool,
        check: impl Fn() -> Result<()>,
    ) -> Result<FileRevision> {
        check()?;
        if bytes.len() > MAX_READ {
            return Err(DomainError::LimitExceeded("mutation bytes".into()));
        }
        let relative = relative(value)?.to_path_buf();
        if relative.components().count() > 32 {
            return Err(DomainError::LimitExceeded("parent depth".into()));
        }
        let _claim = self.claim()?;
        match self.snapshot(value, MAX_READ) {
            Ok(_) => return Err(DomainError::Conflict),
            Err(DomainError::NotFound) => {}
            Err(error) => return Err(error),
        }

        let parent_relative = relative.parent().unwrap_or(Path::new(""));
        let mut parent = self.root.clone();
        for component in parent_relative.components() {
            check()?;
            parent.push(component.as_os_str());
            match fs::symlink_metadata(&parent) {
                Ok(metadata)
                    if metadata.is_dir()
                        && !metadata.file_type().is_symlink()
                        && !has_reparse_point(&metadata) => {}
                Ok(_) => return Err(DomainError::PermissionDenied),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && parents => {
                    match fs::create_dir(&parent) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(io_error(error)),
                    }
                    let metadata = fs::symlink_metadata(&parent).map_err(io_error)?;
                    if !metadata.is_dir()
                        || metadata.file_type().is_symlink()
                        || has_reparse_point(&metadata)
                    {
                        return Err(DomainError::PermissionDenied);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(DomainError::NotFound);
                }
                Err(error) => return Err(io_error(error)),
            }
        }

        let path = self.path(value)?;
        let temporary = parent.join(format!(
            ".ag1-create-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let write = (|| -> Result<()> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(io_error)?;
            file.write_all(bytes).map_err(io_error)?;
            check()?;
            file.sync_all().map_err(|_| DomainError::DurabilityUnknown)
        })();
        if let Err(error) = write {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        check()?;
        self.verify_root()?;
        let old = wide(&path);
        let new = wide(&temporary);
        if unsafe { MoveFileExW(new.as_ptr(), old.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
            let error = std::io::Error::last_os_error();
            let _ = fs::remove_file(&temporary);
            return if error.kind() == std::io::ErrorKind::AlreadyExists {
                Err(DomainError::Conflict)
            } else {
                Err(io_error(error))
            };
        }
        let snapshot = self
            .snapshot(value, MAX_READ)
            .map_err(|_| DomainError::DurabilityUnknown)?;
        if snapshot.bytes != bytes {
            return Err(DomainError::DurabilityUnknown);
        }
        Ok(snapshot.revision)
    }

    /// Rename one existing file only if its revision still matches and the
    /// destination remains absent. Recovery uses this to put a displaced file
    /// back without ever overwriting work created after an interrupted Undo.
    pub fn rename(&self, from: &str, to: &str, expected: &FileRevision) -> Result<()> {
        let _claim = self.claim()?;
        let source = self.snapshot(from, MAX_READ)?;
        if source.revision != *expected || self.exists(to)? {
            return Err(DomainError::Conflict);
        }
        let source_path = self.path(from)?;
        let target_path = self.path(to)?;
        move_no_replace(&source_path, &target_path)?;
        let moved = self
            .snapshot(to, MAX_READ)
            .map_err(|_| DomainError::DurabilityUnknown)?;
        if moved.bytes != source.bytes
            || moved.revision.device != expected.device
            || moved.revision.inode != expected.inode
            || moved.revision.size != expected.size
        {
            return Err(DomainError::DurabilityUnknown);
        }
        self.verify_root()?;
        Ok(())
    }

    /// Reverse a previously recorded native file effect. This stays
    /// deliberately narrower than a general delete or move API: the caller
    /// supplies a receipt revision and a generated sibling recovery name.
    pub fn undo_file_checked(
        &self,
        name: &str,
        expected: &FileRevision,
        original: Option<(&[u8], u32)>,
        recovery: &str,
        check: impl Fn(UndoStage) -> Result<()>,
    ) -> Result<UndoOutcome> {
        check(UndoStage::Prepare)?;
        let path = relative(name)?;
        let backup = relative(recovery)?;
        if path.parent() != backup.parent()
            || name == recovery
            || !backup
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".ag1-undo-"))
            || original.is_some_and(|(bytes, mode)| bytes.len() > MAX_READ || mode & !0o777 != 0)
        {
            return Err(DomainError::InvalidInput(
                "native undo recovery path".into(),
            ));
        }

        let _claim = self.claim()?;
        if self.snapshot(name, MAX_READ)?.revision != *expected || self.exists(recovery)? {
            return Err(DomainError::Conflict);
        }
        let parent = path.parent().unwrap_or(Path::new(""));
        let stage_name = parent.join(format!(
            ".ag1-undo-stage-{}-{}",
            std::process::id(),
            UNDO_SERIAL.fetch_add(1, Ordering::Relaxed) + 1
        ));
        let stage_value = stage_name
            .to_str()
            .ok_or(DomainError::PermissionDenied)?
            .to_owned();
        let staged = if let Some((bytes, mode)) = original {
            let stage_path = self.path(&stage_value)?;
            let written = (|| -> Result<Identity> {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&stage_path)
                    .map_err(io_error)?;
                file.write_all(bytes).map_err(io_error)?;
                if mode & 0o222 == 0 {
                    let mut permissions = file.metadata().map_err(io_error)?.permissions();
                    permissions.set_readonly(true);
                    file.set_permissions(permissions).map_err(io_error)?;
                }
                file.sync_all()
                    .map_err(|_| DomainError::DurabilityUnknown)?;
                identity(&file)
            })();
            match written {
                Ok(identity) => Some((stage_path, identity)),
                Err(error) => {
                    let _ = fs::remove_file(&stage_path);
                    return Err(error);
                }
            }
        } else {
            None
        };

        let path = self.path(name)?;
        let backup_path = self.path(recovery)?;
        let uncertain = |error: DomainError| UndoOutcome {
            state: "recovery_required",
            recovery: Some(recovery.into()),
            revision: None,
            detail: Some(format!("{error:?}")),
        };
        let result = (|| -> Result<UndoOutcome> {
            check(UndoStage::BeforeMove)?;
            if self.snapshot(name, MAX_READ)?.revision != *expected {
                return Err(DomainError::Conflict);
            }
            move_no_replace(&path, &backup_path)?;

            // The postimage is now safely displaced. Every later failure is an
            // explicit recovery outcome, never a signal to repeat the Undo.
            if let Err(error) = check(UndoStage::AfterMove).and_then(|_| self.verify_root()) {
                return Ok(uncertain(error));
            }
            let displaced = match self.snapshot(recovery, MAX_READ) {
                Ok(snapshot) => snapshot,
                Err(error) => return Ok(uncertain(error)),
            };
            let mut moved = displaced.revision.clone();
            moved.changed_ns = expected.changed_ns;
            if moved != *expected {
                // Restore only into an absent path. A racing writer always
                // keeps its own file and leaves the recovery copy untouched.
                if move_no_replace(&backup_path, &path).is_ok() {
                    return Ok(UndoOutcome {
                        state: "not_applied",
                        recovery: None,
                        revision: None,
                        detail: Some(
                            "A concurrent edit was preserved. Undo was not applied.".into(),
                        ),
                    });
                }
                return Ok(uncertain(DomainError::Conflict));
            }
            if let Err(error) = check(UndoStage::BeforePublish).and_then(|_| self.verify_root()) {
                return Ok(uncertain(error));
            }
            if let Some((stage_path, _)) = &staged {
                if let Err(error) = move_no_replace(stage_path, &path) {
                    return Ok(uncertain(error));
                }
            } else if self.exists(name).unwrap_or(true) {
                return Ok(uncertain(DomainError::Conflict));
            }
            if let Err(error) = check(UndoStage::AfterPublish).and_then(|_| self.verify_root()) {
                return Ok(uncertain(error));
            }
            let revision = if let Some((bytes, _)) = original {
                match self.snapshot(name, MAX_READ) {
                    Ok(snapshot) if snapshot.bytes == bytes => Some(snapshot.revision),
                    _ => return Ok(uncertain(DomainError::DurabilityUnknown)),
                }
            } else {
                None
            };
            Ok(UndoOutcome {
                state: "completed",
                recovery: Some(recovery.into()),
                revision,
                detail: None,
            })
        })();

        // Remove only a stage file whose identity still matches this call. A
        // later writer can never be deleted merely because it reused a name.
        if let Some((stage_path, staged_identity)) = staged {
            if File::open(&stage_path)
                .ok()
                .and_then(|file| identity(&file).ok())
                .is_some_and(|current| current == staged_identity)
            {
                let _ = fs::remove_file(stage_path);
            }
        }
        result
    }

    pub fn scan(&self, cap: usize) -> Result<Vec<String>> {
        if cap == 0 || cap > MAX_FILES {
            return Err(DomainError::LimitExceeded("file count".into()));
        }
        let mut pending = vec![PathBuf::new()];
        let mut files = Vec::new();
        let mut entries = 0usize;
        while let Some(relative) = pending.pop() {
            let directory = if relative.as_os_str().is_empty() {
                self.root.clone()
            } else {
                self.path(relative.to_str().ok_or(DomainError::PermissionDenied)?)?
            };
            for entry in fs::read_dir(directory).map_err(io_error)? {
                let entry = entry.map_err(io_error)?;
                entries += 1;
                if entries > MAX_FILES * 10 {
                    return Err(DomainError::LimitExceeded("directory entries".into()));
                }
                let name = entry
                    .file_name()
                    .to_str()
                    .ok_or_else(|| DomainError::InvalidInput("non-UTF8 filename".into()))?
                    .to_owned();
                if [".git", "node_modules", "target", ".ag1"].contains(&name.as_str())
                    || name.starts_with(".ag1-write-")
                    || name.starts_with(".ag1-undo-")
                {
                    continue;
                }
                let next = relative.join(&name);
                let metadata = fs::symlink_metadata(entry.path()).map_err(io_error)?;
                if metadata.file_type().is_symlink() || has_reparse_point(&metadata) {
                    continue;
                }
                if metadata.is_dir() {
                    pending.push(next);
                } else if metadata.is_file() {
                    if files.len() == cap {
                        return Err(DomainError::LimitExceeded("file count".into()));
                    }
                    files.push(next.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        files.sort();
        Ok(files)
    }

    pub fn index_paths(&self, paths: &[String], max_bytes: usize) -> Result<SourceIndex> {
        if paths.len() > MAX_FILES || max_bytes > MAX_INDEX_BYTES {
            return Err(DomainError::LimitExceeded("index limits".into()));
        }
        let mut total = 0;
        let mut files = BTreeMap::new();
        for path in paths {
            let snapshot = self.snapshot(path, MAX_READ.min(max_bytes.saturating_sub(total)))?;
            total += snapshot.bytes.len();
            files.insert(path.clone(), snapshot.revision);
        }
        Ok(SourceIndex {
            workspace: self.id.clone(),
            files,
            bytes_read: total,
        })
    }
}

struct WorkspaceClaim<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
    handle: HANDLE,
}

impl Drop for WorkspaceClaim<'_> {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[derive(Debug)]
pub struct SourceIndex {
    pub workspace: WorkspaceId,
    pub files: BTreeMap<String, FileRevision>,
    pub bytes_read: usize,
}

pub fn preserve_text(snapshot: &ByteSnapshot, text: &str) -> Result<Vec<u8>> {
    let old = std::str::from_utf8(&snapshot.bytes)
        .map_err(|_| DomainError::InvalidInput("UTF-8 text required".into()))?;
    let bom = old.starts_with('\u{feff}');
    let crlf = old.contains("\r\n");
    if crlf && old.replace("\r\n", "").contains('\n') {
        return Err(DomainError::InvalidInput(
            "mixed newline style; use explicit byte mutation".into(),
        ));
    }
    let mut output = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    if crlf {
        output = output.replace('\n', "\r\n");
    }
    if bom {
        output.insert(0, '\u{feff}');
    }
    Ok(output.into_bytes())
}

pub struct WorkspaceService {
    roots: Mutex<BTreeMap<WorkspaceId, Arc<Workspace>>>,
    pool: BlockingPool,
}

impl WorkspaceService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            roots: Mutex::new(BTreeMap::new()),
            pool: BlockingPool::new(4, 64)?,
        })
    }

    pub fn register(&self, root: &Path) -> Result<Arc<Workspace>> {
        let root = display_path(root.canonicalize().map_err(io_error)?);
        let directory = open_directory(&root)?;
        if !directory.metadata().map_err(io_error)?.is_dir() {
            return Err(DomainError::InvalidInput(
                "workspace directory required".into(),
            ));
        }
        let root_identity = identity(&directory)?;
        let id = WorkspaceId::new(format!(
            "ws-{:x}",
            Sha256::digest(
                format!(
                    "{}:{}:{}",
                    root.display(),
                    root_identity.volume,
                    root_identity.file
                )
                .as_bytes()
            )
        ))?;
        let mut roots = self.roots.lock().map_err(|_| DomainError::Conflict)?;
        if let Some(existing) = roots.get(&id) {
            return Ok(existing.clone());
        }
        if roots.len() >= 64 {
            return Err(DomainError::LimitExceeded("workspace registry".into()));
        }
        let workspace = Arc::new(Workspace {
            id: id.clone(),
            root,
            root_identity,
            claim: Mutex::new(()),
        });
        roots.insert(id, workspace.clone());
        Ok(workspace)
    }

    pub fn get(&self, id: &WorkspaceId) -> Result<Arc<Workspace>> {
        self.roots
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .get(id)
            .cloned()
            .ok_or(DomainError::NotFound)
    }
}

impl WorkspacePort for WorkspaceService {
    fn read<'a>(
        &'a self,
        workspace: &'a WorkspaceId,
        path: &'a str,
        limit: usize,
    ) -> PortFuture<'a, Vec<u8>> {
        let root = self.get(workspace);
        let path = path.to_owned();
        self.pool
            .submit(move || root?.snapshot(&path, limit).map(|snapshot| snapshot.bytes))
    }
}

impl MutationPort for WorkspaceService {
    fn replace<'a>(
        &'a self,
        workspace: &'a WorkspaceId,
        path: &'a str,
        expected: &'a FileRevision,
        bytes: &'a [u8],
    ) -> PortFuture<'a, FileRevision> {
        let root = self.get(workspace);
        let path = path.to_owned();
        let expected = expected.clone();
        let bytes = bytes.to_vec();
        self.pool
            .submit(move || root?.replace(&path, &expected, &bytes))
    }
}

pub struct WatchHints {
    watched: BTreeSet<PathBuf>,
    pub generation: u64,
    pub rebuild_required: bool,
}

impl WatchHints {
    pub fn new(workspace: &Workspace, paths: &[String]) -> Result<Self> {
        let mut watched = BTreeSet::new();
        for path in paths {
            let path = relative(path)?;
            watched.insert(workspace.root.join(path.parent().unwrap_or(Path::new(""))));
        }
        if watched.len() > 128 {
            return Err(DomainError::LimitExceeded("watch directories".into()));
        }
        Ok(Self {
            watched,
            generation: 0,
            rebuild_required: true,
        })
    }

    pub fn invalidate_overflow(&mut self) {
        self.generation = self.generation.saturating_add(1);
        self.rebuild_required = true;
    }

    pub fn rebuilt(&mut self) {
        self.rebuild_required = false;
    }

    pub fn poll(&mut self) -> Result<bool> {
        // ReadDirectoryChangesW is added with the desktop event loop. Until that
        // layer exists, keep the conservative "rebuild before use" state rather
        // than claiming an inotify-equivalent watch is active.
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_replace_and_stale_revision_are_checked() {
        let root = tempfile::Builder::new()
            .prefix("ag1w-workspace-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        fs::write(root.path().join("note.txt"), b"first").unwrap();
        let service = WorkspaceService::new().unwrap();
        let workspace = service.register(root.path()).unwrap();

        let first = workspace.snapshot("note.txt", 1024).unwrap();
        let next = workspace
            .replace("note.txt", &first.revision, b"second")
            .unwrap();
        assert_eq!(
            workspace.snapshot("note.txt", 1024).unwrap().bytes,
            b"second"
        );
        assert_ne!(first.revision, next);
        assert!(matches!(
            workspace.replace("note.txt", &first.revision, b"third"),
            Err(DomainError::Conflict)
        ));
    }

    #[test]
    fn scan_keeps_paths_relative_and_skips_build_directories() {
        let root = tempfile::Builder::new()
            .prefix("ag1w-workspace-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::create_dir_all(root.path().join("target")).unwrap();
        fs::write(root.path().join("src").join("main.rs"), b"fn main() {}").unwrap();
        fs::write(root.path().join("target").join("ignored"), b"build output").unwrap();
        let service = WorkspaceService::new().unwrap();
        let workspace = service.register(root.path()).unwrap();

        assert_eq!(workspace.scan(10).unwrap(), vec!["src/main.rs"]);
    }

    #[test]
    fn directory_entries_keep_file_kind_without_following_reparse_points() {
        let root = tempfile::Builder::new()
            .prefix("ag1w-workspace-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("note.txt"), b"note").unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(root.path())
            .unwrap();

        let mut entries = workspace.directory_entries("").unwrap();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "nested");
        assert!(entries[0].directory);
        assert!(!entries[0].symlink);
        assert_eq!(entries[1].name, "note.txt");
        assert!(!entries[1].directory);
        assert!(workspace.regular_file("note.txt").unwrap());
        assert!(!workspace.regular_file("nested").unwrap());
    }

    #[test]
    fn checked_create_never_overwrites_and_checked_replace_revises() {
        let root = tempfile::Builder::new()
            .prefix("ag1w-workspace-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(root.path())
            .unwrap();

        let first = workspace
            .create_checked("nested/note.txt", b"first", true, || Ok(()))
            .unwrap();
        assert_eq!(
            workspace.snapshot("nested/note.txt", 1024).unwrap().bytes,
            b"first"
        );
        assert!(matches!(
            workspace.create_checked("nested/note.txt", b"other", true, || Ok(())),
            Err(DomainError::Conflict)
        ));
        let second = workspace
            .replace_checked("nested/note.txt", &first, b"second", || Ok(()))
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(
            workspace.snapshot("nested/note.txt", 1024).unwrap().bytes,
            b"second"
        );
    }

    #[test]
    fn undo_restores_original_bytes_and_preserves_a_racing_writer() {
        let root = tempfile::Builder::new()
            .prefix("ag1w-workspace-")
            .tempdir_in(env!("CARGO_MANIFEST_DIR"))
            .unwrap();
        fs::write(root.path().join("note.txt"), b"after").unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(root.path())
            .unwrap();
        let expected = workspace.snapshot("note.txt", 1024).unwrap().revision;
        let outcome = workspace
            .undo_file_checked(
                "note.txt",
                &expected,
                Some((b"before", 0o600)),
                ".ag1-undo-first",
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(outcome.state, "completed");
        assert_eq!(
            workspace.snapshot("note.txt", 1024).unwrap().bytes,
            b"before"
        );
        assert_eq!(
            workspace.snapshot(".ag1-undo-first", 1024).unwrap().bytes,
            b"after"
        );

        let next = workspace.snapshot("note.txt", 1024).unwrap().revision;
        let root_path = root.path().to_path_buf();
        let race = workspace
            .undo_file_checked(
                "note.txt",
                &next,
                Some((b"older", 0o600)),
                ".ag1-undo-race",
                |stage| {
                    if stage == UndoStage::AfterMove {
                        fs::write(root_path.join("note.txt"), b"racing writer")
                            .map_err(io_error)?;
                    }
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(race.state, "recovery_required");
        assert_eq!(
            workspace.snapshot("note.txt", 1024).unwrap().bytes,
            b"racing writer"
        );
        assert_eq!(
            workspace.snapshot(".ag1-undo-race", 1024).unwrap().bytes,
            b"before"
        );
    }
}
