use ag1_core::Clock;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

pub type StorageResult<T> = Result<T, StorageError>;
#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    Corrupt(String),
    Invalid(String),
    Conflict(String),
    RevisionConflict { expected: u64, actual: u64 },
    Missing(String),
    Denied,
    Unsupported(String),
    Limit(String),
    Injected(Boundary),
    ImportIncomplete,
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for StorageError {}
impl From<StorageError> for ag1_core::DomainError {
    fn from(error: StorageError) -> Self {
        domain_error(error)
    }
}
impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Boundary {
    BeforeBlobSync,
    AfterBlobSync,
    BeforeRename,
    AfterRename,
    BeforeTransaction,
    BeforeCommit,
    AfterCommit,
    BeforeAck,
    /// Harness observer boundary, after a successful API return has been recorded externally.
    AfterAck,
    BeforeImportCheckpoint,
    AfterImportCheckpoint,
}
pub trait FaultInjector: Send + Sync {
    fn check(&self, boundary: Boundary) -> StorageResult<()>;
    /// Journal-command targeting avoids timing-dependent injections into an
    /// unrelated artifact transaction when accepted native work drains together.
    fn check_journal_command(&self, _command: &str, boundary: Boundary) -> StorageResult<()> {
        self.check(boundary)
    }
}
struct NoFault;
impl FaultInjector for NoFault {
    fn check(&self, _: Boundary) -> StorageResult<()> {
        Ok(())
    }
}
struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(i64::MAX as u128) as u64
    }
}

