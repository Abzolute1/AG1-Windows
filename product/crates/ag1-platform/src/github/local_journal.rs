//! Private native transition custody. Workspace recovery documents are copies,
//! never authority to settle/replay a batch. The private root is excluded from
//! model/renderer folder grants by FolderAccess.
use super::*;
use crate::{change_receipts::Revision, process::ProcessIdentity};
use std::os::fd::AsRawFd;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};

pub(super) struct Journal {
    store: Arc<Workspace>,
    directory: String,
    digest: String,
    lease: Arc<Lease>,
}
/// Kernel-held ownership of one native operation, independent of the desktop
/// process lifetime. This file is private, persistent and never unlinked.
pub(super) struct Lease {
    parent: PinnedDirectory,
    file: std::fs::File,
}
impl Lease {
    fn acquire(store: &Workspace, bucket: &str) -> Result<Self> {
        let parent = store.pin_directory(bucket)?;
        let directory = std::fs::File::open(parent.process_cwd()).map_err(io_error)?;
        let fd = rustix::fs::openat(
            &directory,
            "operation.lock",
            rustix::fs::OFlags::RDWR
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(|_| DomainError::Conflict)?;
        let file = std::fs::File::from(fd);
        let metadata = file.metadata().map_err(io_error)?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(DomainError::PermissionDenied);
        }
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(DomainError::Conflict);
        }
        directory.sync_all().map_err(io_error)?;
        let lease = Self { parent, file };
        lease.verify()?;
        Ok(lease)
    }
    pub(super) fn verify(&self) -> Result<()> {
        self.parent.verify_identity()?;
        let current = std::fs::symlink_metadata(self.parent.process_cwd().join("operation.lock"))
            .map_err(io_error)?;
        let ours = self.file.metadata().map_err(io_error)?;
        if current.is_symlink()
            || !current.is_file()
            || (current.dev(), current.ino()) != (ours.dev(), ours.ino())
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

pub(super) struct Record {
    pub(super) store: Arc<Workspace>,
    pub(super) directory: String,
    pub(super) digest: String,
    pub(super) value: Value,
    pub(super) complete: bool,
    active_revision: FileRevision,
}
impl Record {
    pub(super) fn verify(&self, local: &LocalGit) -> Result<()> {
        local.verify_record_metadata(&self.value)?;
        let active = self
            .store
            .snapshot(&format!("{}/active.json", root_key(local)), 4096)?;
        if active.revision != self.active_revision {
            return Err(DomainError::Conflict);
        }
        let bytes = self
            .store
            .snapshot(&format!("{}/record.json", self.directory), 1024 * 1024)?;
        if self.value["root"] != identity(local)
            || format!("{:x}", Sha256::digest(&bytes.bytes)) != self.digest
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub(super) fn verify_repair(&self, local: &LocalGit, id: &str) -> Result<()> {
        if self.verify(local).is_ok() {
            return Ok(());
        }
        let active = load(local)?.ok_or(DomainError::Conflict)?;
        if active.complete
            || active.value["id"] != id
            || active.value["recovery_of"]["sha256"] != self.digest
        {
            return Err(DomainError::Conflict);
        }
        let retained = self
            .store
            .snapshot(&format!("{}/record.json", self.directory), 1024 * 1024)?;
        if format!("{:x}", Sha256::digest(&retained.bytes)) != self.digest {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub(super) fn acquire(&self, local: &LocalGit) -> Result<Lease> {
        self.verify(local)?;
        let lease = Lease::acquire(&self.store, &root_key(local))?;
        self.verify(local)?;
        // A released operation lease is not enough while the original native
        // child still exists. PID reuse and an earlier boot do not block repair.
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(io_error)?;
        if self.value["boot_id"] == boot.trim() {
            for key in if self.value["lease_version"] == 1 {
                &["child"][..]
            } else {
                &["child", "owner"][..]
            } {
                let child = &self.value[*key];
                let pid = child["pid"]
                    .as_u64()
                    .filter(|p| *p > 0 && *p <= u32::MAX as u64)
                    .ok_or(DomainError::Conflict)? as u32;
                match crate::process::observe(pid) {
                    Ok(current) if Some(current.start_ticks)==child["start_ticks"].as_u64()=>return Err(DomainError::Unavailable("The original Git operation is still active, or predates native operation leases; recovery has not started".into())),
                    Ok(_)|Err(DomainError::NotFound)=>{},Err(e)=>return Err(e),
                }
            }
        }
        Ok(lease)
    }
}
fn process(identity: &ProcessIdentity) -> Value {
    json!({"pid":identity.pid,"start_ticks":identity.start_ticks,"executable":identity.executable,"device":identity.device,"inode":identity.inode})
}
fn root_key(local: &LocalGit) -> String {
    format!(
        "transition-{:x}",
        Sha256::digest(local.root().as_os_str().as_bytes())
    )
}
fn identity(local: &LocalGit) -> Value {
    let (device, inode) = local.workspace.directory_identity();
    json!({"path":local.root(),"device":device,"inode":inode})
}
fn private(local: &LocalGit) -> Result<Arc<Workspace>> {
    // Host composition must not put recovery authority inside a granted root.
    // Normal FolderAccess additionally excludes ancestors of private storage.
    let state = local
        .github
        .as_ref()
        .map(|s| s.git_custody_root())
        .unwrap_or(&local.state);
    if state.starts_with(local.root()) || local.root().starts_with(state) {
        return Err(DomainError::PermissionDenied);
    }
    WorkspaceService::new()?.register(state)
}
fn read_optional(store: &Workspace, name: &str) -> Result<Option<Value>> {
    match store.snapshot(name, 1024 * 1024) {
        Ok(value) => Ok(Some(
            serde_json::from_slice(&value.bytes).map_err(|_| DomainError::Conflict)?,
        )),
        Err(DomainError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}
pub(super) fn load(local: &LocalGit) -> Result<Option<Record>> {
    let store = private(local)?;
    let snapshot = match store.snapshot(&format!("{}/active.json", root_key(local)), 4096) {
        Ok(v) => v,
        Err(DomainError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let active: Value =
        serde_json::from_slice(&snapshot.bytes).map_err(|_| DomainError::Conflict)?;
    if active["version"] != 1 || active["root"] != identity(local) {
        return Err(DomainError::Conflict);
    }
    let id = active["id"]
        .as_str()
        .filter(|s| {
            s.starts_with(".ag1-git-recovery-")
                && s.len() == 50
                && s[18..].bytes().all(|b| b.is_ascii_hexdigit())
        })
        .ok_or(DomainError::Conflict)?;
    let directory = format!("{}/{id}", root_key(local));
    let record = store.snapshot(&format!("{directory}/record.json"), 1024 * 1024)?;
    let digest = format!("{:x}", Sha256::digest(&record.bytes));
    if active["record_sha256"] != digest {
        return Err(DomainError::Conflict);
    }
    let value: Value = serde_json::from_slice(&record.bytes).map_err(|_| DomainError::Conflict)?;
    if value["version"] != 1 || value["root"] != identity(local) || value["id"] != id {
        return Err(DomainError::Conflict);
    }
    local.verify_record_metadata(&value)?;
    let complete = read_optional(&store, &format!("{directory}/completed.json"))?;
    let complete = complete
        .as_ref()
        .is_some_and(|c| c["outcome"] == "applied" && c["record_sha256"] == digest);
    Ok(Some(Record {
        store,
        directory,
        digest,
        value,
        complete,
        active_revision: snapshot.revision,
    }))
}
pub(super) fn ready(local: &LocalGit) -> Result<()> {
    let Some(record) = load(local)? else {
        return Ok(());
    };
    if record.complete {
        return Ok(());
    }
    let id = record.value["id"].as_str().ok_or(DomainError::Conflict)?;
    Err(DomainError::Unavailable(format!(
        "A native Git transition needs recovery at {}. Its private custody record is still pending; editing workspace markers cannot complete it. No automatic replay.",
        local.root().join(id).display()
    )))
}
impl Journal {
    pub(super) fn begin(
        local: &LocalGit,
        plan: &LocalPlan,
        t: &Transition,
        candidate: &[u8],
        head: &[u8],
        child: &ProcessIdentity,
        check: &dyn Fn() -> Result<()>,
    ) -> Result<Self> {
        check()?;
        if let Some(repair) = &t.repair {
            repair.record.verify(local)?;
        } else {
            ready(local)?;
        }
        let store = private(local)?;
        let bucket = root_key(local);
        let pin = store.pin_directory("")?;
        match std::fs::DirBuilder::new()
            .mode(0o700)
            .create(pin.process_cwd().join(&bucket))
        {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_error(e)),
        }
        pin.verify_identity()?;
        std::fs::File::open(pin.process_cwd())
            .map_err(io_error)?
            .sync_all()
            .map_err(io_error)?;
        let parent = store.pin_directory(&bucket)?;
        let metadata = std::fs::metadata(parent.process_cwd()).map_err(io_error)?;
        if metadata.mode() & 0o077 != 0 {
            return Err(DomainError::PermissionDenied);
        }
        let lease = if let Some(repair) = &t.repair {
            repair.lease.as_ref().ok_or(DomainError::Conflict)?.clone()
        } else {
            Arc::new(Lease::acquire(&store, &bucket)?)
        };
        let outer_check = check;
        let check = &|| -> Result<()> {
            outer_check()?;
            lease.verify()
        };
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(parent.process_cwd().join(&t.recovery))
            .map_err(io_error)?;
        parent.verify_identity()?;
        let directory = format!("{bucket}/{}", t.recovery);
        let save = |name: &str, bytes: &[u8]| -> Result<()> {
            store.create_checked(&format!("{directory}/{name}"), bytes, false, check)?;
            Ok(())
        };
        save(
            "index-before",
            plan.index
                .as_ref()
                .map(|(bytes, _)| bytes.as_slice())
                .unwrap_or_default(),
        )?;
        save("index-after", candidate)?;
        save("head-before", head)?;
        for (i, entry) in t.changes.iter().enumerate() {
            if let Some(before) = &entry.before {
                save(&format!("before-{i}"), &before.entry.bytes)?;
            }
            if let Some(after) = &entry.after {
                save(&format!("after-{i}"), &after.bytes)?;
            }
        }
        let mut locks = Vec::new();
        let mut paths = vec![
            ".git/HEAD.lock".to_string(),
            ".git/index.lock".to_string(),
            format!(".git/{}.lock", t.branch),
        ];
        if plan.action == "pull" {
            paths.push(format!(
                ".git/{}.lock",
                plan.preview["tracking_ref"]
                    .as_str()
                    .ok_or(DomainError::Conflict)?
            ));
        }
        paths.sort();
        paths.dedup();
        for path in paths {
            let lock = local.metadata_snapshot(&path, 4096)?;
            locks.push(json!({"path":path,"revision":Revision::from(&lock.revision)}));
        }
        let owner = crate::process::observe(std::process::id())?;
        let boot_id =
            std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(io_error)?;
        let record = json!({"version":1,"id":t.recovery,"root":identity(local),"scope":plan.scope,"plan_revision":plan.revision,"operation":plan.action,"repository":plan.args["repo"],"account":plan.preview.get("account"),"from_branch":plan.branch,"from_sha":plan.head,"target_branch":t.branch,"target_sha":t.target,"target_branch_before":t.branch_before,"index_before":plan.index.as_ref().map(|(_,r)|Revision::from(r)),"index_after_sha256":format!("{:x}",Sha256::digest(candidate)),"head_before_sha256":format!("{:x}",Sha256::digest(head)),"config_before":plan.config_revision.as_ref().map(Revision::from),"owner":process(&owner),"child":process(child),"locks":locks,"remove_directories":t.remove_dirs.iter().map(|p|p.relative_path()).collect::<Vec<_>>(),"entries":t.changes.iter().enumerate().map(|(i,v)|json!({"path":v.path,"backup":v.backup,"before_revision":v.before.as_ref().map(|s|Revision::from(&s.revision)),"before_mode":v.before.as_ref().map(|s|s.entry.mode),"before_bytes":v.before.as_ref().map(|_|format!("before-{i}")),"after_bytes":v.after.as_ref().map(|_|format!("after-{i}")),"after_sha256":v.after.as_ref().map(|s|format!("{:x}",Sha256::digest(&s.bytes))),"after_mode":v.after.as_ref().map(|s|s.mode)})).collect::<Vec<_>>()});
        let mut record = record;
        record["metadata"] = local.metadata()?.identity();
        record["boot_id"] = json!(boot_id.trim());
        record["lease_version"] = json!(1);
        if let Some(repair) = &t.repair {
            record["recovery_of"] = json!({"id":repair.record.value["id"],"sha256":repair.record.digest,"choice":plan.args["choice"]});
        }
        let bytes = serde_json::to_vec(&record).map_err(|_| DomainError::Conflict)?;
        if bytes.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "native Git recovery record".into(),
            ));
        }
        save("record.json", &bytes)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        // Activate only after all immutable preimages/candidates are durable.
        // A crash after activation is pending even before a workspace marker.
        let active = format!("{bucket}/active.json");
        let value = serde_json::to_vec(
            &json!({"version":1,"root":identity(local),"id":t.recovery,"record_sha256":digest}),
        )
        .unwrap();
        check()?;
        if let Some(repair) = &t.repair {
            repair.record.verify(local)?;
        } else {
            ready(local)?;
        }
        match store.snapshot(&active, 4096) {
            Ok(before) => {
                store.replace_checked(&active, &before.revision, &value, check)?;
            }
            Err(DomainError::NotFound) => {
                store.create_checked(&active, &value, false, check)?;
            }
            Err(e) => return Err(e),
        }
        Ok(Self {
            store,
            directory,
            digest,
            lease,
        })
    }
    pub(super) fn checkpoint(
        &self,
        name: &str,
        snapshot: Option<&GitSnapshot>,
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        self.lease.verify()?;
        let value = json!({"record_sha256":self.digest,"revision":snapshot.map(|s|Revision::from(&s.revision)),"git_mode":snapshot.map(|s|s.entry.mode)});
        self.store.create_checked(
            &format!("{}/{name}.json", self.directory),
            &serde_json::to_vec(&value).unwrap(),
            false,
            check,
        )?;
        Ok(())
    }
    pub(super) fn complete(&self, check: impl Fn() -> Result<()>) -> Result<()> {
        self.lease.verify()?;
        self.store.create_checked(
            &format!("{}/completed.json", self.directory),
            &serde_json::to_vec(&json!({"outcome":"applied","record_sha256":self.digest})).unwrap(),
            false,
            check,
        )?;
        Ok(())
    }
}
