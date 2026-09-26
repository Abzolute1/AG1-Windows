//! User-owned per-conversation directory grants. Model arguments cannot add grants.
use crate::{
    io_error,
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::DomainError;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
type Result<T> = std::result::Result<T, DomainError>;

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
    pub fn primary(&self) -> Arc<Workspace> {
        self.roots
            .iter()
            .find(|r| r.root() == self.selection.primary)
            .cloned()
            .unwrap_or_else(|| self.fallback.clone())
    }
    pub fn primary_available(&self) -> bool {
        self.selection
            .folders
            .iter()
            .any(|g| g.path == self.selection.primary && g.available)
    }
    pub fn primary_writable(&self) -> bool {
        self.selection.folders.iter().any(|g| {
            g.path == self.selection.primary && g.available && g.mode == AccessMode::ReadWrite
        })
    }
    pub fn resolve(&self, input: &str, write: bool) -> Result<Arc<Workspace>> {
        if input.len() > 4096 || input.contains('\0') {
            return Err(DomainError::InvalidInput(
                "Folder path is too long or invalid".into(),
            ));
        }
        if !Path::new(input).is_absolute() && !self.primary_available() {
            return Err(DomainError::InvalidInput("The working folder is unavailable. Choose an available working folder in Chat folders.".into()));
        }
        let normal = crate::file_tools::FileService::new(self.primary()).absolute_path(input)?;
        for grant in &self.selection.folders {
            if normal.starts_with(&grant.path) {
                let root=self.roots.iter().find(|root|root.root()==grant.path).ok_or_else(||DomainError::InvalidInput(format!("Granted folder is unavailable or changed identity: {}. Remove it and add it again in Chat folders.",grant.path.display())))?;
                if write && grant.mode != AccessMode::ReadWrite {
                    return Err(DomainError::InvalidInput(format!(
                        "Folder is read-only: {}. Ask the user to change it to Read & write in Chat folders.",
                        grant.path.display()
                    )));
                }
                root.verify_root()?;
                return Ok(root.clone());
            }
        }
        Err(DomainError::InvalidInput(format!(
            "Folder access is not granted for {}. Ask the user to add its folder in Chat folders; do not work around this with another tool.",
            normal.display()
        )))
    }
    pub fn prompt(&self) -> String {
        format!(
            "Chat folder access (user-controlled, revision {}):\nWorking folder: {}\n{}\nRelative paths use the working folder. Use absolute paths for other granted folders. If access is missing, ask the user to update Chat folders. Do not use shell or another tool to bypass a denied file request. Grants do not bypass operation approvals. Shell retains host-wide read access; only listed read/write folders permit persistent shell writes.",
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
                .join("\n")
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
        assert_eq!(changed.resolve("a.txt", true).unwrap().root(), other);
        assert_eq!(changed.primary().id(), a.storage.id());
        assert!(update(&a, "remove", &other, None).is_err());
        update(&a, "primary", a.storage.root(), None).unwrap();
        let revoked = update(&a, "remove", &other, None).unwrap();
        assert!(revoked.resolve(other.to_str().unwrap(), false).is_err());
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
        let mut protected = vec![state.to_owned()];
        for key in ["AG1_DESKTOP_STATE", "AG1_NATIVE_CONFIG"] {
            if let Some(path) = std::env::var_os(key).map(PathBuf::from).filter(|p|p.is_absolute()) {
                protected.push(path.canonicalize().unwrap_or(path));
            }
        }
        if let Some(path) = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p|p.is_absolute()) {
            protected.push(path.join("ag1"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            protected.extend([
                PathBuf::from(&home).join(".ag1"),
                PathBuf::from(home).join(".local/state/ag1"),
            ]);
        }
        let selection = match fs::read(&path) {
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
        // A missing primary must not silently reactivate a revoked old project.
        let recovery = directory.join("unavailable");
        fs::create_dir_all(&recovery).map_err(io_error)?;
        let fallback = WorkspaceService::new()?
            .register(&recovery)?
            .for_conversation(storage.id())?;
        let value = Self::bind(&storage, selection, fallback.clone())?;
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
                .any(|g| g.path == selection.primary)
        {
            return Err(DomainError::InvalidInput(
                "Choose one working folder from at most 16 chat folders".into(),
            ));
        }
        let registry = WorkspaceService::new()?;
        let mut roots = Vec::new();
        for (i, g) in selection.folders.iter().enumerate() {
            if !g.path.is_absolute()
                || selection.folders[..i]
                    .iter()
                    .any(|other| g.path.starts_with(&other.path) || other.path.starts_with(&g.path))
            {
                return Err(DomainError::InvalidInput(
                    "Chat folders must be separate, non-overlapping directories".into(),
                ));
            }
            if let Ok(root) = registry.register(&g.path) {
                if root.root() == g.path && root.directory_identity() == (g.device, g.inode) {
                    roots.push(root.for_conversation(storage.id())?);
                }
            }
        }
        for grant in &mut selection.folders {
            grant.available = roots.iter().any(|root| root.root() == grant.path);
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
            grant.available = snapshot.roots.iter().any(|r| r.root() == grant.path);
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
    pub(crate) fn change(&self, change: Change) -> Result<Snapshot> {
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
                    .any(|p| root.root().starts_with(p) || p.starts_with(root.root()))
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
                    .find(|g| g.path == change.path)
                    .ok_or(DomainError::NotFound)?
                    .mode = change.mode.ok_or_else(|| {
                    DomainError::InvalidInput("Choose Read only or Read & write".into())
                })?;
            }
            "primary" => {
                if !next.folders.iter().any(|g| g.path == change.path) {
                    return Err(DomainError::NotFound);
                }
                next.primary = change.path;
            }
            "remove" => {
                if next.primary == change.path {
                    return Err(DomainError::InvalidInput(
                        "Choose another working folder before removing this one".into(),
                    ));
                }
                let len = next.folders.len();
                next.folders.retain(|g| g.path != change.path);
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
        let bytes = serde_json::to_vec(&next.selection).map_err(|_| DomainError::Conflict)?;
        if bytes.len() > 24000 {
            return Err(DomainError::LimitExceeded("Chat folder paths".into()));
        }
        let mut file =
            tempfile::NamedTempFile::new_in(self.path.parent().ok_or(DomainError::Conflict)?)
                .map_err(io_error)?;
        file.write_all(&bytes).map_err(io_error)?;
        file.as_file().sync_all().map_err(io_error)?;
        file.persist(&self.path).map_err(|e| io_error(e.error))?;
        *value = next.clone();
        fs::File::open(self.path.parent().ok_or(DomainError::Conflict)?)
            .and_then(|f| f.sync_all())
            .map_err(|_| DomainError::DurabilityUnknown)?;
        Ok(next)
    }
}
