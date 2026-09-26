//! Full native recovery closure. Process records are historical observations;
//! restoration creates neither a workspace lease nor a runtime-control grant.
use super::*;
use std::collections::BTreeSet;

const MAX_FILE: u64 = 256 * 1024 * 1024;
const MAX_TOTAL: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryFile {
    pub path: PathBuf,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifest {
    pub version: u32,
    pub kind: String,
    pub authority: Record,
    pub files: Vec<RecoveryFile>,
    pub directories: Vec<PathBuf>,
}

#[derive(Default, PartialEq, Eq)]
pub(super) struct Tree {
    pub(super) files: Vec<RecoveryFile>,
    pub(super) directories: Vec<PathBuf>,
}

pub(super) fn tree(root: &Path, skip: &[&str]) -> Result<Tree> {
    fn walk(
        root: &Path,
        path: &Path,
        skip: &[&str],
        out: &mut Tree,
        total: &mut u64,
    ) -> Result<()> {
        let mut entries = fs::read_dir(path)
            .map_err(io)?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(io)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|_| DomainError::PermissionDenied)?;
            if skip.iter().any(|name| relative == Path::new(name)) {
                continue;
            }
            let metadata = fs::symlink_metadata(&path).map_err(io)?;
            if metadata.is_symlink() {
                return Err(DomainError::PermissionDenied);
            }
            if out.files.len() + out.directories.len() >= 300_000 {
                return Err(DomainError::LimitExceeded("recovery tree".into()));
            }
            if metadata.is_dir() {
                out.directories.push(relative.to_path_buf());
                walk(root, &path, &[], out, total)?;
            } else if metadata.is_file() {
                let bytes = bounded(&path, MAX_FILE)?;
                *total += bytes.len() as u64;
                if *total > MAX_TOTAL {
                    return Err(DomainError::LimitExceeded("recovery archive".into()));
                }
                out.files.push(RecoveryFile {
                    path: relative.to_path_buf(),
                    sha256: digest(&bytes),
                    bytes: bytes.len() as u64,
                });
            } else {
                // Sockets, devices and FIFOs are not portable durable state.
                return Err(DomainError::PermissionDenied);
            }
        }
        Ok(())
    }
    if !fs::symlink_metadata(root).map_err(io)?.is_dir() {
        return Err(DomainError::PermissionDenied);
    }
    let mut out = Tree::default();
    walk(root, root, skip, &mut out, &mut 0)?;
    Ok(out)
}

pub(super) fn copy_tree(source: &Path, destination: &Path, tree: &Tree) -> Result<()> {
    private_dir(destination)?;
    for directory in &tree.directories {
        private_dir(&destination.join(directory))?;
    }
    for file in &tree.files {
        let bytes = bounded(&source.join(&file.path), file.bytes)?;
        if bytes.len() as u64 != file.bytes || digest(&bytes) != file.sha256 {
            return Err(DomainError::Conflict);
        }
        let target = destination.join(&file.path);
        private_dir(target.parent().ok_or(DomainError::PermissionDenied)?)?;
        atomic(&target, &bytes)?;
    }
    // Empty directories and their parent entries are durable too.
    for directory in tree.directories.iter().rev() {
        File::open(destination.join(directory))
            .and_then(|f| f.sync_all())
            .map_err(io)?;
    }
    File::open(destination)
        .and_then(|f| f.sync_all())
        .map_err(io)
}

