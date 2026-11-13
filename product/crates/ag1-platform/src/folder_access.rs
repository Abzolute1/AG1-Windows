//! User-owned per-conversation directory grants. Model arguments cannot add grants.
use crate::{
    io_error,
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use std::ffi::OsStr;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
#[cfg(windows)]
use {
    std::os::windows::ffi::OsStrExt,
    windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FlushFileBuffers, MOVEFILE_REPLACE_EXISTING,
            MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING,
        },
    },
};
type Result<T> = std::result::Result<T, DomainError>;

/// The WSL launcher enables this only for the local desktop harness.  Keeping
/// the switch host-owned means a model or renderer cannot expand its own file
/// scope, while Windows volumes remain discoverable through the same read-only
/// folder capabilities as every other local search.
#[cfg(target_os = "linux")]
fn readonly_windows_volume_roots() -> Vec<PathBuf> {
    if std::env::var_os("AG1_READONLY_WINDOWS_VOLUMES").as_deref() != Some(OsStr::new("1")) {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir("/mnt") else {
        return Vec::new();
    };
    let mut roots = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.len() == 1
                && name.bytes().all(|byte| byte.is_ascii_alphabetic())
                && entry.file_type().ok()?.is_dir())
            .then(|| entry.path())
        })
        .collect::<Vec<_>>();
    roots.sort();
    roots
}

#[cfg(windows)]
fn readonly_windows_volume_roots() -> Vec<PathBuf> {
    // A Windows-native host receives ordinary Windows folder grants directly.
    // `/mnt/*` is a WSL-only discovery mechanism and must not leak into them.
    Vec::new()
}

fn absolute_from(root: &Path, input: &str) -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok());
    let value = ag1_core::tools::path::expand(input, home.as_deref())?;
    let raw = Path::new(&value);
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };
    let mut normal = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                normal.pop();
            }
            std::path::Component::CurDir => {}
            _ => normal.push(component.as_os_str()),
        }
    }
    Ok(normal)
}

#[cfg(windows)]
fn public_path(path: PathBuf) -> PathBuf {
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

#[cfg(not(windows))]
fn public_path(path: PathBuf) -> PathBuf {
    path
}

// Windows paths name the same object regardless of ordinary case. `Path` does
// not know that, so its `starts_with` and `==` would reject a valid grant when
// canonicalize changes only `OpenAI` to `openai`, for example.
#[cfg(windows)]
fn same_path(left: &Path, right: &Path) -> bool {
    let mut left = left.components();
    let mut right = right.components();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return true,
            (Some(left), Some(right))
                if left
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy()) => {}
            _ => return false,
        }
    }
}

#[cfg(windows)]
fn path_within(path: &Path, root: &Path) -> bool {
    let mut path = path.components();
    for root_component in root.components() {
        let Some(path_component) = path.next() else {
            return false;
        };
        if !path_component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&root_component.as_os_str().to_string_lossy())
        {
            return false;
        }
    }
    true
}

#[cfg(not(windows))]
fn same_path(left: &Path, right: &Path) -> bool {
    left == right
}

