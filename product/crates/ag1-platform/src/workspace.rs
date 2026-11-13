use crate::{executor::BlockingPool, io_error};
use ag1_core::{
    ByteSnapshot, DomainError, FileRevision, MutationPort, PortFuture, WorkspaceId, WorkspacePort,
};
use rustix::fs::{self as rfs, Mode, OFlags, ResolveFlags};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, Metadata},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const MAX_READ: usize = 8 * 1024 * 1024;
pub const MAX_STREAM_READ: u64 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 1000;
pub const MAX_INDEX_BYTES: usize = 16 * 1024 * 1024;
static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);
#[path = "workspace_undo.rs"]
mod undo;
pub use undo::{UndoOutcome, UndoStage};
#[path = "workspace_git.rs"]
mod git_publication;
pub(crate) use git_publication::{GitEntry, GitSnapshot};

fn errno(error: rustix::io::Errno) -> DomainError {
    if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::XDEV {
        DomainError::PermissionDenied
    } else {
        io_error(error.into())
    }
}
fn relative(value: &str) -> Result<&Path, DomainError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > 4096
        || value.contains('\0')
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(DomainError::InvalidInput(
            "contained relative path required".into(),
        ));
    }
    Ok(path)
}
fn beneath(directory: &File, path: &Path, flags: OFlags, mode: Mode) -> Result<File, DomainError> {
    rfs::openat2(
        directory,
        path,
        flags | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        mode,
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )
    .map(File::from)
    .map_err(errno)
}
fn revision(metadata: &Metadata, sha256: &str) -> FileRevision {
    FileRevision {
        sha256: sha256.into(),
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified_ns: metadata.mtime() as i128 * 1_000_000_000 + metadata.mtime_nsec() as i128,
        changed_ns: metadata.ctime() as i128 * 1_000_000_000 + metadata.ctime_nsec() as i128,
        mode: metadata.mode() & 0o7777,
    }
}