pub(super) fn archive_native(
    record: &Record,
    state: &Path,
    store: &Store,
    archive: &Path,
) -> Result<serde_json::Value> {
    let archive = super::migration::new_path(archive)?;
    let state = fs::canonicalize(state).map_err(io)?;
    if archive.starts_with(&state) || archive.starts_with(&record.workspace) {
        return Err(DomainError::PermissionDenied);
    }
    let credentials = crate::client::imported::export_credentials(&state)?;
    let auxiliary = tree(&state, &["store"])?;
    let mut exported = store.export(&archive, false)?;
    copy_tree(&state, &archive.join("native-state"), &auxiliary)?;
    if tree(&state, &["store"])? != auxiliary
        || tree(&archive.join("native-state"), &[])? != auxiliary
    {
        return Err(DomainError::Conflict);
    }
    if let Some(credentials) = credentials {
        atomic(
            &archive.join("bridge/auth.json"),
            &serde_json::to_vec(&credentials).map_err(json)?,
        )?;
    }
    let all = tree(&archive, &[])?;
    let manifest = RecoveryManifest {
        version: 1,
        kind: "ag1-native-cutover-recovery".into(),
        authority: record.clone(),
        files: all.files,
        directories: all.directories,
    };
    let bytes = serde_json::to_vec(&manifest).map_err(json)?;
    atomic(&archive.join("recovery.json"), &bytes)?;
    exported["cutover_recovery_sha256"] = digest(&bytes).into();
    exported["native_auxiliary_files"] = auxiliary.files.len().into();
    Ok(exported)
}

/// Restore all native state into a new destination. The manifest hash must come
/// from the archived cutover receipt, not be recomputed from untrusted input.
/// Callers must subsequently reconcile observations and acquire authority.
pub fn restore_native_archive(
    archive: &Path,
    destination: &Path,
    expected_sha256: &str,
) -> Result<RecoveryManifest> {
    let (manifest, actual) = verify_archive(archive, expected_sha256)?;
    restore_verified(archive, destination, manifest, actual)
}

pub(super) fn verify_archive(
    archive: &Path,
    expected_sha256: &str,
) -> Result<(RecoveryManifest, Tree)> {
    let bytes = bounded(&archive.join("recovery.json"), 32 * 1024 * 1024)?;
    if digest(&bytes) != expected_sha256 {
        return Err(DomainError::Conflict);
    }
    let manifest: RecoveryManifest = serde_json::from_slice(&bytes).map_err(json)?;
    if manifest.version != 1
        || manifest.kind != "ag1-native-cutover-recovery"
        || manifest.authority.owner != Owner::Native
        || manifest.authority.stage != Stage::Draining
        || !manifest.authority.quiescent
    {
        return Err(DomainError::PermissionDenied);
    }
    let actual = tree(archive, &["recovery.json"])?;
    // Comparing the complete tree also rejects duplicate, escaping and injected
    // paths before any archive-controlled path can be used for a write.
    if actual.files != manifest.files || actual.directories != manifest.directories {
        return Err(DomainError::Conflict);
    }
    Ok((manifest, actual))
}

fn restore_verified(
    archive: &Path,
    destination: &Path,
    manifest: RecoveryManifest,
    actual: Tree,
) -> Result<RecoveryManifest> {
    let destination = super::migration::new_path(destination)?;
    let canonical_archive = fs::canonicalize(archive).map_err(io)?;
    if destination.starts_with(&canonical_archive)
        || destination.starts_with(&manifest.authority.workspace)
        || manifest
            .authority
            .state
            .as_ref()
            .is_some_and(|state| destination.starts_with(state))
    {
        return Err(DomainError::PermissionDenied);
    }
    let parent = destination.parent().ok_or(DomainError::PermissionDenied)?;
    let staging = parent.join(format!(".ag1-recover-{}", crate::client::nonce()?));
    fs::create_dir(&staging).map_err(io)?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).map_err(io)?;
    // Keep failed staging trees for diagnosis; never erase partial recovery.
    Store::restore_export(archive, &staging.join("store"))?;
    let auxiliary = tree(&archive.join("native-state"), &[])?;
    let top: BTreeSet<_> = auxiliary
        .files
        .iter()
        .map(|f| f.path.components().next())
        .chain(auxiliary.directories.iter().map(|p| p.components().next()))
        .collect();
    if top.contains(&Some(std::path::Component::Normal(std::ffi::OsStr::new(
        "store",
    )))) {
        return Err(DomainError::PermissionDenied);
    }
    copy_tree(&archive.join("native-state"), &staging, &auxiliary)?;
    if tree(&staging, &["store"])? != auxiliary || tree(archive, &["recovery.json"])? != actual {
        return Err(DomainError::Conflict);
    }
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        &staging,
        rustix::fs::CWD,
        &destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from)
    .map_err(io)?;
    File::open(parent).and_then(|f| f.sync_all()).map_err(io)?;
    Ok(manifest)
}
