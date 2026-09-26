//! Large source trees are a streaming byte archive, separate from agent-state
//! recovery. Links are literal records, never traversed or recreated.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use sha2::{Digest, Sha256};
use std::{
    io::BufWriter,
    os::unix::{ffi::OsStrExt, io::AsRawFd},
};

const MAX_TOTAL: u64 = 128 * 1024 * 1024 * 1024;
const MAX_FILE: u64 = 16 * 1024 * 1024 * 1024;
const MAX_INDEX: u64 = 1024 * 1024 * 1024;
const MAX_ENTRIES: u64 = 2_000_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackedManifest {
    version: u32,
    kind: String,
    workspace: PathBuf,
    device: u64,
    inode: u64,
    git: Option<GitIdentity>,
    tree: Summary,
    link_semantics: String,
}
#[derive(Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    entries: u64,
    links: u64,
    bytes: u64,
    index_bytes: u64,
    index_sha256: String,
    data_sha256: String,
}
#[derive(Serialize)]
struct Entry {
    path_base64: String,
    kind: &'static str,
    mode: u32,
    offset: u64,
    bytes: u64,
    sha256: String,
    link_base64: String,
}
struct Scan<'a> {
    data: Option<&'a mut dyn Write>,
    index: Option<&'a mut dyn Write>,
    data_hash: Sha256,
    index_hash: Sha256,
    summary: Summary,
}
fn stable(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    (
        a.dev(),
        a.ino(),
        a.mode(),
        a.len(),
        a.mtime(),
        a.mtime_nsec(),
        a.ctime(),
        a.ctime_nsec(),
    ) == (
        b.dev(),
        b.ino(),
        b.mode(),
        b.len(),
        b.mtime(),
        b.mtime_nsec(),
        b.ctime(),
        b.ctime_nsec(),
    )
}
fn overlay_whiteout(mode: u32, device: u64) -> bool {
    mode & libc::S_IFMT == libc::S_IFCHR && device == 0
}
fn open(path: &Path, directory: bool) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | if directory { libc::O_DIRECTORY } else { 0 })
        .open(path)
        .map_err(io)
}
impl Scan<'_> {
    fn entry(&mut self, entry: Entry) -> Result<()> {
        self.summary.entries += 1;
        if self.summary.entries > MAX_ENTRIES {
            return Err(DomainError::LimitExceeded("source entries".into()));
        }
        let mut bytes = serde_json::to_vec(&entry).map_err(json)?;
        bytes.push(b'\n');
        self.summary.index_bytes += bytes.len() as u64;
        if self.summary.index_bytes > MAX_INDEX {
            return Err(DomainError::LimitExceeded("source index".into()));
        }
        self.index_hash.update(&bytes);
        if let Some(out) = self.index.as_mut() {
            out.write_all(&bytes).map_err(io)?;
        }
        Ok(())
    }
    fn walk(&mut self, directory: File, relative: &Path, depth: usize) -> Result<()> {
        if depth > 128 {
            return Err(DomainError::LimitExceeded("source depth".into()));
        }
        let before = directory.metadata().map_err(io)?;
        // Directory descriptors pin every ancestor; a renamed/replaced parent
        // cannot redirect a read through an external link.
        let base = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let mut entries = fs::read_dir(&base)
            .map_err(io)?
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(io)?;
        if entries.len() as u64 + self.summary.entries > MAX_ENTRIES {
            return Err(DomainError::LimitExceeded("source entries".into()));
        }
        entries.sort_by_key(|e| e.file_name());
        for child in entries {
            let path = base.join(child.file_name());
            let name = relative.join(child.file_name());
            let meta = fs::symlink_metadata(&path).map_err(io)?;
            let mut row = Entry {
                path_base64: B64.encode(name.as_os_str().as_bytes()),
                kind: "directory",
                mode: meta.mode() & 0o7777,
                offset: self.summary.bytes,
                bytes: 0,
                sha256: String::new(),
                link_base64: String::new(),
            };
            if meta.is_symlink() {
                row.kind = "literal-symlink";
                let target = fs::read_link(&path).map_err(io)?;
                if target.as_os_str().as_bytes().len() > 65536 {
                    return Err(DomainError::LimitExceeded("source link".into()));
                }
                row.link_base64 = B64.encode(target.as_os_str().as_bytes());
                self.summary.links += 1;
                self.entry(row)?;
            } else if meta.is_dir() {
                let child = open(&path, true)?;
                if !stable(&meta, &child.metadata().map_err(io)?) {
                    return Err(DomainError::Conflict);
                }
                self.entry(row)?;
                self.walk(child, &name, depth + 1)?;
            } else if meta.is_file() {
                if meta.len() > MAX_FILE || self.summary.bytes + meta.len() > MAX_TOTAL {
                    return Err(DomainError::LimitExceeded("source bytes".into()));
                }
                let mut file = open(&path, false)?;
                if !stable(&meta, &file.metadata().map_err(io)?) {
                    return Err(DomainError::Conflict);
                }
                let mut hash = Sha256::new();
                let mut count = 0;
                let mut buffer = vec![0u8; 128 * 1024];
                loop {
                    let n = file.read(&mut buffer).map_err(io)?;
                    if n == 0 {
                        break;
                    }
                    count += n as u64;
                    if count > meta.len() {
                        return Err(DomainError::Conflict);
                    }
                    hash.update(&buffer[..n]);
                    self.data_hash.update(&buffer[..n]);
                    if let Some(out) = self.data.as_mut() {
                        out.write_all(&buffer[..n]).map_err(io)?;
                    }
                }
                if count != meta.len() || !stable(&meta, &file.metadata().map_err(io)?) {
                    return Err(DomainError::Conflict);
                }
                row.kind = "file";
                row.bytes = count;
                row.sha256 = format!("{:x}", hash.finalize());
                self.summary.bytes += count;
                self.entry(row)?;
            } else if overlay_whiteout(meta.mode(), meta.rdev()) {
                // Overlay deletion markers have no readable payload. Record
                // their exact type/device semantics without opening a device.
                row.kind = "literal-overlay-whiteout-char-0-0";
                self.entry(row)?;
            } else {
                return Err(DomainError::Unavailable(
                    "Source sockets/devices require explicit settlement before capture".into(),
                ));
            }
            if !stable(&meta, &fs::symlink_metadata(&path).map_err(io)?) {
                return Err(DomainError::Conflict);
            }
        }
        if !stable(&before, &directory.metadata().map_err(io)?) {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    fn finish(mut self, workspace: &Path) -> Result<Summary> {
        let root = open(workspace, true)?;
        self.entry(Entry {
            path_base64: String::new(),
            kind: "directory",
            mode: root.metadata().map_err(io)?.mode() & 0o7777,
            offset: 0,
            bytes: 0,
            sha256: String::new(),
            link_base64: String::new(),
        })?;
        self.walk(root, Path::new(""), 0)?;
        self.summary.index_sha256 = format!("{:x}", self.index_hash.finalize());
        self.summary.data_sha256 = format!("{:x}", self.data_hash.finalize());
        Ok(self.summary)
    }
}

fn scan<'a>(
    workspace: &Path,
    data: Option<&'a mut dyn Write>,
    index: Option<&'a mut dyn Write>,
) -> Result<Summary> {
    Scan {
        data,
        index,
        data_hash: Sha256::new(),
        index_hash: Sha256::new(),
        summary: Summary::default(),
    }
    .finish(workspace)
}
fn hash_file(path: &Path, expected_bytes: u64, expected_hash: &str) -> Result<()> {
    let mut file = open(path, false)?;
    let before = file.metadata().map_err(io)?;
    if !before.is_file() || before.len() != expected_bytes {
        return Err(DomainError::Conflict);
    }
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = vec![0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(io)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        if bytes > expected_bytes {
            return Err(DomainError::Conflict);
        }
        hash.update(&buffer[..n]);
    }
    if bytes != expected_bytes
        || format!("{:x}", hash.finalize()) != expected_hash
        || !stable(&before, &file.metadata().map_err(io)?)
    {
        return Err(DomainError::Conflict);
    }
    Ok(())
}
pub(super) fn verify(receipt: &SourceSnapshot, workspace: &Path, unchanged: bool) -> Result<()> {
    let raw = bounded(&receipt.path.join("source-snapshot.json"), 1024 * 1024)?;
    if digest(&raw) != receipt.sha256 {
        return Err(DomainError::Conflict);
    }
    let m: PackedManifest = serde_json::from_slice(&raw).map_err(json)?;
    let (path, device, inode) = identity(workspace)?;
    if m.version != 2
        || m.kind != "ag1-source-streaming-snapshot"
        || m.link_semantics
            != "literal entries only; external targets are neither read nor restored"
        || (m.workspace.clone(), m.device, m.inode) != (path, device, inode)
        || m.tree.bytes > MAX_TOTAL
        || m.tree.index_bytes > MAX_INDEX
        || m.tree.entries > MAX_ENTRIES
    {
        return Err(DomainError::Conflict);
    }
    let mut names = fs::read_dir(&receipt.path)
        .map_err(io)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(io)?;
    names.sort();
    if names != ["source-snapshot.json", "tree.data", "tree.jsonl"].map(std::ffi::OsString::from) {
        return Err(DomainError::Conflict);
    }
    hash_file(
        &receipt.path.join("tree.jsonl"),
        m.tree.index_bytes,
        &m.tree.index_sha256,
    )?;
    hash_file(
        &receipt.path.join("tree.data"),
        m.tree.bytes,
        &m.tree.data_sha256,
    )?;
    if unchanged && (scan(workspace, None, None)? != m.tree || git_identity(workspace)? != m.git) {
        return Err(DomainError::Conflict);
    }
    Ok(())
}
pub(super) fn capture(lease: &Lease, destination: &Path) -> Result<SourceSnapshot> {
    let r = lease.record()?;
    if r.stage != Stage::Draining
        || !r.quiescent
        || r.owner != Owner::TypeScript
        || r.transfer.is_some()
    {
        return Err(conflict());
    }
    let destination = super::super::migration::new_path(destination)?;
    if destination.starts_with(&r.workspace)
        || r.workspace.starts_with(&destination)
        || r.state
            .as_ref()
            .is_some_and(|s| destination.starts_with(s) || s.starts_with(&destination))
    {
        return Err(DomainError::PermissionDenied);
    }
    if !r.workspace.join(".git").exists()
        && r.workspace
            .ancestors()
            .skip(1)
            .any(|p| p.join(".git").exists())
    {
        return Err(DomainError::Unavailable(
            "Parent Git closure requires explicit mapping".into(),
        ));
    }
    let git = git_identity(&r.workspace)?;
    fs::create_dir(&destination).map_err(io)?;
    fs::set_permissions(&destination, fs::Permissions::from_mode(0o700)).map_err(io)?;
    let create = |name: &str| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(destination.join(name))
            .map_err(io)
    };
    let mut data = BufWriter::new(create("tree.data")?);
    let mut index = BufWriter::new(create("tree.jsonl")?);
    let tree = scan(&r.workspace, Some(&mut data), Some(&mut index))?;
    data.flush().map_err(io)?;
    index.flush().map_err(io)?;
    data.get_ref().sync_all().map_err(io)?;
    index.get_ref().sync_all().map_err(io)?;
    let m = PackedManifest {
        version: 2,
        kind: "ag1-source-streaming-snapshot".into(),
        workspace: r.workspace.clone(),
        device: r.device,
        inode: r.inode,
        git,
        tree,
        link_semantics: "literal entries only; external targets are neither read nor restored"
            .into(),
    };
    let raw = serde_json::to_vec(&m).map_err(json)?;
    atomic(&destination.join("source-snapshot.json"), &raw)?;
    File::open(destination.parent().ok_or(DomainError::PermissionDenied)?)
        .and_then(|f| f.sync_all())
        .map_err(io)?;
    let receipt = SourceSnapshot {
        path: destination,
        sha256: digest(&raw),
    };
    verify(&receipt, &r.workspace, true)?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_zero_device_character_nodes_are_literal_whiteouts() {
        assert!(overlay_whiteout(libc::S_IFCHR, 0));
        assert!(overlay_whiteout(libc::S_IFCHR | 0o600, 0));
        for mode in [libc::S_IFBLK, libc::S_IFIFO, libc::S_IFSOCK, libc::S_IFREG] {
            assert!(!overlay_whiteout(mode, 0));
        }
        assert!(!overlay_whiteout(libc::S_IFCHR, 1));
    }
}