struct WriterLease(File);
impl Drop for WriterLease {
    fn drop(&mut self) {
        // Closing alone leaves flock held by an unrelated fork's inherited
        // open description until it execs. Release this owner's lease explicitly.
        #[cfg(target_os = "linux")]
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}
pub struct Store {
    pub(crate) root: PathBuf,
    pub(crate) db: Mutex<Connection>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) faults: Arc<dyn FaultInjector>,
    // Keep last: the database connection must close before another writer enters.
    _writer: WriterLease,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Health {
    pub schema: u32,
    pub sequence: u64,
    pub integrity: String,
    pub orphan_blobs: usize,
    pub staged_files: usize,
    pub uncertain_commands: usize,
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(crate) fn domain_error(e: StorageError) -> ag1_core::DomainError {
    match e {
        StorageError::Denied => ag1_core::DomainError::PermissionDenied,
        StorageError::Conflict(_) | StorageError::RevisionConflict { .. } => {
            ag1_core::DomainError::Conflict
        }
        StorageError::Missing(_) => ag1_core::DomainError::NotFound,
        StorageError::Limit(s) => ag1_core::DomainError::LimitExceeded(s),
        StorageError::Invalid(s) => ag1_core::DomainError::InvalidInput(s),
        other => ag1_core::DomainError::Io(other.to_string()),
    }
}
pub(crate) fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn valid_id(s: &str) -> StorageResult<()> {
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    {
        return Err(StorageError::Invalid("Identifier".into()));
    }
    Ok(())
}
pub(crate) fn safe_relative(path: &str) -> StorageResult<&Path> {
    let p = Path::new(path);
    if path.is_empty()
        || path.contains('\\')
        || !p.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(StorageError::Invalid("Unsafe archive path".into()));
    }
    Ok(p)
}
pub(crate) fn sync_dir(path: &Path) -> StorageResult<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}
pub(crate) fn publish_directory(source: &Path, destination: &Path) -> StorageResult<()> {
    #[cfg(target_os = "linux")]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (source, destination);
        Err(StorageError::Unsupported(
            "Atomic directory publication is not validated on this platform".into(),
        ))
    }
}
pub(crate) fn private_dir(path: &Path) -> StorageResult<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(crate) fn no_symlink(path: &Path) -> StorageResult<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    for p in absolute.ancestors() {
        if fs::symlink_metadata(p)?.file_type().is_symlink() {
            return Err(StorageError::Invalid(
                "Symlink in storage/source path".into(),
            ));
        }
    }
    Ok(())
}
pub(crate) fn read_bounded(path: &Path, max: u64) -> StorageResult<Vec<u8>> {
    no_symlink(path)?;
    let mut f = File::open(path)?;
    let before = f.metadata()?;
    if !before.is_file() || before.len() > max {
        return Err(StorageError::Limit(
            "Source is not a bounded regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut f).take(max + 1).read_to_end(&mut bytes)?;
    let after = f.metadata()?;
    let current = fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let identity = |m: &fs::Metadata| {
            (
                m.dev(),
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime(),
                m.ctime_nsec(),
            )
        };
        if identity(&before) != identity(&after) || identity(&after) != identity(&current) {
            return Err(StorageError::Conflict(
                "Source changed while reading".into(),
            ));
        }
    }
    if bytes.len() as u64 != before.len()
        || bytes.len() as u64 > max
        || after.len() != current.len()
    {
        return Err(StorageError::Corrupt("Incomplete source read".into()));
    }
    Ok(bytes)
}

impl Store {
    pub fn open(path: &Path) -> StorageResult<Self> {
        Self::open_with(path, Arc::new(SystemClock), Arc::new(NoFault))
    }
    pub fn open_with(
        path: &Path,
        clock: Arc<dyn Clock>,
        faults: Arc<dyn FaultInjector>,
    ) -> StorageResult<Self> {
        if !cfg!(target_os = "linux") {
            return Err(StorageError::Unsupported(
                "Storage writer/dir-sync platform not validated".into(),
            ));
        }
        // Caller supplies a private shadow destination; never an original project/state directory.
        if path.exists() {
            no_symlink(path)?;
            let candidate = path.join("storage.sqlite");
            if candidate.exists() {
                no_symlink(&candidate)?;
                for sidecar in ["storage.sqlite-wal", "storage.sqlite-shm"] {
                    let p = path.join(sidecar);
                    if p.exists() {
                        no_symlink(&p)?;
                    }
                }
                let probe = Connection::open_with_flags(
                    &candidate,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                let app: i64 = probe.query_row("PRAGMA application_id", [], |r| r.get(0))?;
                let version: u32 = probe.query_row("PRAGMA user_version", [], |r| r.get(0))?;
                if app != 1095184723 || version != 1 {
                    return Err(StorageError::Unsupported(
                        "Unrecognized database identity/version; left unchanged".into(),
                    ));
                }
            }
            if !path.join("storage.sqlite").exists() && fs::read_dir(path)?.next().is_some() {
                return Err(StorageError::Conflict(
                    "Destination must be new or an existing AG1 Rust store".into(),
                ));
            }
        } else {
            no_symlink(
                path.parent()
                    .ok_or_else(|| StorageError::Invalid("Destination parent".into()))?,
            )?;
        }
        private_dir(path)?;
        let root = path.canonicalize()?;
        let lock = root.join("writer.lock");
        if lock.exists() {
            no_symlink(&lock)?;
        }
        let writer = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock)?;
        #[cfg(target_os = "linux")]
        rustix::fs::flock(
            &writer,
            rustix::fs::FlockOperation::NonBlockingLockExclusive,
        )
        .map_err(|_| StorageError::Conflict("Destination already has a writer".into()))?;
        let writer = WriterLease(writer);
        for name in ["blobs", "staging", "quarantine"] {
            let p = root.join(name);
            if p.exists() {
                no_symlink(&p)?;
            }
            private_dir(&p)?;
        }
        let db_path = root.join("storage.sqlite");
        if db_path.exists() {
            no_symlink(&db_path)?;
        }
        let mut db = Connection::open(&db_path)?;
        let version: u32 = db.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            return Err(StorageError::Unsupported(format!(
                "Storage schema {version}"
            )));
        }
        if version == 0 {
            let tables: i64 = db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return Err(StorageError::Corrupt(
                    "Unversioned nonempty database".into(),
                ));
            }
        }
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA trusted_schema=OFF; PRAGMA busy_timeout=1000; PRAGMA temp_store=FILE; PRAGMA temp.cache_size=-2048;")?;
        if version == 0 {
            let tx = db.transaction()?;
            tx.execute_batch(include_str!("schema.sql"))?;
            tx.execute_batch("PRAGMA application_id=1095184723; PRAGMA user_version=1;")?;
            tx.commit()?;
        }
        let app: i64 = db.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if app != 1095184723 {
            return Err(StorageError::Corrupt("Not an AG1 storage database".into()));
        }
        let integrity: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if integrity != "ok" {
            return Err(StorageError::Corrupt(integrity));
        }
        // An intent is not proof that an external effect completed.
        db.execute(
            "UPDATE commands SET state='uncertain' WHERE state='intent'",
            [],
        )?;
        sync_dir(&root)?;
        sync_dir(root.parent().unwrap())?;
        let store = Self {
            root,
            db: Mutex::new(db),
            clock,
            faults,
            _writer: writer,
        };
        store.reconcile()?;
        Ok(store)
    }
    pub(crate) fn conn(&self) -> StorageResult<MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| StorageError::Corrupt("Poisoned storage owner".into()))
    }
    pub(crate) fn ready(db: &Connection) -> StorageResult<()> {
        let n: i64 = db.query_row(
            "SELECT count(*) FROM imports WHERE state='staging'",
            [],
            |r| r.get(0),
        )?;
        if n != 0 {
            return Err(StorageError::ImportIncomplete);
        }
        Ok(())
    }
    pub(crate) fn hit(&self, b: Boundary) -> StorageResult<()> {
        self.faults.check(b)
    }
    pub(crate) fn hit_journal_command(&self, command: &str, b: Boundary) -> StorageResult<()> {
        self.faults.check_journal_command(command, b)
    }
    pub(crate) fn next_sequence(db: &Connection) -> StorageResult<u64> {
        let value: u64 = db.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='sequence'",
            [],
            |r| r.get(0),
        )?;
        let next = value
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or_else(|| StorageError::Limit("Committed sequence exhausted".into()))?;
        db.execute(
            "UPDATE meta SET value=?1 WHERE key='sequence'",
            [next.to_string()],
        )?;
        Ok(next)
    }
    pub fn sequence(&self) -> StorageResult<u64> {
        let db = self.conn()?;
        Self::ready(&db)?;
        Ok(db.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='sequence'",
            [],
            |r| r.get(0),
        )?)
    }
    pub fn blob_path(&self, hash: &str) -> StorageResult<PathBuf> {
        if !valid_digest(hash) {
            return Err(StorageError::Invalid("Blob digest".into()));
        }
        Ok(self.root.join("blobs").join(hash))
    }
    pub(crate) fn publish(&self, bytes: &[u8]) -> StorageResult<String> {
        let hash = digest(bytes);
        let path = self.blob_path(&hash)?;
        if path.exists() {
            self.verify_blob(&hash)?;
            return Ok(hash);
        }
        let mut tmp = tempfile::NamedTempFile::new_in(self.root.join("staging"))?;
        tmp.write_all(bytes)?;
        self.hit(Boundary::BeforeBlobSync)?;
        tmp.as_file().sync_all()?;
        self.hit(Boundary::AfterBlobSync)?;
        self.hit(Boundary::BeforeRename)?;
        tmp.persist_noclobber(&path)
            .map_err(|e| StorageError::Io(e.error))?;
        self.hit(Boundary::AfterRename)?;
        sync_dir(&self.root.join("blobs"))?;
        Ok(hash)
    }
    pub(crate) fn verify_blob(&self, hash: &str) -> StorageResult<File> {
        let p = self.blob_path(hash)?;
        no_symlink(&p)?;
        let mut file = File::open(p)?;
        if !file.metadata()?.is_file() {
            return Err(StorageError::Corrupt("Non-file blob".into()));
        }
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        if format!("{:x}", hasher.finalize()) != hash {
            return Err(StorageError::Corrupt(format!("Blob hash mismatch: {hash}")));
        }
        Ok(file)
    }
    pub(crate) fn blob_bytes(&self, hash: &str) -> StorageResult<Vec<u8>> {
        let (size, bytes) = self.blob_window(hash, 0, 256 * 1024 * 1024)?;
        if size != bytes.len() as u64 {
            return Err(StorageError::Limit(
                "Blob materialization exceeds 256 MiB".into(),
            ));
        }
        Ok(bytes)
    }
    /// Return bytes collected during the hash pass itself, closing the verify-then-read race.
    pub(crate) fn blob_window(
        &self,
        hash: &str,
        offset: u64,
        max: usize,
    ) -> StorageResult<(u64, Vec<u8>)> {
        let path = self.blob_path(hash)?;
        no_symlink(&path)?;
        let mut file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(StorageError::Corrupt("Non-file blob".into()));
        }
        let end = offset
            .checked_add(max as u64)
            .ok_or_else(|| StorageError::Limit("Blob window overflow".into()))?;
        let mut hasher = Sha256::new();
        let mut bytes = Vec::new();
        let mut position = 0u64;
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            let next = position + n as u64;
            let start = position.max(offset);
            let stop = next.min(end);
            if start < stop {
                bytes.extend_from_slice(
                    &buffer[(start - position) as usize..(stop - position) as usize],
                );
            }
            position = next;
        }
        if format!("{:x}", hasher.finalize()) != hash {
            return Err(StorageError::Corrupt(format!("Blob hash mismatch: {hash}")));
        }
        Ok((position, bytes))
    }
    pub(crate) fn referenced_blobs(db: &Connection) -> StorageResult<BTreeSet<String>> {
        let mut q=db.prepare("SELECT digest FROM artifacts a WHERE NOT EXISTS(SELECT 1 FROM evictions e WHERE e.origin=a.origin AND e.artifact=a.id) UNION SELECT record_digest FROM artifacts UNION SELECT digest FROM archives UNION SELECT digest FROM memory UNION SELECT digest FROM metadata")?;
        Ok(q.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?)
    }
    pub fn reconcile(&self) -> StorageResult<Health> {
        let db = self.conn()?;
        let referenced = Self::referenced_blobs(&db)?;
        for hash in &referenced {
            self.verify_blob(hash)?;
        }
        let orphan_blobs = fs::read_dir(self.root.join("blobs"))?
            .filter_map(Result::ok)
            .filter(|e| !referenced.contains(&e.file_name().to_string_lossy().into_owned()))
            .count();
        let staged_files = fs::read_dir(self.root.join("staging"))?.count();
        // Preserve failed publication bytes for diagnosis; no incomplete metadata is invented.
        for entry in fs::read_dir(self.root.join("staging"))? {
            let e = entry?;
            let name = e.file_name();
            let target = self.root.join("quarantine").join(name);
            if !target.exists() {
                fs::rename(e.path(), target)?;
            }
        }
        sync_dir(&self.root.join("quarantine"))?;
        sync_dir(&self.root.join("staging"))?;
        db.execute("DELETE FROM catalog", [])?;
        db.execute(
            "INSERT INTO catalog SELECT origin,id,digest FROM artifacts WHERE encoding='utf8'",
            [],
        )?;
        let sequence = db.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key='sequence'",
            [],
            |r| r.get(0),
        )?;
        let uncertain_commands = db.query_row(
            "SELECT count(*) FROM commands WHERE state='uncertain'",
            [],
            |r| r.get(0),
        )?;
        Ok(Health {
            schema: 1,
            sequence,
            integrity: "ok".into(),
            orphan_blobs,
            staged_files,
            uncertain_commands,
        })
    }
    pub fn metadata(&self, kind: &str, id: &str) -> StorageResult<Option<Vec<u8>>> {
        let db = self.conn()?;
        Self::ready(&db)?;
        Ok(db
            .query_row(
                "SELECT raw FROM metadata WHERE kind=?1 AND id=?2",
                params![kind, id],
                |r| r.get(0),
            )
            .optional()?)
    }
}
