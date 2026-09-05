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
        unix::fs::MetadataExt,
    },
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

pub const MAX_READ: usize = 8 * 1024 * 1024;
pub const MAX_FILES: usize = 1000;
pub const MAX_INDEX_BYTES: usize = 16 * 1024 * 1024;
static TEMP_SERIAL: AtomicU64 = AtomicU64::new(1);

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
    pub fn id(&self) -> &WorkspaceId {
        &self.id
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
        let mut file = beneath(
            &parent,
            Path::new(&temporary),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )?;
        let result = (|| {
            file.write_all(bytes).map_err(io_error)?;
            // Preserve executable/read/write bits. Set-ID/sticky are never silently copied to new bytes.
            if expected.mode & 0o7000 != 0 {
                return Err(DomainError::PermissionDenied);
            }
            rfs::fchmod(&file, Mode::from_raw_mode(expected.mode)).map_err(errno)?;
            file.sync_all().map_err(io_error)?;
            if self.snapshot(name, MAX_READ)?.revision != *expected {
                return Err(DomainError::Conflict);
            }
            self.verify_root()?;
            rfs::renameat(&parent, &temporary, &parent, path.file_name().unwrap())
                .map_err(errno)?;
            parent
                .sync_all()
                .map_err(|_| DomainError::DurabilityUnknown)?;
            let snapshot = self.snapshot(name, MAX_READ)?;
            if snapshot.bytes != bytes {
                return Err(DomainError::Conflict);
            }
            Ok(snapshot.revision)
        })();
        let _ = rfs::unlinkat(&parent, &temporary, rfs::AtFlags::empty());
        result
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
