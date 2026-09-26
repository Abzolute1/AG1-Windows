//! Source custody is separate from session recovery. Nothing here restores a
//! worktree, runs Git, follows links, or grants authority over external files.
use super::*;
mod streaming;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    kind: String,
    workspace: PathBuf,
    device: u64,
    inode: u64,
    files: Vec<recovery::RecoveryFile>,
    directories: Vec<PathBuf>,
    modes: BTreeMap<PathBuf, u32>,
    /// Raw HEAD, selected loose ref/packed-refs and index are also in files.
    git: Option<GitIdentity>,
}

#[derive(PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GitIdentity {
    head: String,
    object: Option<String>,
}

fn git_identity(root: &Path) -> Result<Option<GitIdentity>> {
    let git = root.join(".git");
    match fs::symlink_metadata(&git) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_dir() && !m.is_symlink() => (),
        _ => {
            return Err(DomainError::Unavailable(
                "Linked Git directories require explicit source closure mapping".into(),
            ));
        }
    }
    // These mechanisms refer outside this snapshot. Do not silently certify an
    // incomplete object database or read another worktree's private files.
    for name in [
        "commondir",
        "objects/info/alternates",
        "objects/info/http-alternates",
        "reftable",
    ] {
        if fs::symlink_metadata(git.join(name)).is_ok() {
            return Err(DomainError::Unavailable(
                "External Git object closure requires explicit mapping".into(),
            ));
        }
    }
    let head =
        String::from_utf8(bounded(&git.join("HEAD"), 4096)?).map_err(|_| DomainError::Conflict)?;
    let selected = head.trim();
    let object = if let Some(reference) = selected.strip_prefix("ref: ") {
        let path = Path::new(reference);
        if !reference.starts_with("refs/")
            || path
                .components()
                .any(|p| !matches!(p, std::path::Component::Normal(_)))
        {
            return Err(DomainError::PermissionDenied);
        }
        match fs::symlink_metadata(git.join(path)) {
            Ok(_) => Some(
                String::from_utf8(bounded(&git.join(path), 4096)?)
                    .map_err(|_| DomainError::Conflict)?
                    .trim()
                    .to_owned(),
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match fs::symlink_metadata(git.join("packed-refs")) {
                    Ok(_) => {
                        String::from_utf8(bounded(&git.join("packed-refs"), 16 * 1024 * 1024)?)
                            .map_err(|_| DomainError::Conflict)?
                            .lines()
                            .filter_map(|line| line.split_once(' '))
                            .find(|(_, name)| *name == reference)
                            .map(|(oid, _)| oid.to_owned())
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None, // Unborn branch.
                    Err(e) => return Err(io(e)),
                }
            }
            Err(e) => return Err(io(e)),
        }
    } else {
        Some(selected.to_owned())
    };
    if object
        .as_ref()
        .is_some_and(|s| ![40, 64].contains(&s.len()) || !s.bytes().all(|c| c.is_ascii_hexdigit()))
    {
        return Err(DomainError::Conflict);
    }
    Ok(Some(GitIdentity { head, object }))
}

fn modes(root: &Path, tree: &recovery::Tree) -> Result<BTreeMap<PathBuf, u32>> {
    tree.files
        .iter()
        .map(|f| &f.path)
        .chain(tree.directories.iter())
        .map(|path| {
            let metadata = fs::symlink_metadata(root.join(path)).map_err(io)?;
            if metadata.is_symlink() {
                return Err(DomainError::PermissionDenied);
            }
            Ok((path.clone(), metadata.mode() & 0o7777))
        })
        .collect()
}