#[cfg(not(windows))]
fn path_within(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

/// Add host-selected discovery roots without changing the chat's writable
/// working directory.  These are deliberately regular folder grants so the
/// native `read`, `ls`, `grep`, and `find` tools enforce the same path and
/// identity checks as they do for user-selected folders.
fn add_readonly_system_roots(
    selection: &mut Selection,
    roots: impl IntoIterator<Item = PathBuf>,
) -> Result<bool> {
    let registry = WorkspaceService::new()?;
    let mut changed = false;
    for path in roots {
        if selection.folders.len() >= 16
            || selection
                .folders
                .iter()
                .any(|grant| path_within(&path, &grant.path) || path_within(&grant.path, &path))
        {
            continue;
        }
        let root = registry.register(&path)?;
        let (device, inode) = root.directory_identity();
        selection.folders.push(Grant {
            path: root.root().into(),
            mode: AccessMode::ReadOnly,
            available: true,
            device,
            inode,
        });
        changed = true;
    }
    Ok(changed)
}

fn selection_bytes(selection: &Selection) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(selection).map_err(|_| DomainError::Conflict)?;
    if bytes.len() > 24000 {
        return Err(DomainError::LimitExceeded("Chat folder paths".into()));
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn persist_selection(path: &Path, selection: &Selection) -> Result<()> {
    let bytes = selection_bytes(selection)?;
    let parent = path.parent().ok_or(DomainError::Conflict)?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    file.write_all(&bytes).map_err(io_error)?;
    file.as_file().sync_all().map_err(io_error)?;
    file.persist(path).map_err(|e| io_error(e.error))?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| DomainError::DurabilityUnknown)
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn flush_directory(path: &Path) -> Result<()> {
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
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let flushed = unsafe { FlushFileBuffers(handle) } != 0;
    let error = (!flushed).then(std::io::Error::last_os_error);
    unsafe { CloseHandle(handle) };
    if flushed {
        Ok(())
    } else if matches!(error.and_then(|error| error.raw_os_error()), Some(1 | 5)) {
        // Some Windows filesystems reject directory flushes. The selection
        // file was already synced and MoveFileEx used WRITE_THROUGH, so this
        // is not a failed publication; keep other flush errors observable.
        Ok(())
    } else {
        Err(DomainError::DurabilityUnknown)
    }
}

#[cfg(windows)]
fn persist_selection(path: &Path, selection: &Selection) -> Result<()> {
    let bytes = selection_bytes(selection)?;
    let parent = path.parent().ok_or(DomainError::Conflict)?;
    let temporary = parent.join(format!(
        ".ag1w-folder-access-{}-{}",
        std::process::id(),
        selection.revision
    ));
    let write = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
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
        return Err(io_error(std::io::Error::last_os_error()));
    }
    flush_directory(parent)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub path: PathBuf,
    pub mode: AccessMode,
    #[serde(default)]
    pub available: bool,
    device: u64,
    inode: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub revision: u64,
    pub primary: PathBuf,
    pub folders: Vec<Grant>,
}
#[derive(Clone)]
pub struct Snapshot {
    pub selection: Selection,
    pub roots: Vec<Arc<Workspace>>,
    fallback: Arc<Workspace>,
}
impl Snapshot {
    /// A frozen list of currently verified roots.  Callers still must resolve
    /// each model-supplied path through `resolve_relative`; this accessor is
    /// only for bounded discovery across folders already granted by the user.
    pub fn roots(&self) -> &[Arc<Workspace>] {
        &self.roots
    }

    pub fn primary(&self) -> Arc<Workspace> {
        self.roots
            .iter()
            .find(|r| same_path(r.root(), &self.selection.primary))
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }
    pub fn primary_available(&self) -> bool {
        self.selection
            .folders
            .iter()
            .any(|g| same_path(&g.path, &self.selection.primary) && g.available)
    }
    pub fn primary_writable(&self) -> bool {
        self.selection.folders.iter().any(|g| {
            same_path(&g.path, &self.selection.primary)
                && g.available
                && g.mode == AccessMode::ReadWrite
        })
    }
    pub fn resolve(&self, input: &str, write: bool) -> Result<Arc<Workspace>> {
        self.resolve_relative(input, write)
            .map(|(workspace, _)| workspace)
    }

    /// Resolve a tool path to both its granted root and the path that root can
    /// safely accept.  Keeping this conversion here means Windows path case
    /// rules are applied once, before a project tool touches the filesystem.
    pub fn resolve_relative(&self, input: &str, write: bool) -> Result<(Arc<Workspace>, String)> {
        if input.len() > 4096 || input.contains('\0') {
            return Err(DomainError::InvalidInput(
                "Folder path is too long or invalid".into(),
            ));
        }
        if !Path::new(input).is_absolute() && !self.primary_available() {
            return Err(DomainError::InvalidInput("The working folder is unavailable. Choose an available working folder in Chat folders.".into()));
        }
        let normal = absolute_from(self.primary().root(), input)?;
        for grant in &self.selection.folders {
            if path_within(&normal, &grant.path) {
                let root=self.roots.iter().find(|root|root.root()==grant.path).ok_or_else(||DomainError::InvalidInput(format!("Granted folder is unavailable or changed identity: {}. Remove it and add it again in Chat folders.",grant.path.display())))?;
                if write && grant.mode != AccessMode::ReadWrite {
                    return Err(DomainError::InvalidInput(format!(
                        "Folder is read-only: {}. Ask the user to change it to Read & write in Chat folders.",
                        grant.path.display()
                    )));
                }
                root.verify_root()?;
                let relative = normal
                    .components()
                    .skip(root.root().components().count())
                    .collect::<PathBuf>();
                let relative = relative.to_string_lossy().replace('\\', "/");
                return Ok((root.clone(), relative));
            }
        }
        Err(DomainError::InvalidInput(format!(
            "Folder access is not granted for {}. Ask the user to add its folder in Chat folders; do not work around this with another tool.",
            normal.display()
        )))
    }
    pub fn prompt(&self) -> String {
        let windows = self
            .selection
            .folders
            .iter()
            .filter(|grant| {
                grant.mode == AccessMode::ReadOnly
                    && grant.path.parent() == Some(Path::new("/mnt"))
                    && grant
                        .path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name.len() == 1 && name.bytes().all(|byte| byte.is_ascii_alphabetic())
                        })
            })
            .map(|grant| grant.path.display().to_string())
            .collect::<Vec<_>>();
        let windows_guidance = if windows.is_empty() {
            String::new()
        } else {
            format!(
                "\nRead-only Windows discovery volumes: {}. In WSL, Windows C:\\ is /mnt/c. Use absolute paths under these roots with read, ls, grep, or find when asked to locate local files. They are not writable.",
                windows.join(", ")
            )
        };
        format!(
            "Chat folder access (revision {}; host-selected read-only discovery volumes may be included):\nWorking folder: {}\n{}{}\nRelative paths use the working folder. Use absolute paths for other granted folders. If access is missing, ask the user to update Chat folders. Do not use shell or another tool to bypass a denied file request. Grants do not bypass operation approvals. Only listed read/write folders permit persistent shell writes.",
            self.selection.revision,
            self.selection.primary.display(),
            self.selection
                .folders
                .iter()
                .map(|g| format!(
                    "- {}: {}",
                    g.path.display(),
                    if !g.available {
                        "unavailable; no access"
                    } else if g.mode == AccessMode::ReadWrite {
                        "read/write"
                    } else {
                        "read-only"
                    }
                ))
                .collect::<Vec<_>>()
                .join("\n"),
            windows_guidance,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Arc<FolderAccess>, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let other = dir.path().join("other");
        let state = dir.path().join("state");
        for p in [&root, &other, &state] {
            fs::create_dir(p).unwrap();
        }
        let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
        let access = FolderAccess::open(workspace, &state, "chat-a").unwrap();
        (dir, access, other)
    }
    fn update(
        a: &FolderAccess,
        action: &str,
        path: &Path,
        mode: Option<AccessMode>,
    ) -> Result<Snapshot> {
        a.change(Change {
            revision: a.snapshot()?.selection.revision,
            action: action.into(),
            path: path.into(),
            mode,
        })
    }
    #[test]
    fn grants_default_readonly_persist_and_are_conversation_local() {
        let (dir, a, other) = fixture();
        let old = a.snapshot().unwrap();
        let added = update(&a, "add", &other, None).unwrap();
        assert!(a.check(old.selection.revision).is_err());
        assert!(
            added
                .resolve(other.join("test.txt").to_str().unwrap(), false)
                .is_ok()
        );
        assert!(
            added
                .resolve(other.join("test.txt").to_str().unwrap(), true)
                .is_err()
        );
        assert!(added.resolve("../../outside", false).is_err());
        let reopened =
            FolderAccess::open(a.storage.clone(), &dir.path().join("state"), "chat-a").unwrap();
        assert_eq!(reopened.snapshot().unwrap().selection.folders.len(), 2);
        let sibling =
            FolderAccess::open(a.storage.clone(), &dir.path().join("state"), "chat-b").unwrap();
        assert!(
            sibling
                .snapshot()
                .unwrap()
                .resolve(other.to_str().unwrap(), false)
                .is_err()
        );
        update(&a, "mode", &other, Some(AccessMode::ReadWrite)).unwrap();
        let changed = update(&a, "primary", &other, None).unwrap();
        // Canonical Windows paths may preserve a different casing than the
        // caller supplied, even though both paths name the same directory.
        assert!(same_path(
            changed.resolve("a.txt", true).unwrap().root(),
            &other
        ));
        assert_eq!(changed.primary().id(), a.storage.id());
        assert!(update(&a, "remove", &other, None).is_err());
        update(&a, "primary", a.storage.root(), None).unwrap();
        let revoked = update(&a, "remove", &other, None).unwrap();
        assert!(revoked.resolve(other.to_str().unwrap(), false).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn host_discovery_roots_are_readonly_and_keep_the_project_primary() {
        let (dir, access, _) = fixture();
        let volume = dir.path().join("c");
        fs::create_dir(&volume).unwrap();
        let mut selection = access.snapshot().unwrap().selection;
        assert!(add_readonly_system_roots(&mut selection, vec![volume.clone()]).unwrap());
        assert_eq!(selection.primary, access.storage.root());
        let snapshot =
            FolderAccess::bind(&access.storage, selection, access.fallback.clone()).unwrap();
        assert!(snapshot.resolve(volume.to_str().unwrap(), false).is_ok());
        assert!(snapshot.resolve(volume.to_str().unwrap(), true).is_err());
    }
    #[test]
    fn rejects_stale_overlapping_private_and_replaced_directories() {
        let (dir, a, other) = fixture();
        assert!(
            a.change(Change {
                revision: 0,
                action: "add".into(),
                path: other.clone(),
                mode: None
            })
            .is_err()
        );
        assert!(update(&a, "add", dir.path(), None).is_err());
        assert!(update(&a, "add", &dir.path().join("state"), None).is_err());
        let nested = a.storage.root().join("nested");
        fs::create_dir(&nested).unwrap();
        assert!(update(&a, "add", &nested, None).is_err());
        let added = update(&a, "add", &other, None).unwrap();
        fs::rename(&other, dir.path().join("moved")).unwrap();
        fs::create_dir(&other).unwrap();
        assert!(added.resolve(other.to_str().unwrap(), false).is_err());
        let reopened =
            FolderAccess::open(a.storage.clone(), &dir.path().join("state"), "chat-a").unwrap();
        assert!(!reopened.snapshot().unwrap().selection.folders[1].available);
        assert!(
            reopened
                .snapshot()
                .unwrap()
                .resolve(other.to_str().unwrap(), false)
                .is_err()
        );
        update(&reopened, "remove", &other, None).unwrap();
        assert!(
            update(&reopened, "add", &other, None)
                .unwrap()
                .selection
                .folders[1]
                .available
        );
    }
}
pub struct FolderAccess {
    value: Mutex<Snapshot>,
    path: PathBuf,
    storage: Arc<Workspace>,
    fallback: Arc<Workspace>,
    protected: Vec<PathBuf>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub revision: u64,
    pub action: String,
    pub path: PathBuf,
    pub mode: Option<AccessMode>,
}
impl FolderAccess {
    pub fn open(storage: Arc<Workspace>, state: &Path, session: &str) -> Result<Arc<Self>> {
        let directory = state.join("folder-access");
        fs::create_dir_all(&directory).map_err(io_error)?;
        let path = directory.join(format!("{}.json", ag1_storage::digest(session.as_bytes())));
        let mut protected = vec![public_path(
            state.canonicalize().unwrap_or_else(|_| state.to_owned()),
        )];
        for key in ["AG1_DESKTOP_STATE", "AG1_NATIVE_CONFIG"] {
            if let Some(path) = std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
            {
                protected.push(public_path(path.canonicalize().unwrap_or(path)));
            }
        }
        if let Some(path) = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            protected.push(path.join("ag1"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            protected.extend([
                PathBuf::from(&home).join(".ag1"),
                PathBuf::from(home).join(".local/state/ag1"),
            ]);
        }
        let mut selection = match fs::read(&path) {
            Ok(bytes) if bytes.len() <= 32768 => serde_json::from_slice(&bytes).map_err(|_| {
                DomainError::InvalidInput("Saved chat folder permissions are invalid".into())
            })?,
            Ok(_) => return Err(DomainError::LimitExceeded("chat folders".into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (device, inode) = storage.directory_identity();
                Selection {
                    revision: 1,
                    primary: storage.root().into(),
                    folders: vec![Grant {
                        path: storage.root().into(),
                        mode: AccessMode::ReadWrite,
                        available: true,
                        device,
                        inode,
                    }],
                }
            }
            Err(e) => return Err(io_error(e)),
        };
        let added_system_roots =
            add_readonly_system_roots(&mut selection, readonly_windows_volume_roots())?;
        // A missing primary must not silently reactivate a revoked old project.
        let recovery = directory.join("unavailable");
        fs::create_dir_all(&recovery).map_err(io_error)?;
        let fallback = WorkspaceService::new()?
            .register(&recovery)?
            .for_conversation(storage.id())?;
        let value = Self::bind(&storage, selection, fallback.clone())?;
        if added_system_roots {
            persist_selection(&path, &value.selection)?;
        }
        Ok(Arc::new(Self {
            value: Mutex::new(value),
            path,
            storage,
            fallback,
            protected,
        }))
    }
    fn bind(
        storage: &Arc<Workspace>,
        mut selection: Selection,
        fallback: Arc<Workspace>,
    ) -> Result<Snapshot> {
        if selection.revision == 0
            || selection.folders.is_empty()
            || selection.folders.len() > 16
            || !selection
                .folders
                .iter()
                .any(|g| same_path(&g.path, &selection.primary))
        {
            return Err(DomainError::InvalidInput(
                "Choose one working folder from at most 16 chat folders".into(),
            ));
        }
        let registry = WorkspaceService::new()?;
        let mut roots = Vec::new();
        for (i, g) in selection.folders.iter().enumerate() {
            if !g.path.is_absolute()
                || selection.folders[..i].iter().any(|other| {
                    path_within(&g.path, &other.path) || path_within(&other.path, &g.path)
                })
            {
                return Err(DomainError::InvalidInput(
                    "Chat folders must be separate, non-overlapping directories".into(),
                ));
            }
            if let Ok(root) = registry.register(&g.path) {
                if same_path(root.root(), &g.path)
                    && root.directory_identity() == (g.device, g.inode)
                {
                    roots.push(root.for_conversation(storage.id())?);
                }
            }
        }
        for grant in &mut selection.folders {
            grant.available = roots.iter().any(|root| same_path(root.root(), &grant.path));
        }
        Ok(Snapshot {
            selection,
            roots,
            fallback,
        })
    }
    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut snapshot = self
            .value
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clone();
        snapshot.roots.retain(|r| r.verify_root().is_ok());
        for grant in &mut snapshot.selection.folders {
            grant.available = snapshot
                .roots
                .iter()
                .any(|root| same_path(root.root(), &grant.path));
        }
        Ok(snapshot)
    }
    pub fn check(&self, revision: u64) -> Result<()> {
        if self.snapshot()?.selection.revision != revision {
            Err(DomainError::Conflict)
        } else {
            Ok(())
        }
    }
    /// Called only by idle, exact-scope host UI admission. Publishing invalidates old runners.
    pub fn change(&self, change: Change) -> Result<Snapshot> {
        let mut value = self.value.lock().map_err(|_| DomainError::Conflict)?;
        if change.revision != value.selection.revision {
            return Err(DomainError::Conflict);
        }
        let mut next = value.selection.clone();
        match change.action.as_str() {
            "add" => {
                if !change.path.is_absolute() {
                    return Err(DomainError::InvalidInput(
                        "Choose an absolute folder path".into(),
                    ));
                }
                let root = WorkspaceService::new()?.register(&change.path)?;
                if self
                    .protected
                    .iter()
                    .any(|p| path_within(root.root(), p) || path_within(p, root.root()))
                {
                    return Err(DomainError::InvalidInput("That folder includes AG1's private storage. Choose a project subfolder instead.".into()));
                }
                let (device, inode) = root.directory_identity();
                next.folders.push(Grant {
                    path: root.root().into(),
                    mode: change.mode.unwrap_or(AccessMode::ReadOnly),
                    available: true,
                    device,
                    inode,
                });
            }
            "mode" => {
                next.folders
                    .iter_mut()
                    .find(|g| same_path(&g.path, &change.path))
                    .ok_or(DomainError::NotFound)?
                    .mode = change.mode.ok_or_else(|| {
                    DomainError::InvalidInput("Choose Read only or Read & write".into())
                })?;
            }
            "primary" => {
                let primary = next
                    .folders
                    .iter()
                    .find(|g| same_path(&g.path, &change.path))
                    .map(|grant| grant.path.clone())
                    .ok_or(DomainError::NotFound)?;
                next.primary = primary;
            }
            "remove" => {
                if same_path(&next.primary, &change.path) {
                    return Err(DomainError::InvalidInput(
                        "Choose another working folder before removing this one".into(),
                    ));
                }
                let len = next.folders.len();
                next.folders.retain(|g| !same_path(&g.path, &change.path));
                if len == next.folders.len() {
                    return Err(DomainError::NotFound);
                }
            }
            _ => {
                return Err(DomainError::InvalidInput(
                    "Unknown chat folder action".into(),
                ));
            }
        }
        next.revision = next.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        let next = Self::bind(&self.storage, next, self.fallback.clone())?;
        persist_selection(&self.path, &next.selection)?;
        *value = next.clone();
        Ok(next)
    }
}