/// A host-only pinned working directory. Keep it alive until the owned child has inherited its cwd.
/// The helper receives a descriptor path owned by this process, never a rediscovered workspace path.
pub struct PinnedDirectory {
    directory: File,
    root: File,
    root_path: PathBuf,
    root_device: u64,
    root_inode: u64,
    relative: PathBuf,
    device: u64,
    inode: u64,
}
impl PinnedDirectory {
    pub(crate) fn relative_path(&self) -> &Path {
        &self.relative
    }
    pub fn process_cwd(&self) -> PathBuf {
        PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            self.directory.as_raw_fd()
        ))
    }
    pub fn verify_identity(&self) -> Result<(), DomainError> {
        let root = std::fs::symlink_metadata(&self.root_path).map_err(io_error)?;
        if root.is_symlink() || root.dev() != self.root_device || root.ino() != self.root_inode {
            return Err(DomainError::Conflict);
        }
        let current = if self.relative.as_os_str().is_empty() {
            self.root.try_clone().map_err(io_error)?
        } else {
            beneath(
                &self.root,
                &self.relative,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?
        };
        let metadata = current.metadata().map_err(io_error)?;
        if metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

/// A canonical root directory capability. Reopening a moved/replaced root is a conflict.
pub struct Workspace {
    id: WorkspaceId,
    root: PathBuf,
    directory: File,
    device: u64,
    inode: u64,
    claim: Mutex<()>,
}
impl Workspace {
    /// Host-only directory capability under a stable conversation storage identity.
    /// This copies the pinned directory, not another workspace's journal or authority.
    pub(crate) fn for_conversation(&self, id: &WorkspaceId) -> Result<Arc<Self>, DomainError> {
        self.verify_root()?;
        Ok(Arc::new(Self {
            id: id.clone(),
            root: self.root.clone(),
            directory: self.directory.try_clone().map_err(io_error)?,
            device: self.device,
            inode: self.inode,
            claim: Mutex::new(()),
        }))
    }
    pub(crate) fn directory_identity(&self) -> (u64, u64) {
        (self.device, self.inode)
    }
    pub fn id(&self) -> &WorkspaceId {
        &self.id
    }
    /// Directory identity is distinct from the conversation journal owner after
    /// a user changes the working folder. Only a verified native root supplies it.
    pub(crate) fn canonical_id(&self) -> Result<WorkspaceId, DomainError> {
        self.verify_root()?;
        WorkspaceId::new(format!(
            "ws-{:x}",
            Sha256::digest(
                format!("{}:{}:{}", self.root.display(), self.device, self.inode).as_bytes()
            )
        ))
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn verify_root(&self) -> Result<(), DomainError> {
        let metadata = std::fs::symlink_metadata(&self.root).map_err(io_error)?;
        if metadata.is_symlink() || metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub fn pin_directory(&self, name: &str) -> Result<PinnedDirectory, DomainError> {
        self.verify_root()?;
        let relative = if name.is_empty() {
            PathBuf::new()
        } else {
            relative(name)?.to_path_buf()
        };
        let directory = if relative.as_os_str().is_empty() {
            self.directory.try_clone().map_err(io_error)?
        } else {
            beneath(
                &self.directory,
                &relative,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?
        };
        let metadata = directory.metadata().map_err(io_error)?;
        let pinned = PinnedDirectory {
            directory,
            root: self.directory.try_clone().map_err(io_error)?,
            root_path: self.root.clone(),
            root_device: self.device,
            root_inode: self.inode,
            relative,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        pinned.verify_identity()?;
        Ok(pinned)
    }
    pub fn exists(&self, name: &str) -> Result<bool, DomainError> {
        self.verify_root()?;
        if name.is_empty() {
            return Ok(true);
        }
        match rfs::openat2(
            &self.directory,
            relative(name)?,
            OFlags::PATH | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        ) {
            Ok(fd) => {
                if File::from(fd).metadata().map_err(io_error)?.is_symlink() {
                    return Err(DomainError::PermissionDenied);
                }
                self.verify_root()?;
                Ok(true)
            }
            Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR) => Ok(false),
            Err(e) => Err(errno(e)),
        }
    }
    /// Capability-relative parent check; symlinks are rejected, never followed.
    pub fn directory_exists(&self, name: &str) -> Result<bool, DomainError> {
        self.verify_root()?;
        if name.is_empty() {
            return Ok(true);
        }
        let path = relative(name)?;
        match rfs::openat2(
            &self.directory,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        ) {
            Ok(_) => {
                self.verify_root()?;
                Ok(true)
            }
            Err(rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR) => Ok(false),
            Err(e) => Err(errno(e)),
        }
    }
    /// Inspect file kind without opening content or following symlink targets.
    pub(crate) fn regular_file(&self, name: &str) -> Result<bool, DomainError> {
        self.verify_root()?;
        let file = beneath(
            &self.directory,
            relative(name)?,
            OFlags::PATH,
            Mode::empty(),
        )?;
        let regular = file.metadata().map_err(io_error)?.is_file();
        self.verify_root()?;
        Ok(regular)
    }
    /// One bounded descriptor-relative directory observation. Symlink targets are not followed.
    pub fn directory_entries(
        &self,
        name: &str,
    ) -> Result<Vec<ag1_core::tools::list::Entry>, DomainError> {
        self.verify_root()?;
        let directory = if name.is_empty() {
            self.directory.try_clone().map_err(io_error)?
        } else {
            beneath(
                &self.directory,
                relative(name)?,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?
        };
        let before = directory.metadata().map_err(io_error)?;
        let mut entries = Vec::new();
        let mut bytes = 0;
        for entry in rfs::Dir::read_from(&directory).map_err(errno)? {
            let entry = entry.map_err(errno)?;
            let name = entry
                .file_name()
                .to_str()
                .map_err(|_| DomainError::InvalidInput("non-UTF8 filename".into()))?;
            if [".", ".."].contains(&name) {
                continue;
            }
            if entries.len() >= 10000 {
                return Err(DomainError::LimitExceeded("directory entries".into()));
            }
            bytes += name.len();
            if bytes > 4 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("directory names".into()));
            }
            let metadata =
                rfs::statat(&directory, name, rfs::AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
            let kind = rfs::FileType::from_raw_mode(metadata.st_mode);
            entries.push(ag1_core::tools::list::Entry {
                name: name.into(),
                directory: kind == rfs::FileType::Directory,
                symlink: kind == rfs::FileType::Symlink,
            });
        }
        let after = directory.metadata().map_err(io_error)?;
        if revision(&before, "") != revision(&after, "") {
            return Err(DomainError::Conflict);
        }
        let current = if name.is_empty() {
            self.directory.try_clone().map_err(io_error)?
        } else {
            beneath(
                &self.directory,
                relative(name)?,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?
        };
        if revision(&current.metadata().map_err(io_error)?, "") != revision(&after, "") {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(entries)
    }
    pub fn snapshot(&self, name: &str, limit: usize) -> Result<ByteSnapshot, DomainError> {
        self.snapshot_with_barrier(name, limit, || {})
    }
    /// Host instrumentation seam: runs after byte read and before identity validation.
    /// Never exposed as a renderer command. Used to force mutation races deterministically.
    pub fn snapshot_with_barrier(
        &self,
        name: &str,
        limit: usize,
        after_read: impl FnOnce(),
    ) -> Result<ByteSnapshot, DomainError> {
        self.verify_root()?;
        if limit == 0 || limit > MAX_READ {
            return Err(DomainError::LimitExceeded("read bytes".into()));
        }
        let path = relative(name)?;
        let mut file = beneath(
            &self.directory,
            path,
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        let before = file.metadata().map_err(io_error)?;
        if !before.is_file() || before.len() > limit as u64 {
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
        let after = file.metadata().map_err(io_error)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if revision(&before, &digest) != revision(&after, &digest) {
            return Err(DomainError::Conflict);
        }
        // Reopen through the root capability to detect path replacement during the read.
        let current = beneath(
            &self.directory,
            path,
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )?
        .metadata()
        .map_err(io_error)?;
        if revision(&current, &digest) != revision(&after, &digest) {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(ByteSnapshot {
            revision: revision(&after, &digest),
            bytes,
        })
    }
    /// Stream regular-file bytes through one descriptor while hashing the same chunks.
    /// A successful return certifies before/after metadata, path identity and the root.
    pub fn stream(
        &self,
        name: &str,
        limit: u64,
        mut consume: impl FnMut(&[u8]) -> Result<(), DomainError>,
    ) -> Result<FileRevision, DomainError> {
        self.stream_with_barrier(name, limit, &mut consume, || {})
    }
    pub fn stream_with_barrier(
        &self,
        name: &str,
        limit: u64,
        mut consume: impl FnMut(&[u8]) -> Result<(), DomainError>,
        after_read: impl FnOnce(),
    ) -> Result<FileRevision, DomainError> {
        self.stream_bounded(name, limit, MAX_STREAM_READ, &mut consume, after_read)
    }
    pub(crate) fn stream_runtime_material(
        &self,
        name: &str,
        limit: u64,
        consume: impl FnMut(&[u8]) -> Result<(), DomainError>,
    ) -> Result<FileRevision, DomainError> {
        self.stream_bounded(name, limit, 2 * 1024 * 1024 * 1024, consume, || {})
    }
    fn stream_bounded(
        &self,
        name: &str,
        limit: u64,
        maximum: u64,
        mut consume: impl FnMut(&[u8]) -> Result<(), DomainError>,
        after_read: impl FnOnce(),
    ) -> Result<FileRevision, DomainError> {
        self.verify_root()?;
        if limit == 0 || limit > maximum {
            return Err(DomainError::LimitExceeded("streamed file bytes".into()));
        }
        let path = relative(name)?;
        let mut file = beneath(
            &self.directory,
            path,
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        let before = file.metadata().map_err(io_error)?;
        if !before.is_file() || before.len() > limit {
            return Err(DomainError::LimitExceeded("regular streamed file".into()));
        }
        let mut buffer = [0u8; 65536];
        let mut hash = Sha256::new();
        let mut count = 0;
        loop {
            let n = file.read(&mut buffer).map_err(io_error)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > limit {
                return Err(DomainError::LimitExceeded("streamed file bytes".into()));
            }
            hash.update(&buffer[..n]);
            consume(&buffer[..n])?;
        }
        after_read();
        let after = file.metadata().map_err(io_error)?;
        let digest = format!("{:x}", hash.finalize());
        if count != after.len() || revision(&before, &digest) != revision(&after, &digest) {
            return Err(DomainError::Conflict);
        }
        let current = beneath(
            &self.directory,
            path,
            OFlags::RDONLY | OFlags::NONBLOCK,
            Mode::empty(),
        )?
        .metadata()
        .map_err(io_error)?;
        if revision(&current, &digest) != revision(&after, &digest) {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(revision(&after, &digest))
    }
    /// Serializes workspace mutations and Git observations without retaining a claim across await/UI work.
    pub(crate) fn claim(&self) -> Result<WorkspaceClaim<'_>, DomainError> {
        let guard = self.claim.try_lock().map_err(|_| DomainError::Conflict)?;
        self.verify_root()?;
        // A fresh file description gives independent flock ownership, including other processes.
        let file = File::open(&self.root).map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        if metadata.dev() != self.device || metadata.ino() != self.inode {
            return Err(DomainError::Conflict);
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(DomainError::Conflict);
        }
        Ok(WorkspaceClaim {
            _guard: guard,
            _file: file,
        })
    }
    pub fn replace(
        &self,
        name: &str,
        expected: &FileRevision,
        bytes: &[u8],
    ) -> Result<FileRevision, DomainError> {
        self.replace_checked(name, expected, bytes, || Ok(()))
    }
    /// Host cancellation/failure seam checked before publication. No renderer callback is accepted.
    pub fn replace_checked(
        &self,
        name: &str,
        expected: &FileRevision,
        bytes: &[u8],
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<FileRevision, DomainError> {
        self.replace_guarded(name, expected, bytes, |_| check())
    }
    /// The guard may ignore only this operation's exact temporary path when
    /// checking Git dirt; all other files remain part of the conflict fence.
    pub(crate) fn replace_guarded(
        &self,
        name: &str,
        expected: &FileRevision,
        bytes: &[u8],
        check: impl Fn(Option<&str>) -> Result<(), DomainError>,
    ) -> Result<FileRevision, DomainError> {
        check(None)?;
        if bytes.len() > MAX_READ {
            return Err(DomainError::LimitExceeded("mutation bytes".into()));
        }
        let _claim = self.claim()?;
        let path = relative(name)?;
        if self.snapshot(name, MAX_READ)?.revision != *expected {
            return Err(DomainError::Conflict);
        }
        let parent = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(parent) => beneath(
                &self.directory,
                parent,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?,
            None => self.directory.try_clone().map_err(io_error)?,
        };
        let temporary = format!(
            ".ag1-write-{}-{}",
            std::process::id(),
            TEMP_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let temporary_relative = path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&temporary)
            .to_string_lossy()
            .into_owned();
        let mut file = beneath(
            &parent,
            Path::new(&temporary),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )?;
        let result = (|| {
            file.write_all(bytes).map_err(io_error)?;
            check(Some(&temporary_relative))?;
            // Preserve executable/read/write bits. Set-ID/sticky are never silently copied to new bytes.
            if expected.mode & 0o7000 != 0 {
                return Err(DomainError::PermissionDenied);
            }
            rfs::fchmod(&file, Mode::from_raw_mode(expected.mode)).map_err(errno)?;
            file.sync_all().map_err(io_error)?;
            check(Some(&temporary_relative))?;
            if self.snapshot(name, MAX_READ)?.revision != *expected {
                return Err(DomainError::Conflict);
            }
            self.verify_root()?;
            rfs::renameat(&parent, &temporary, &parent, path.file_name().unwrap())
                .map_err(errno)?;
            parent
                .sync_all()
                .map_err(|_| DomainError::DurabilityUnknown)?;
            let snapshot = self
                .snapshot(name, MAX_READ)
                .map_err(|_| DomainError::DurabilityUnknown)?;
            if snapshot.bytes != bytes {
                return Err(DomainError::DurabilityUnknown);
            }
            Ok(snapshot.revision)
        })();
        let _ = rfs::unlinkat(&parent, &temporary, rfs::AtFlags::empty());
        result
    }
    /// Publish a previously absent file without overwriting a competing creator.
    /// Parent directories are created only when explicitly requested by an authorized write.
    pub fn create_checked(
        &self,
        name: &str,
        bytes: &[u8],
        parents: bool,
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<FileRevision, DomainError> {
        check()?;
        if bytes.len() > MAX_READ {
            return Err(DomainError::LimitExceeded("mutation bytes".into()));
        }
        let path = relative(name)?;
        if path.components().count() > 32 {
            return Err(DomainError::LimitExceeded("parent depth".into()));
        }
        let _claim = self.claim()?;
        match self.snapshot(name, MAX_READ) {
            Ok(_) => return Err(DomainError::Conflict),
            Err(DomainError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let mut parent = self.directory.try_clone().map_err(io_error)?;
        let mut prefix = PathBuf::new();
        let segments: Vec<_> = path.components().collect();
        for segment in &segments[..segments.len() - 1] {
            check()?;
            let segment = segment.as_os_str();
            prefix.push(segment);
            match beneath(
                &parent,
                Path::new(segment),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            ) {
                Ok(next) => parent = next,
                Err(DomainError::NotFound) if parents => {
                    match rfs::mkdirat(&parent, segment, Mode::from_raw_mode(0o777)) {
                        Ok(()) => {}
                        Err(rustix::io::Errno::EXIST) => {}
                        Err(e) => return Err(errno(e)),
                    }
                    parent
                        .sync_all()
                        .map_err(|_| DomainError::DurabilityUnknown)?;
                    parent = beneath(
                        &parent,
                        Path::new(segment),
                        OFlags::RDONLY | OFlags::DIRECTORY,
                        Mode::empty(),
                    )?;
                }
                Err(e) => return Err(e),
            }
        }
        let temporary = format!(
            ".ag1-create-{}-{}",
            std::process::id(),
            TEMP_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let mut file = beneath(
            &parent,
            Path::new(&temporary),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(0o666),
        )?;
        let result = (|| {
            file.write_all(bytes).map_err(io_error)?;
            check()?;
            file.sync_all().map_err(io_error)?;
            check()?;
            self.verify_root()?;
            let current = if prefix.as_os_str().is_empty() {
                self.directory.try_clone().map_err(io_error)?
            } else {
                beneath(
                    &self.directory,
                    &prefix,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                )?
            };
            let expected = parent.metadata().map_err(io_error)?;
            let observed = current.metadata().map_err(io_error)?;
            if expected.dev() != observed.dev() || expected.ino() != observed.ino() {
                return Err(DomainError::Conflict);
            }
            rfs::renameat_with(
                &parent,
                &temporary,
                &parent,
                path.file_name().unwrap(),
                rfs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| {
                if e == rustix::io::Errno::EXIST {
                    DomainError::Conflict
                } else {
                    errno(e)
                }
            })?;
            parent
                .sync_all()
                .map_err(|_| DomainError::DurabilityUnknown)?;
            let snapshot = self
                .snapshot(name, MAX_READ)
                .map_err(|_| DomainError::DurabilityUnknown)?;
            if snapshot.bytes != bytes {
                return Err(DomainError::DurabilityUnknown);
            }
            Ok(snapshot.revision)
        })();
        let _ = rfs::unlinkat(&parent, &temporary, rfs::AtFlags::empty());
        result
    }
    /// Publish a complete new directory and one document with NOREPLACE. This
    /// supports resource scaffolds: competing creators cannot merge directories,
    /// and discovery cannot observe a partially written SKILL.md.
    pub fn create_directory_document_checked(
        &self,
        directory: &str,
        document: &str,
        bytes: &[u8],
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<FileRevision, DomainError> {
        check()?;
        let path = relative(directory)?;
        if path.components().count() > 30
            || bytes.len() > 1024 * 1024
            || relative(document)?.components().count() != 1
        {
            return Err(DomainError::LimitExceeded("resource scaffold".into()));
        }
        let _claim = self.claim()?;
        if self.exists(directory)? {
            return Err(DomainError::Conflict);
        }
        let mut parent = self.directory.try_clone().map_err(io_error)?;
        let mut prefix = PathBuf::new();
        let segments: Vec<_> = path.components().collect();
        for segment in &segments[..segments.len() - 1] {
            check()?;
            let segment = segment.as_os_str();
            prefix.push(segment);
            match beneath(
                &parent,
                Path::new(segment),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            ) {
                Ok(next) => parent = next,
                Err(DomainError::NotFound) => {
                    match rfs::mkdirat(&parent, segment, Mode::from_raw_mode(0o777)) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                        Err(e) => return Err(errno(e)),
                    }
                    parent
                        .sync_all()
                        .map_err(|_| DomainError::DurabilityUnknown)?;
                    parent = beneath(
                        &parent,
                        Path::new(segment),
                        OFlags::RDONLY | OFlags::DIRECTORY,
                        Mode::empty(),
                    )?;
                }
                Err(e) => return Err(e),
            }
        }
        check()?;
        let temporary = format!(
            ".ag1-resource-{}-{}",
            std::process::id(),
            TEMP_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        rfs::mkdirat(&parent, &temporary, Mode::from_raw_mode(0o777)).map_err(errno)?;
        let staging = match beneath(
            &parent,
            Path::new(&temporary),
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        ) {
            Ok(file) => file,
            Err(error) => {
                let _ = rfs::unlinkat(&parent, &temporary, rfs::AtFlags::REMOVEDIR);
                return Err(error);
            }
        };
        let staging_identity = staging.metadata().map_err(io_error)?;
        let staging_is_current = || -> Result<bool, DomainError> {
            let current = beneath(
                &parent,
                Path::new(&temporary),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?
            .metadata()
            .map_err(io_error)?;
            Ok(current.dev() == staging_identity.dev() && current.ino() == staging_identity.ino())
        };
        let mut published = false;
        let result = (|| {
            let mut file = beneath(
                &staging,
                Path::new(document),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                Mode::from_raw_mode(0o666),
            )?;
            file.write_all(bytes).map_err(io_error)?;
            check()?;
            file.sync_all().map_err(io_error)?;
            staging.sync_all().map_err(io_error)?;
            check()?;
            self.verify_root()?;
            let current = if prefix.as_os_str().is_empty() {
                self.directory.try_clone().map_err(io_error)?
            } else {
                beneath(
                    &self.directory,
                    &prefix,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                )?
            };
            let expected = parent.metadata().map_err(io_error)?;
            let observed = current.metadata().map_err(io_error)?;
            if expected.dev() != observed.dev() || expected.ino() != observed.ino() {
                return Err(DomainError::Conflict);
            }
            if !staging_is_current()? {
                return Err(DomainError::Conflict);
            }
            rfs::renameat_with(
                &parent,
                &temporary,
                &parent,
                path.file_name().unwrap(),
                rfs::RenameFlags::NOREPLACE,
            )
            .map_err(|e| {
                if e == rustix::io::Errno::EXIST {
                    DomainError::Conflict
                } else {
                    errno(e)
                }
            })?;
            published = true;
            parent
                .sync_all()
                .map_err(|_| DomainError::DurabilityUnknown)?;
            let snapshot = self
                .snapshot(&format!("{directory}/{document}"), 1024 * 1024)
                .map_err(|_| DomainError::DurabilityUnknown)?;
            if snapshot.bytes != bytes {
                return Err(DomainError::DurabilityUnknown);
            }
            Ok(snapshot.revision)
        })();
        // Cleanup only our still-unpublished staging directory. The descriptor may
        // now name the published directory; never unlink through it after success.
        if !published && staging_is_current().unwrap_or(false) {
            let _ = rfs::unlinkat(&staging, document, rfs::AtFlags::empty());
            let _ = rfs::unlinkat(&parent, &temporary, rfs::AtFlags::REMOVEDIR);
        }
        result
    }
    /// Capability-relative directory creation for an already authorized package
    /// operation. Existing symlink components are never followed.
    pub fn create_directories_checked(
        &self,
        name: &str,
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        check()?;
        if name.is_empty() {
            return self.verify_root();
        }
        let path = relative(name)?;
        if path.components().count() > 32 {
            return Err(DomainError::LimitExceeded("package directory depth".into()));
        }
        let _claim = self.claim()?;
        self.exists(name)?;
        let mut parent = self.directory.try_clone().map_err(io_error)?;
        for component in path.components() {
            check()?;
            let name = component.as_os_str();
            match beneath(
                &parent,
                Path::new(name),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            ) {
                Ok(next) => parent = next,
                Err(DomainError::NotFound) => {
                    match rfs::mkdirat(&parent, name, Mode::from_raw_mode(0o777)) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                        Err(e) => return Err(errno(e)),
                    }
                    parent
                        .sync_all()
                        .map_err(|_| DomainError::DurabilityUnknown)?;
                    parent = beneath(
                        &parent,
                        Path::new(name),
                        OFlags::RDONLY | OFlags::DIRECTORY,
                        Mode::empty(),
                    )?;
                }
                Err(e) => return Err(e),
            }
        }
        self.verify_root()?;
        let current = beneath(
            &self.directory,
            path,
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )?
        .metadata()
        .map_err(io_error)?;
        let expected = parent.metadata().map_err(io_error)?;
        if current.dev() != expected.dev() || current.ino() != expected.ino() {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    /// Prune one empty package ancestor. Never removes the workspace root or
    /// recursively deletes a directory that gained contents while being checked.
    pub fn remove_empty_directory_checked(
        &self,
        expected: PinnedDirectory,
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<bool, DomainError> {
        check()?;
        if expected.root_path != self.root
            || expected.root_device != self.device
            || expected.root_inode != self.inode
            || expected.relative.as_os_str().is_empty()
        {
            return Err(DomainError::PermissionDenied);
        }
        let _claim = self.claim()?;
        expected.verify_identity()?;
        let path = &expected.relative;
        let parent = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => beneath(
                &self.directory,
                p,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?,
            None => self.directory.try_clone().map_err(io_error)?,
        };
        check()?;
        expected.verify_identity()?;
        match rfs::unlinkat(&parent, path.file_name().unwrap(), rfs::AtFlags::REMOVEDIR) {
            Ok(()) => {
                parent
                    .sync_all()
                    .map_err(|_| DomainError::DurabilityUnknown)?;
                Ok(true)
            }
            Err(rustix::io::Errno::NOTEMPTY | rustix::io::Errno::EXIST) => Ok(false),
            Err(error) => Err(errno(error)),
        }
    }
    /// Remove a previously pinned owned package directory. Detachment is atomic;
    /// cleanup is bounded and never follows a link or crosses a nested mount.
    /// Failures after detachment report uncertainty and may leave hidden owned
    /// staging data for recovery; they never claim that the original tree remains.
    pub fn remove_directory_tree_checked(
        &self,
        expected: PinnedDirectory,
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        check()?;
        if expected.root_path != self.root
            || expected.root_device != self.device
            || expected.root_inode != self.inode
            || expected.relative.as_os_str().is_empty()
        {
            return Err(DomainError::PermissionDenied);
        }
        let _claim = self.claim()?;
        expected.verify_identity()?;
        let source = &expected.relative;
        let parent = match source.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => beneath(
                &self.directory,
                p,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?,
            None => self.directory.try_clone().map_err(io_error)?,
        };
        let temporary = format!(
            ".ag1-package-remove-{}-{}",
            std::process::id(),
            TEMP_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        check()?;
        expected.verify_identity()?;
        rfs::renameat_with(
            &parent,
            source.file_name().unwrap(),
            &parent,
            &temporary,
            rfs::RenameFlags::NOREPLACE,
        )
        .map_err(errno)?;
        let detached = beneath(
            &parent,
            Path::new(&temporary),
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .map_err(|_| DomainError::DurabilityUnknown)?;
        let identity = detached
            .metadata()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        if identity.dev() != expected.device || identity.ino() != expected.inode {
            return match rfs::renameat_with(
                &parent,
                &temporary,
                &parent,
                source.file_name().unwrap(),
                rfs::RenameFlags::NOREPLACE,
            ) {
                Ok(()) => {
                    parent
                        .sync_all()
                        .map_err(|_| DomainError::DurabilityUnknown)?;
                    Err(DomainError::Conflict)
                }
                Err(_) => Err(DomainError::DurabilityUnknown),
            };
        }
        parent
            .sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        fn remove_contents(
            root: &File,
            dir: &File,
            prefix: &Path,
            depth: usize,
            remaining: &mut usize,
            check: &impl Fn() -> Result<(), DomainError>,
        ) -> Result<(), DomainError> {
            if depth > 32 {
                return Err(DomainError::LimitExceeded("package removal depth".into()));
            }
            let mut names = vec![];
            for entry in rfs::Dir::read_from(dir).map_err(errno)? {
                check()?;
                let entry = entry.map_err(errno)?;
                let name = entry.file_name();
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    continue;
                }
                *remaining = remaining
                    .checked_sub(1)
                    .ok_or_else(|| DomainError::LimitExceeded("package removal entries".into()))?;
                names.push(name.to_owned());
            }
            for name in names {
                check()?;
                // Recheck that a recursively opened directory still belongs to
                // the detached tree before mutating its children.
                if !prefix.as_os_str().is_empty() {
                    let current = beneath(
                        root,
                        prefix,
                        OFlags::RDONLY | OFlags::DIRECTORY,
                        Mode::empty(),
                    )?
                    .metadata()
                    .map_err(io_error)?;
                    let expected = dir.metadata().map_err(io_error)?;
                    if current.dev() != expected.dev() || current.ino() != expected.ino() {
                        return Err(DomainError::Conflict);
                    }
                }
                let stat =
                    rfs::statat(dir, &name, rfs::AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
                if rfs::FileType::from_raw_mode(stat.st_mode) == rfs::FileType::Directory {
                    let child: File = rfs::openat2(
                        dir,
                        &name,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                        Mode::empty(),
                        ResolveFlags::BENEATH
                            | ResolveFlags::NO_SYMLINKS
                            | ResolveFlags::NO_MAGICLINKS
                            | ResolveFlags::NO_XDEV,
                    )
                    .map_err(errno)?
                    .into();
                    use std::os::unix::ffi::OsStrExt;
                    let relative = prefix.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
                    remove_contents(root, &child, &relative, depth + 1, remaining, check)?;
                    rfs::unlinkat(dir, &name, rfs::AtFlags::REMOVEDIR).map_err(errno)?;
                } else {
                    rfs::unlinkat(dir, &name, rfs::AtFlags::empty()).map_err(errno)?;
                }
            }
            dir.sync_all().map_err(io_error)?;
            Ok(())
        }
        let attached = || -> Result<(), DomainError> {
            check()?;
            self.verify_root()?;
            let current = beneath(
                &parent,
                Path::new(&temporary),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?
            .metadata()
            .map_err(io_error)?;
            if current.dev() != identity.dev() || current.ino() != identity.ino() {
                return Err(DomainError::Conflict);
            }
            Ok(())
        };
        remove_contents(
            &detached,
            &detached,
            Path::new(""),
            0,
            &mut 20000,
            &attached,
        )
        .map_err(|_| DomainError::DurabilityUnknown)?;
        attached().map_err(|_| DomainError::DurabilityUnknown)?;
        rfs::unlinkat(&parent, &temporary, rfs::AtFlags::REMOVEDIR)
            .map_err(|_| DomainError::DurabilityUnknown)?;
        parent
            .sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        self.verify_root()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        Ok(())
    }
    pub fn rename(&self, from: &str, to: &str, expected: &FileRevision) -> Result<(), DomainError> {
        let _claim = self.claim()?;
        let source = relative(from)?;
        let target = relative(to)?;
        if self.snapshot(from, MAX_READ)?.revision != *expected {
            return Err(DomainError::Conflict);
        }
        let open_parent = |path: &Path| match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(parent) => beneath(
                &self.directory,
                parent,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            ),
            None => self.directory.try_clone().map_err(io_error),
        };
        let source_parent = open_parent(source)?;
        let target_parent = open_parent(target)?;
        rfs::renameat_with(
            &source_parent,
            source.file_name().unwrap(),
            &target_parent,
            target.file_name().unwrap(),
            rfs::RenameFlags::NOREPLACE,
        )
        .map_err(errno)?;
        source_parent
            .sync_all()
            .and_then(|_| target_parent.sync_all())
            .map_err(|_| DomainError::DurabilityUnknown)?;
        Ok(())
    }
    /// Publish a native-owned completed clone beside its staging name. No
    /// replacement, cross-parent move or implicit directory merge is allowed.
    pub(crate) fn publish_owned_directory_checked(
        &self,
        source: &Workspace,
        target: &str,
        parent: &PinnedDirectory,
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        let target = relative(target)?;
        if target.parent().unwrap_or(Path::new("")) != parent.relative_path()
            || source.root().parent() != Some(self.root().join(parent.relative_path()).as_path())
        {
            return Err(DomainError::PermissionDenied);
        }
        let _claim = self.claim()?;
        check()?;
        self.verify_root()?;
        parent.verify_identity()?;
        source.verify_root()?;
        let name = source
            .root()
            .file_name()
            .ok_or(DomainError::PermissionDenied)?;
        let current = beneath(
            &parent.directory,
            Path::new(name),
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )?
        .metadata()
        .map_err(io_error)?;
        if (current.dev(), current.ino()) != source.directory_identity() {
            return Err(DomainError::Conflict);
        }
        rfs::renameat_with(
            &parent.directory,
            name,
            &parent.directory,
            target.file_name().unwrap(),
            rfs::RenameFlags::NOREPLACE,
        )
        .map_err(|e| {
            if e == rustix::io::Errno::EXIST {
                DomainError::Conflict
            } else {
                errno(e)
            }
        })?;
        parent
            .directory
            .sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        let published = beneath(
            &parent.directory,
            Path::new(target.file_name().unwrap()),
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )
        .map_err(|_| DomainError::DurabilityUnknown)?
        .metadata()
        .map_err(|_| DomainError::DurabilityUnknown)?;
        if (published.dev(), published.ino()) != source.directory_identity() {
            return Err(DomainError::DurabilityUnknown);
        }
        check().map_err(|_| DomainError::DurabilityUnknown)?;
        parent
            .verify_identity()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        Ok(())
    }
    /// Flush a bounded, native-owned clone tree before publishing its name.
    /// Symlink targets and special files are never opened or followed.
    pub(crate) fn sync_owned_tree_checked(
        &self,
        check: impl Fn() -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        let mut pending = vec![PathBuf::new()];
        let mut directories = Vec::new();
        let mut count = 0;
        while let Some(path) = pending.pop() {
            check()?;
            self.verify_root()?;
            if path.components().count() > 64 {
                return Err(DomainError::LimitExceeded("clone tree depth".into()));
            }
            let directory = if path.as_os_str().is_empty() {
                self.directory.try_clone().map_err(io_error)?
            } else {
                beneath(
                    &self.directory,
                    &path,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                )?
            };
            for entry in rfs::Dir::read_from(&directory).map_err(errno)? {
                check()?;
                let entry = entry.map_err(errno)?;
                let name = entry.file_name();
                if name.to_bytes() == b"." || name.to_bytes() == b".." {
                    continue;
                }
                count += 1;
                if count > 20000 {
                    return Err(DomainError::LimitExceeded("clone tree entries".into()));
                }
                let name = std::ffi::OsStr::from_bytes(name.to_bytes());
                let stat =
                    rfs::statat(&directory, name, rfs::AtFlags::SYMLINK_NOFOLLOW).map_err(errno)?;
                match rfs::FileType::from_raw_mode(stat.st_mode) {
                    rfs::FileType::Directory => pending.push(path.join(name)),
                    rfs::FileType::RegularFile => {
                        beneath(&directory, Path::new(name), OFlags::RDONLY, Mode::empty())?
                            .sync_all()
                            .map_err(io_error)?;
                    }
                    rfs::FileType::Symlink => {}
                    _ => return Err(DomainError::PermissionDenied),
                }
            }
            directories.push(directory);
        }
        for directory in directories.into_iter().rev() {
            check()?;
            directory.sync_all().map_err(io_error)?;
        }
        self.verify_root()
    }
    /// Non-Git fallback scan. Git repositories should pass GitService::files() to index_paths().
    pub fn scan(&self, cap: usize) -> Result<Vec<String>, DomainError> {
        if cap == 0 || cap > MAX_FILES {
            return Err(DomainError::LimitExceeded("file count".into()));
        }
        let mut pending = vec![PathBuf::new()];
        let mut files = vec![];
        let mut entries = 0;
        while let Some(path) = pending.pop() {
            if pending.len() > MAX_FILES || path.components().count() > 32 {
                return Err(DomainError::LimitExceeded("directory traversal".into()));
            }
            let directory = if path.as_os_str().is_empty() {
                self.directory.try_clone().map_err(io_error)?
            } else {
                beneath(
                    &self.directory,
                    &path,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                )?
            };
            for entry in rfs::Dir::read_from(&directory).map_err(errno)? {
                let entry = entry.map_err(errno)?;
                entries += 1;
                if entries > MAX_FILES * 10 {
                    return Err(DomainError::LimitExceeded("directory entries".into()));
                }
                let name = entry
                    .file_name()
                    .to_str()
                    .map_err(|_| DomainError::InvalidInput("non-UTF8 filename".into()))?;
                if [".", "..", ".git", "node_modules", "target", ".ag1"].contains(&name)
                    || name.starts_with(".ag1-write-")
                {
                    continue;
                }
                let child = path.join(name);
                if entry.file_type() == rfs::FileType::Directory {
                    pending.push(child);
                } else if entry.file_type() == rfs::FileType::RegularFile {
                    if files.len() >= cap {
                        return Err(DomainError::LimitExceeded("file count".into()));
                    }
                    files.push(child.to_string_lossy().into_owned());
                }
            }
        }
        files.sort();
        Ok(files)
    }
    pub fn index_paths(
        &self,
        paths: &[String],
        max_bytes: usize,
    ) -> Result<SourceIndex, DomainError> {
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
    pub fn search(
        &self,
        paths: &[String],
        needle: &[u8],
        limit: usize,
    ) -> Result<Vec<(String, usize)>, DomainError> {
        if needle.is_empty()
            || needle.len() > 4096
            || limit == 0
            || limit > 1000
            || paths.len() > MAX_FILES
        {
            return Err(DomainError::LimitExceeded("search limits".into()));
        }
        let mut matches = vec![];
        let mut total = 0;
        for path in paths {
            let snapshot = self.snapshot(path, MAX_READ.min(MAX_INDEX_BYTES - total))?;
            total += snapshot.bytes.len();
            for (offset, window) in snapshot.bytes.windows(needle.len()).enumerate() {
                if window == needle {
                    if matches.len() == limit {
                        return Err(DomainError::LimitExceeded("search matches".into()));
                    }
                    matches.push((path.clone(), offset));
                }
            }
        }
        Ok(matches)
    }
}
pub(crate) struct WorkspaceClaim<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
    _file: File,
}
impl Drop for WorkspaceClaim<'_> {
    fn drop(&mut self) {
        // Explicit unlock also releases the open-file-description lock during a concurrent fork/exec.
        unsafe {
            libc::flock(self._file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug)]
pub struct SourceIndex {
    pub workspace: WorkspaceId,
    pub files: BTreeMap<String, FileRevision>,
    pub bytes_read: usize,
}

pub fn preserve_text(snapshot: &ByteSnapshot, text: &str) -> Result<Vec<u8>, DomainError> {
    let old = std::str::from_utf8(&snapshot.bytes)
        .map_err(|_| DomainError::InvalidInput("UTF-8 text required".into()))?;
    let has_bom = old.starts_with('\u{feff}');
    let crlf = old.contains("\r\n");
    if crlf && old.replace("\r\n", "").contains('\n') {
        return Err(DomainError::InvalidInput(
            "mixed newline style; use explicit byte mutation".into(),
        ));
    }
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut result = if crlf {
        normalized.replace('\n', "\r\n")
    } else {
        normalized
    };
    if has_bom {
        result.insert(0, '\u{feff}');
    }
    Ok(result.into_bytes())
}

pub struct WorkspaceService {
    roots: Mutex<BTreeMap<WorkspaceId, Arc<Workspace>>>,
    pool: BlockingPool,
}
impl WorkspaceService {
    pub fn new() -> Result<Self, DomainError> {
        Ok(Self {
            roots: Mutex::new(BTreeMap::new()),
            pool: BlockingPool::new(4, 64)?,
        })
    }
    pub fn register(&self, root: &Path) -> Result<Arc<Workspace>, DomainError> {
        let root = root.canonicalize().map_err(io_error)?;
        let directory = File::open(&root).map_err(io_error)?;
        let meta = directory.metadata().map_err(io_error)?;
        if !meta.is_dir() {
            return Err(DomainError::InvalidInput(
                "workspace directory required".into(),
            ));
        }
        let id = WorkspaceId::new(format!(
            "ws-{:x}",
            Sha256::digest(format!("{}:{}:{}", root.display(), meta.dev(), meta.ino()).as_bytes())
        ))?;
        let mut roots = self.roots.lock().unwrap();
        if let Some(existing) = roots.get(&id) {
            return Ok(existing.clone());
        }
        if roots.len() >= 64 {
            return Err(DomainError::LimitExceeded("workspace registry".into()));
        }
        let workspace = Arc::new(Workspace {
            id: id.clone(),
            root,
            directory,
            device: meta.dev(),
            inode: meta.ino(),
            claim: Mutex::new(()),
        });
        roots.insert(id, workspace.clone());
        Ok(workspace)
    }
    pub fn get(&self, id: &WorkspaceId) -> Result<Arc<Workspace>, DomainError> {
        self.roots
            .lock()
            .unwrap()
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
        if bytes.len() > MAX_READ {
            return Box::pin(async { Err(DomainError::LimitExceeded("mutation bytes".into())) });
        }
        let root = self.get(workspace);
        let path = path.to_owned();
        let expected = expected.clone();
        let bytes = bytes.to_vec();
        self.pool
            .submit(move || root?.replace(&path, &expected, &bytes))
    }
}

/// Nonblocking inotify hints. Any event invalidates the index; overflow explicitly requests a full rebuild.
pub struct WatchHints {
    fd: OwnedFd,
    pub generation: u64,
    pub rebuild_required: bool,
}
impl WatchHints {
    pub fn new(workspace: &Workspace, paths: &[String]) -> Result<Self, DomainError> {
        use std::os::fd::FromRawFd;
        let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if raw < 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let mut parents = BTreeSet::from([PathBuf::new()]);
        for path in paths {
            relative(path)?;
            if let Some(parent) = Path::new(path).parent() {
                parents.insert(parent.to_path_buf());
            }
        }
        if parents.len() > 128 {
            return Err(DomainError::LimitExceeded("watch directories".into()));
        }
        for parent in parents {
            let directory = if parent.as_os_str().is_empty() {
                workspace.directory.try_clone().map_err(io_error)?
            } else {
                beneath(
                    &workspace.directory,
                    &parent,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                )?
            };
            let path =
                std::ffi::CString::new(format!("/proc/self/fd/{}", directory.as_raw_fd())).unwrap();
            let mask = libc::IN_CREATE
                | libc::IN_DELETE
                | libc::IN_MODIFY
                | libc::IN_MOVED_FROM
                | libc::IN_MOVED_TO
                | libc::IN_DELETE_SELF
                | libc::IN_MOVE_SELF;
            if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), mask) } < 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
        }
        Ok(Self {
            fd,
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
    pub fn poll(&mut self) -> Result<bool, DomainError> {
        let mut bytes = [0u8; 64 * 1024];
        let count =
            unsafe { libc::read(self.fd.as_raw_fd(), bytes.as_mut_ptr().cast(), bytes.len()) };
        if count < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(false);
            }
            return Err(io_error(err));
        }
        if count > 0 {
            self.invalidate_overflow();
            return Ok(true);
        }
        Ok(false)
    }
}

impl Workspace {
    /// Snapshot a link itself, never its target. Used by the preserved Godot
    /// staging manifest; ancestors retain the normal no-symlink workspace fence.
    pub fn link_snapshot(&self, name: &str) -> Result<(String, FileRevision), DomainError> {
        self.verify_root()?;
        let path = relative(name)?;
        let file = beneath(&self.directory, path, OFlags::PATH, Mode::empty())?;
        let before = file.metadata().map_err(io_error)?;
        if !before.file_type().is_symlink() {
            return Err(DomainError::InvalidInput("symbolic link required".into()));
        }
        let target = rfs::readlinkat(&file, "", Vec::new())
            .map_err(errno)?
            .into_string()
            .map_err(|_| DomainError::InvalidInput("UTF-8 link target".into()))?;
        let hash = format!("{:x}", Sha256::digest(target.as_bytes()));
        let current = beneath(&self.directory, path, OFlags::PATH, Mode::empty())?
            .metadata()
            .map_err(io_error)?;
        if revision(&before, &hash) != revision(&current, &hash) {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok((target, revision(&before, &hash)))
    }
    pub fn create_local_link(&self, name: &str, target: &str) -> Result<(), DomainError> {
        let _claim = self.claim()?;
        let path = relative(name)?;
        // Pinned backend library links are adjacent. Escaping links are not part
        // of a contained runtime package even if a permissive old stager copied them.
        if relative(target)?.components().count() != 1 {
            return Err(DomainError::PermissionDenied);
        }
        let parent = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => beneath(
                &self.directory,
                p,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?,
            None => self.directory.try_clone().map_err(io_error)?,
        };
        rfs::symlinkat(target, &parent, path.file_name().unwrap()).map_err(errno)?;
        parent
            .sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)
    }
    pub fn set_mode_checked(
        &self,
        name: &str,
        expected: &FileRevision,
        mode: u32,
    ) -> Result<(), DomainError> {
        if mode & !0o777 != 0 {
            return Err(DomainError::PermissionDenied);
        }
        let _claim = self.claim()?;
        let file = beneath(
            &self.directory,
            relative(name)?,
            OFlags::RDONLY,
            Mode::empty(),
        )?;
        if revision(&file.metadata().map_err(io_error)?, &expected.sha256) != *expected {
            return Err(DomainError::Conflict);
        }
        rfs::fchmod(&file, Mode::from_raw_mode(mode)).map_err(errno)?;
        file.sync_all().map_err(|_| DomainError::DurabilityUnknown)
    }
}

impl Workspace {
    /// Copy bounded runtime material from an explicit read-only source into an
    /// absent file. Used only in a newly owned staging tree, never a user restore.
    pub(crate) fn copy_runtime_material(
        &self,
        source: &Workspace,
        name: &str,
        control: &crate::resource_fs::ScanControl<'_>,
    ) -> Result<(), DomainError> {
        let path = relative(name)?;
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            self.create_directories_checked(
                parent.to_str().ok_or(DomainError::PermissionDenied)?,
                || control.check(),
            )?;
        }
        let _claim = self.claim()?;
        control.check()?;
        let mut output = beneath(
            &self.directory,
            path,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )?;
        let revision = source.stream_runtime_material(name, 2 * 1024 * 1024 * 1024, |bytes| {
            control.check()?;
            output.write_all(bytes).map_err(io_error)
        })?;
        if revision.mode & !0o777 != 0 {
            return Err(DomainError::PermissionDenied);
        }
        rfs::fchmod(&output, Mode::from_raw_mode(revision.mode)).map_err(errno)?;
        output
            .sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        self.directory
            .sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)
    }
}