impl SourceSnapshot {
    /// Check retained bytes independently of the current worktree. Session
    /// rollback deliberately does not call a source restore operation.
    pub fn verify_archive(&self, workspace: &Path) -> Result<()> {
        if self.streaming()? {
            return streaming::verify(self, workspace, false);
        }
        self.manifest(workspace).map(|_| ())
    }
    fn streaming(&self) -> Result<bool> {
        let raw = bounded(&self.path.join("source-snapshot.json"), 64 * 1024 * 1024)?;
        if digest(&raw) != self.sha256 {
            return Err(DomainError::Conflict);
        }
        let value: serde_json::Value = serde_json::from_slice(&raw).map_err(json)?;
        Ok(value["version"] == 2)
    }
    fn manifest(&self, workspace: &Path) -> Result<Manifest> {
        let raw = bounded(&self.path.join("source-snapshot.json"), 64 * 1024 * 1024)?;
        if digest(&raw) != self.sha256 {
            return Err(DomainError::Conflict);
        }
        let m: Manifest = serde_json::from_slice(&raw).map_err(json)?;
        let (path, device, inode) = identity(workspace)?;
        if m.version != 1
            || m.kind != "ag1-source-snapshot"
            || (m.workspace.clone(), m.device, m.inode) != (path, device, inode)
        {
            return Err(DomainError::Conflict);
        }
        let tree = recovery::tree(&self.path.join("files"), &[])?;
        if tree.files != m.files
            || tree.directories != m.directories
            || git_identity(&self.path.join("files"))? != m.git
        {
            return Err(DomainError::Conflict);
        }
        Ok(m)
    }
    pub fn verify_unchanged(&self, workspace: &Path) -> Result<()> {
        if self.streaming()? {
            return streaming::verify(self, workspace, true);
        }
        let m = self.manifest(workspace)?;
        let tree = recovery::tree(workspace, &[])?;
        if tree.files != m.files
            || tree.directories != m.directories
            || modes(workspace, &tree)? != m.modes
            || git_identity(workspace)? != m.git
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

impl Lease {
    /// Stream all worktree entries, retaining literal links without traversal.
    pub fn snapshot_source_streaming(&self, destination: &Path) -> Result<SourceSnapshot> {
        streaming::capture(self, destination)
    }

    /// Snapshot only an already drained workspace, into a separate new archive.
    /// Full regular-file/Git closure is bounded like recovery. Unsupported
    /// links, special files and oversized trees block, with partial copies kept.
    pub fn snapshot_source(&self, destination: &Path) -> Result<SourceSnapshot> {
        let r = self.record()?;
        if r.stage != Stage::Draining
            || !r.quiescent
            || r.owner != Owner::TypeScript
            || r.transfer.is_some()
        {
            return Err(conflict());
        }
        let destination = super::migration::new_path(destination)?;
        if destination.starts_with(&r.workspace)
            || r.workspace.starts_with(&destination)
            || r.state
                .as_ref()
                .is_some_and(|s| destination.starts_with(s) || s.starts_with(&destination))
        {
            return Err(DomainError::PermissionDenied);
        }
        if fs::symlink_metadata(r.workspace.join(".git")).is_err()
            && r.workspace
                .ancestors()
                .skip(1)
                .any(|p| fs::symlink_metadata(p.join(".git")).is_ok())
        {
            return Err(DomainError::Unavailable(
                "Workspace inside a parent Git repository requires explicit source closure mapping"
                    .into(),
            ));
        }
        let tree = recovery::tree(&r.workspace, &[])?;
        if tree
            .files
            .iter()
            .any(|f| f.path.file_name().is_some_and(|n| n == ".git"))
        {
            return Err(DomainError::Unavailable(
                "Linked Git worktrees/submodules require explicit source closure mapping".into(),
            ));
        }
        let m = Manifest {
            version: 1,
            kind: "ag1-source-snapshot".into(),
            workspace: r.workspace.clone(),
            device: r.device,
            inode: r.inode,
            modes: modes(&r.workspace, &tree)?,
            git: git_identity(&r.workspace)?,
            files: tree.files.clone(),
            directories: tree.directories.clone(),
        };
        fs::create_dir(&destination).map_err(io)?;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).map_err(io)?;
        recovery::copy_tree(&r.workspace, &destination.join("files"), &tree)?;
        let raw = serde_json::to_vec(&m).map_err(json)?;
        atomic(&destination.join("source-snapshot.json"), &raw)?;
        File::open(destination.parent().ok_or(DomainError::PermissionDenied)?)
            .and_then(|f| f.sync_all())
            .map_err(io)?;
        let receipt = SourceSnapshot {
            path: destination,
            sha256: digest(&raw),
        };
        receipt.verify_unchanged(&r.workspace)?;
        Ok(receipt)
    }
}
