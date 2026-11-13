//! Aggregate admission budgets for native Git metadata and retained recovery.
//! Existing user bytes are never pruned to make room. The process-wide/private
//! file lease serializes new Git writes across projects and desktop processes.
//! External writers are not filesystem-quota controlled; runtime checks stop
//! further owned work and a post-effect excess is reported as uncertain.
use super::*;
use crate::workspace::PinnedDirectory;
use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
const MIB: u64 = 1024 * 1024;
#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub repository: u64,
    pub journal: u64,
    pub recovery: u64,
    pub entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            repository: 4 * 1024 * MIB,
            journal: 1024 * MIB,
            recovery: 2 * 1024 * MIB,
            entries: 100_000,
        }
    }
}
#[derive(Default, Clone, Copy)]
struct Usage {
    bytes: u64,
    entries: usize,
}
impl Usage {
    fn add(&mut self, size: u64, limit: Limits) -> Result<()> {
        self.bytes = self.bytes.checked_add(size).ok_or(DomainError::Conflict)?;
        self.entries += 1;
        if self.entries > limit.entries || self.bytes > limit.repository.max(limit.recovery) {
            return Err(DomainError::LimitExceeded(
                "Git storage scan; retained bytes were preserved".into(),
            ));
        }
        Ok(())
    }
}
fn walk(
    root: &Workspace,
    path: &str,
    usage: &mut Usage,
    limits: Limits,
    c: &ScanControl<'_>,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        return Err(DomainError::LimitExceeded(
            "Git metadata directory depth".into(),
        ));
    }
    c.check()?;
    let pin = root.pin_directory(path)?;
    for entry in std::fs::read_dir(pin.process_cwd()).map_err(io_error)? {
        c.check()?;
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let p = Path::new(path).join(&name);
        let p = p.to_str().ok_or(DomainError::PermissionDenied)?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(io_error)?;
        // Never follow links, FIFOs, sockets or device nodes while accounting.
        // Directory traversal goes through Workspace's no-follow pins.
        usage.add(
            if metadata.is_file() || metadata.is_symlink() {
                metadata.len()
            } else {
                0
            },
            limits,
        )?;
        if metadata.is_dir() {
            walk(root, p, usage, limits, c, depth + 1)?;
        }
    }
    pin.verify_identity()
}
fn selected(
    root: &Workspace,
    usage: &mut Usage,
    limits: Limits,
    c: &ScanControl<'_>,
    private: bool,
) -> Result<()> {
    let pin = root.pin_directory("")?;
    let mut count = 0;
    for entry in std::fs::read_dir(pin.process_cwd()).map_err(io_error)? {
        c.check()?;
        count += 1;
        if count > limits.entries {
            return Err(DomainError::LimitExceeded(
                "Git storage directory entries".into(),
            ));
        }
        let entry = entry.map_err(io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let chosen = if private {
            name.starts_with("transition-") || name.starts_with("git-")
        } else {
            name.starts_with(".ag1-git-recovery-") || name.starts_with(".ag1-clone-")
        };
        if !chosen {
            continue;
        }
        let m = std::fs::symlink_metadata(entry.path()).map_err(io_error)?;
        usage.add(
            if m.is_file() || m.is_symlink() {
                m.len()
            } else {
                0
            },
            limits,
        )?;
        if m.is_dir() {
            walk(root, name, usage, limits, c, 1)?;
        }
    }
    pin.verify_identity()
}
fn reservation(action: &str) -> (u64, u64, usize) {
    match action {
        "fetch" | "fetch_pr" => (1024 * MIB, 16 * MIB, 1024),
        "clone" => (1280 * MIB, 16 * MIB, 12000),
        "checkout" | "pull" | "recover" => (128 * MIB, 128 * MIB, 6000),
        "stage" | "unstage" => (32 * MIB, 16 * MIB, 512),
        "commit" => (64 * MIB, 16 * MIB, 2048),
        _ => (MIB, MIB, 64),
    }
}
fn room(
    usage: Usage,
    extra: u64,
    entries: usize,
    cap: u64,
    limits: Limits,
    label: &str,
) -> Result<()> {
    if usage.bytes.saturating_add(extra) > cap
        || usage.entries.saturating_add(entries) > limits.entries
    {
        return Err(DomainError::LimitExceeded(format!(
            "{label}: not enough room for this Git operation. Existing bytes and recovery records are preserved; reads remain available. Free space or archive completed recovery records before a fresh review."
        )));
    }
    Ok(())
}
pub(super) struct Lease {
    pin: PinnedDirectory,
    file: std::fs::File,
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Another thread may be between fork and exec with an inherited copy.
        // Release the open-description lock explicitly after owned work ends;
        // CLOEXEC alone could leave it held until that unrelated child execs.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
impl Lease {
    pub(super) fn acquire(local: &LocalGit) -> Result<Self> {
        let root = local.git_private_storage()?;
        let pin = root.pin_directory("")?;
        let dir = std::fs::File::open(pin.process_cwd()).map_err(io_error)?;
        let fd = rustix::fs::openat(
            &dir,
            "git-storage.lock",
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(|_| DomainError::Conflict)?;
        let file = std::fs::File::from(fd);
        let m = file.metadata().map_err(io_error)?;
        if !m.is_file() || m.nlink() != 1 || m.mode() & 0o077 != 0 {
            return Err(DomainError::PermissionDenied);
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(DomainError::Unavailable("Another native Git write is using the shared storage budget; review again after it finishes".into()));
        }
        let result = Self { pin, file };
        result.verify()?;
        Ok(result)
    }
    pub(super) fn verify(&self) -> Result<()> {
        self.pin.verify_identity()?;
        let actual = std::fs::symlink_metadata(self.pin.process_cwd().join("git-storage.lock"))
            .map_err(io_error)?;
        let ours = self.file.metadata().map_err(io_error)?;
        if !actual.is_file()
            || actual.is_symlink()
            || (actual.dev(), actual.ino()) != (ours.dev(), ours.ino())
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}
impl LocalGit {
    fn storage_limits(&self) -> Limits {
        #[cfg(test)]
        if let Some(limits) = self.storage_limits {
            return limits;
        }
        Limits::default()
    }
    pub(super) fn git_private_storage(&self) -> Result<Arc<Workspace>> {
        let path = self
            .github
            .as_ref()
            .map(|s| s.git_custody_root())
            .unwrap_or(&self.state);
        if path.starts_with(self.root()) || self.root().starts_with(path) {
            return Err(DomainError::PermissionDenied);
        }
        WorkspaceService::new()?.register(path)
    }
    fn storage_usage(&self, c: &ScanControl<'_>) -> Result<(Usage, Usage)> {
        let limits = self.storage_limits();
        let mut repository = Usage::default();
        let mut journal = Usage::default();
        let metadata = self
            .metadata
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clone();
        if let Some(metadata) = metadata {
            metadata.verify(self, false)?;
            walk(&metadata.common, "", &mut repository, limits, c, 0)?;
        }
        selected(&self.workspace, &mut repository, limits, c, false)?;
        let private = self.git_private_storage()?;
        selected(&private, &mut journal, limits, c, true)?;
        if private.root() != self.state {
            let legacy = WorkspaceService::new()?.register(&self.state)?;
            selected(&legacy, &mut journal, limits, c, true)?;
        }
        Ok((repository, journal))
    }
    pub(super) fn storage_admit(&self, action: &str, c: &ScanControl<'_>) -> Result<Value> {
        let limits = self.storage_limits();
        let (repository, journal) = self.storage_usage(c)?;
        let (repo_extra, journal_extra, entries) = reservation(action);
        let journal_limit = if action == "recover" {
            limits.recovery
        } else {
            limits.journal
        };
        room(
            repository,
            repo_extra,
            entries,
            limits.repository,
            limits,
            "Repository Git storage",
        )?;
        room(
            journal,
            journal_extra,
            entries,
            journal_limit,
            limits,
            "Private Git recovery storage",
        )?;
        Ok(
            json!({"repository_bytes":repository.bytes,"repository_limit":limits.repository,"journal_bytes":journal.bytes,"journal_limit":journal_limit,"repository_reservation":repo_extra,"journal_reservation":journal_extra,"entry_limit":limits.entries,"limits":"Aggregate native Git metadata and retained recovery; existing source files are not pruned. New writes are serialized. External writers are not controlled by a filesystem quota. Pending recovery has a separate reserved allowance; no automatic cleanup."}),
        )
    }
    pub(super) fn storage_check(&self, c: &ScanControl<'_>) -> Result<()> {
        c.check()?;
        let mut polled = self
            .storage_polled
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if polled.elapsed() < Duration::from_millis(100) {
            return Ok(());
        }
        let result = self.storage_check_now(c);
        *polled = Instant::now();
        result
    }
    pub(super) fn storage_check_now(&self, c: &ScanControl<'_>) -> Result<()> {
        let mode = self
            .storage_active
            .load(std::sync::atomic::Ordering::Acquire);
        if mode == 0 {
            return Ok(());
        }
        let limits = self.storage_limits();
        let (repository, journal) = self.storage_usage(c)?;
        room(
            repository,
            0,
            0,
            limits.repository,
            limits,
            "Repository Git storage",
        )?;
        room(
            journal,
            0,
            0,
            if mode == 2 {
                limits.recovery
            } else {
                limits.journal
            },
            limits,
            "Private Git recovery storage",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    fn sparse(path: &Path, len: u64) {
        std::fs::File::create(path).unwrap().set_len(len).unwrap();
    }
    #[test]
    fn aggregate_repository_journal_and_entry_budgets_recheck_before_writes_and_preserve_reads() {
        for area in ["repository", "journal", "entries"] {
            let (_temp, mut local, _, _) = super::super::transition::tests::setup();
            local.storage_limits = Some(Limits {
                repository: 64 * MIB,
                journal: 32 * MIB,
                recovery: 256 * MIB,
                entries: 600,
            });
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            std::fs::write(local.root().join("a"), b"new stage").unwrap();
            let args = json!({"repo":"owned/repo","paths":["a"]});
            let plan = local.prepare("stage", &args, "scope", &c).unwrap();
            let index = local.index().unwrap().unwrap().0;
            assert!(
                !local.state.join("git-storage.lock").exists(),
                "preview reserves no persistent lease"
            );
            let retained = match area {
                "repository" => {
                    let p = local.root().join(".git/objects/owned-sparse");
                    sparse(&p, 33 * MIB);
                    p
                }
                "journal" => {
                    let p = local.state.join("git-index-recovery-owned");
                    sparse(&p, 17 * MIB);
                    p
                }
                "entries" => {
                    for i in 0..100 {
                        std::fs::write(
                            local.root().join(format!(".git/objects/owned-{i}")),
                            b"owned",
                        )
                        .unwrap();
                    }
                    local.root().join(".git/objects/owned-0")
                }
                _ => unreachable!(),
            };
            assert!(
                matches!(
                    local.prepare("stage", &args, "scope", &c),
                    Err(DomainError::LimitExceeded(_))
                ),
                "{area}"
            );
            assert!(
                matches!(
                    local.execute(plan, "scope", &c),
                    Err(DomainError::LimitExceeded(_))
                ),
                "{area}"
            );
            assert_eq!(local.index().unwrap().unwrap().0, index);
            assert!(retained.exists());
            assert!(
                !local
                    .metadata()
                    .unwrap()
                    .git
                    .root()
                    .join("index.lock")
                    .exists()
            );
            let plan = local
                .prepare("status", &json!({"repo":"owned/repo"}), "scope", &c)
                .unwrap();
            assert!(
                local.execute(plan, "scope", &c).is_ok(),
                "Reads remain available at the storage cap"
            );
        }
    }
    #[test]
    fn quota_scan_never_follows_external_links_and_shared_lease_is_exclusive() {
        let (temp, local, _, _) = super::super::transition::tests::setup();
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let outside = temp.path().join("outside");
        sparse(&outside, 8 * 1024 * MIB);
        std::os::unix::fs::symlink(&outside, local.root().join(".git/objects/owned-link")).unwrap();
        local.layout().unwrap();
        let usage = local.storage_admit("stage", &c).unwrap();
        assert!(usage["repository_bytes"].as_u64().unwrap() < MIB);
        let lease = Lease::acquire(&local).unwrap();
        assert!(Lease::acquire(&local).is_err());
        lease.verify().unwrap();
        let inherited = lease.file.try_clone().unwrap();
        drop(lease);
        assert!(Lease::acquire(&local).is_ok());
        drop(inherited);
        cancel.store(true, Ordering::Release);
        assert_eq!(
            local.storage_admit("stage", &c),
            Err(DomainError::Cancelled)
        );
        assert_eq!(std::fs::metadata(outside).unwrap().len(), 8 * 1024 * MIB);
    }
    #[test]
    fn external_growth_after_publication_is_uncertain_and_never_prunes_retained_recovery() {
        let (_temp, mut local, _, target) = super::super::transition::tests::setup();
        let root = local.root().to_path_buf();
        let growth = root.join(".git/objects/owned-growth");
        let observed = growth.clone();
        local.transition_hook = Some(Arc::new(move |phase, _| {
            if phase == "after_index" {
                sparse(&observed, 5 * 1024 * MIB)
            }
        }));
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(40),
        };
        let plan = local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &c,
            )
            .unwrap();
        let recovery = PathBuf::from(plan.preview["recovery_path"].as_str().unwrap());
        assert_eq!(
            local.execute(plan, "scope", &c),
            Err(DomainError::DurabilityUnknown)
        );
        assert!(recovery.join("index-before").exists());
        assert!(growth.exists());
        let plan = local
            .prepare(
                "recovery_status",
                &json!({"repo":"owned/repo"}),
                "inspect",
                &c,
            )
            .unwrap();
        let status = local.execute(plan, "inspect", &c).unwrap();
        assert!(["pending", "complete"].contains(&status["outcome"].as_str().unwrap()));
        assert_eq!(std::fs::metadata(growth).unwrap().len(), 5 * 1024 * MIB);
        assert!(recovery.join("index-before").exists());
    }
    #[test]
    fn retained_pending_recovery_uses_reserved_allowance_after_normal_journal_budget_is_full() {
        let (_temp, mut local, _, target) = super::super::transition::tests::setup();
        let cancel = Arc::new(AtomicBool::new(false));
        let observed = cancel.clone();
        local.transition_hook = Some(Arc::new(move |phase, _| {
            if phase == "after_index" {
                observed.store(true, Ordering::Release)
            }
        }));
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(40),
        };
        let plan = local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &c,
            )
            .unwrap();
        let id = Path::new(plan.preview["recovery_path"].as_str().unwrap())
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            local.execute(plan, "scope", &c),
            Err(DomainError::DurabilityUnknown)
        );
        cancel.store(false, Ordering::Release);
        local.transition_hook = None;
        let retained = local.state.join("git-index-recovery-retained");
        sparse(&retained, 960 * MIB);
        assert!(matches!(
            local.storage_admit("checkout", &c),
            Err(DomainError::LimitExceeded(_))
        ));
        let plan = local
            .prepare(
                "recover",
                &json!({"repo":"owned/repo","recovery_id":id,"choice":"restore"}),
                "fresh",
                &c,
            )
            .unwrap();
        assert_eq!(plan.preview["storage"]["journal_limit"], 2 * 1024 * MIB);
        local.execute(plan, "fresh", &c).unwrap();
        assert_eq!(
            std::fs::read(local.root().join("a")).unwrap(),
            b"original a"
        );
        assert_eq!(std::fs::metadata(retained).unwrap().len(), 960 * MIB);
    }
}
