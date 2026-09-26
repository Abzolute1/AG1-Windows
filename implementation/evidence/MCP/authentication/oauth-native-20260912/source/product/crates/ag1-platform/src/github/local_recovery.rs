//! Explicit, privately authenticated repair of an interrupted Git transition.
//! A repair is another reviewed transition, never replay of an old approval.
use super::*;
use crate::change_receipts::Revision;

pub(super) struct Repair {
    pub(super) record: journal::Record,
    pub(super) lease: Option<Arc<journal::Lease>>,
    pub(super) candidate: Vec<u8>,
    pub(super) directory: Option<PinnedDirectory>,
    pub(super) modes: BTreeMap<String, u32>,
    pub(super) unmodified: Vec<(String, GitSnapshot)>,
    locks: Vec<(String, GitSnapshot)>,
    absent_locks: Vec<String>,
}
fn revision(value: &Value) -> Result<FileRevision> {
    let value: Revision =
        serde_json::from_value(value.clone()).map_err(|_| DomainError::Conflict)?;
    FileRevision::try_from(&value)
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or(DomainError::Conflict)
}
fn checkpoint(record: &journal::Record, name: &str) -> Result<Option<FileRevision>> {
    match record
        .store
        .snapshot(&format!("{}/{name}.json", record.directory), 4096)
    {
        Ok(bytes) => {
            let value: Value =
                serde_json::from_slice(&bytes.bytes).map_err(|_| DomainError::Conflict)?;
            if value["record_sha256"] != record.digest {
                return Err(DomainError::Conflict);
            }
            if value["revision"].is_null() {
                Ok(None)
            } else {
                revision(&value["revision"]).map(Some)
            }
        }
        Err(DomainError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}
fn entry(
    record: &journal::Record,
    row: &Value,
    i: usize,
    before: bool,
) -> Result<Option<GitEntry>> {
    let side = if before { "before" } else { "after" };
    let Some(mode) = row[format!("{side}_mode")].as_u64() else {
        return Ok(None);
    };
    if ![0o100644, 0o100755, 0o120000].contains(&mode) {
        return Err(DomainError::Conflict);
    }
    let bytes = record
        .store
        .snapshot(&format!("{}/{side}-{i}", record.directory), 8 * 1024 * 1024)?
        .bytes;
    let expected = if before {
        string(&row["before_revision"], "sha256")?
    } else {
        string(row, "after_sha256")?
    };
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(DomainError::Conflict);
    }
    Ok(Some(GitEntry {
        bytes,
        mode: mode as u32,
    }))
}
fn path_valid(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 2048
        && !path.contains('\0')
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|s| !s.is_empty() && ![".", "..", ".git"].contains(&s))
}

impl LocalGit {
    pub(in super::super) fn recovery_status(
        &self,
        args: &Value,
        c: &ScanControl<'_>,
    ) -> Result<Value> {
        c.check()?;
        self.authority(false)?;
        self.layout()?;
        let Some(record) = journal::load(self)? else {
            return Ok(
                json!({"operation":"recovery_status","outcome":"none","message":"No native Git transition is pending in this folder."}),
            );
        };
        if record.value["repository"] != args["repo"] {
            return Err(DomainError::PermissionDenied);
        }
        let rows = record.value["entries"]
            .as_array()
            .ok_or(DomainError::Conflict)?;
        Ok(
            json!({"operation":"recovery_status","outcome":if record.complete{"complete"}else{"pending"},"recovery_id":record.value["id"],"original_operation":record.value["operation"],"from_branch":record.value["from_branch"],"from_sha":record.value["from_sha"],"target_branch":record.value["target_branch"],"target_sha":record.value["target_sha"],"recovery_path":self.root().join(string(&record.value,"id")?),"file_count":rows.len(),"files":rows.iter().take(32).map(|r|json!({"path":r["path"]})).collect::<Vec<_>>(),"can_restore":record.value.get("recovery_of").is_none(),"limits":"Finish the retained target, or restore the pre-transition state, only after a new exact native review. Later writers and unknown/live/replaced locks are refused. No network request, automatic replay, or deletion of retained copies."}),
        )
    }
    pub(in super::super) fn prepare_recovery(
        &self,
        action: &str,
        args: &Value,
        scope: &str,
        c: &ScanControl<'_>,
    ) -> Result<LocalPlan> {
        c.check()?;
        self.layout()?;
        self.authority(action != "recovery_status")?;
        if args
            .as_object()
            .ok_or(DomainError::PermissionDenied)?
            .keys()
            .any(|k| !["action", "repo", "folder", "recovery_id", "choice"].contains(&k.as_str()))
        {
            return Err(DomainError::InvalidInput(
                "Recovery accepts only the native recovery_id and an explicit choice".into(),
            ));
        }
        crate::github::repo(args).map_err(DomainError::InvalidInput)?;
        let mut preview = self.recovery_status(args, c)?;
        preview["action"] = json!(action);
        preview["folder"] = json!(self.root());
        preview["repository"] = args["repo"].clone();
        let mut preview = preview;
        preview["metadata"] = self.metadata()?.identity();
        if action=="recover" {preview["storage"]=self.storage_admit(action,c)?;}
        let mut plan = LocalPlan {
            action: action.into(),
            args: args.clone(),
            scope: scope.into(),
            head: String::new(),
            branch: String::new(),
            index: None,
            sources: Vec::new(),
            config_revision: None,
            network: None,
            clone_target: None,
            transition: None,
            revision: String::new(),
            expires: Instant::now() + Duration::from_secs(300),
            preview,
        };
        if action == "recovery_status" {
            plan.revision = format!(
                "local-git-{:x}",
                Sha256::digest(json!([scope, plan.preview]).to_string())
            );
            return Ok(plan);
        }
        let record = journal::load(self)?.ok_or(DomainError::NotFound)?;
        if record.complete || record.value["id"] != args["recovery_id"] {
            return Err(DomainError::Conflict);
        }
        let restore = match args["choice"].as_str() {
            Some("finish") => false,
            Some("restore") => true,
            _ => {
                return Err(DomainError::InvalidInput(
                    "Choose finish or restore explicitly".into(),
                ));
            }
        };
        if restore && record.value.get("recovery_of").is_some() {
            return Err(DomainError::InvalidInput("An interrupted repair must be finished; its earlier retained versions are preserved, not automatically rewound again".into()));
        }
        let _lease = record.acquire(self)?;
        self.transition_idle()?;
        let (head, branch) = self.state(c)?;
        if ![
            string(&record.value, "from_sha")?,
            string(&record.value, "target_sha")?,
        ]
        .contains(&head.as_str())
            || ![
                string(&record.value, "from_branch")?,
                string(&record.value, "target_branch")?,
            ]
            .contains(&branch.as_str())
        {
            return Err(DomainError::Conflict);
        }
        let config = self.metadata_snapshot(".git/config", 1024 * 1024)?
            .revision;
        if config != revision(&record.value["config_before"])? {
            return Err(DomainError::Conflict);
        }
        let index = self.index()?;
        let candidate = record
            .store
            .snapshot(
                &format!(
                    "{}/index-{}",
                    record.directory,
                    if restore { "before" } else { "after" }
                ),
                8 * 1024 * 1024,
            )?
            .bytes;
        let expected = if restore {
            string(&record.value["index_before"], "sha256")?
        } else {
            string(&record.value, "index_after_sha256")?
        };
        if format!("{:x}", Sha256::digest(&candidate)) != expected {
            return Err(DomainError::Conflict);
        }
        let old_revision = if record.value["index_before"].is_null() {
            None
        } else {
            Some(revision(&record.value["index_before"])?)
        };
        let index_after = checkpoint(&record, "index-published")?;
        if let Some((bytes, r)) = &index {
            if bytes != &candidate
                && old_revision.as_ref() != Some(r)
                && index_after.as_ref() != Some(r)
            {
                return Err(DomainError::InvalidInput("The index was changed after the interrupted operation; it has not been overwritten".into()));
            }
        } else if index_after.is_some() {
            return Err(DomainError::InvalidInput("The published index was removed after the transition; recovery has not recreated it".into()));
        } else if let Some(old_revision) = old_revision {
            let backup = self
                .workspace
                .git_snapshot(&format!("{}/index-displaced", string(&record.value, "id")?))?
                .ok_or(DomainError::Conflict)?;
            let mut expected = old_revision.clone();
            expected.changed_ns = backup.revision.changed_ns;
            if backup.revision != expected {
                return Err(DomainError::Conflict);
            }
        }
        let target = string(
            &record.value,
            if restore { "from_sha" } else { "target_sha" },
        )?
        .to_string();
        let target_branch = string(
            &record.value,
            if restore {
                "from_branch"
            } else {
                "target_branch"
            },
        )?
        .to_string();
        self.run(&["check-ref-format", &target_branch], c)?;
        let branch_before = self.reference(&target_branch, c)?;
        if target_branch != branch && branch_before != "0".repeat(40) && branch_before != target {
            return Err(DomainError::Conflict);
        }
        self.branch_available(&target_branch, &branch, c)?;
        let rows = record.value["entries"]
            .as_array()
            .filter(|r| r.len() <= 1000)
            .ok_or(DomainError::Conflict)?;
        let mut versions = BTreeMap::new();
        let mut total = 0;
        for (i, row) in rows.iter().enumerate() {
            c.check()?;
            let path = string(row, "path")?;
            if !path_valid(path) || versions.contains_key(path) {
                return Err(DomainError::Conflict);
            }
            let before = entry(&record, row, i, true)?;
            let after = entry(&record, row, i, false)?;
            total += before.as_ref().map_or(0, |s| s.bytes.len())
                + after.as_ref().map_or(0, |s| s.bytes.len());
            if total > MAX_TRANSITION_BYTES {
                return Err(DomainError::LimitExceeded("recovery retained bytes".into()));
            }
            versions.insert(path.to_string(), (i, row, before, after));
        }
        let recovery = format!(".ag1-git-recovery-{}", crate::client::nonce()?);
        let mut changes = Vec::new();
        let mut blocked = BTreeSet::new();
        let mut remove = BTreeSet::new();
        let mut modes = BTreeMap::new();
        let mut unmodified = Vec::new();
        for (path, (i, row, before, after)) in &versions {
            c.check()?;
            self.authority(true)?;self.storage_check(c)?;
            let wanted = if restore { before } else { after };
            let parent = Path::new(path)
                .ancestors()
                .skip(1)
                .filter_map(Path::to_str)
                .find(|p| {
                    versions.contains_key(*p)
                        && self.workspace.git_snapshot(p).ok().flatten().is_some()
                });
            let is_directory = parent.is_none() && self.workspace.pin_directory(path).is_ok();
            let actual = if let Some(parent) = parent {
                let (_, _, old, new) = versions.get(parent).unwrap();
                let parent_wanted = if restore { old } else { new };
                if parent_wanted.is_some() {
                    if wanted.is_none() {
                        continue;
                    }
                    return Err(DomainError::Conflict);
                }
                blocked.insert(parent.into());
                None
            } else if is_directory {
                if wanted.is_none() {
                    continue;
                }
                // Only a shape represented by retained transition entries can
                // be removed. Inventory validation rejects all extra children.
                if !versions.keys().any(|p| p.starts_with(&format!("{path}/"))) {
                    return Err(DomainError::Conflict);
                }
                remove.insert(path.clone());
                None
            } else {
                self.workspace.git_snapshot(path)?
            };
            if actual.as_ref().map(|s| &s.entry) == wanted.as_ref() {
                if let Some(actual) = actual {
                    unmodified.push((path.clone(), actual));
                }
                continue;
            }
            let admitted = if let Some(actual) = &actual {
                before.is_some() && actual.revision == revision(&row["before_revision"])?
                    || checkpoint(&record, &format!("entry-{i}-published"))?.as_ref()
                        == Some(&actual.revision)
            } else if checkpoint(&record, &format!("entry-{i}-published"))?.is_some() {
                false
            } else if parent.is_some() || is_directory || before.is_none() {
                true
            } else {
                let backup = self.workspace.git_snapshot(string(row, "backup")?)?;
                backup.as_ref().is_some_and(|s| {
                    let Ok(mut expected) = revision(&row["before_revision"]) else {
                        return false;
                    };
                    expected.changed_ns = s.revision.changed_ns;
                    s.revision == expected
                })
            };
            if !admitted {
                return Err(DomainError::InvalidInput(format!(
                    "Later or unverified working entry blocks recovery: {path}"
                )));
            }
            if restore && before.is_some() {
                modes.insert(
                    path.clone(),
                    revision(&row["before_revision"])?.mode & 0o777,
                );
            }
            changes.push(Change {
                path: path.clone(),
                before: actual,
                after: wanted.clone(),
                backup: format!("{recovery}/entry-{}", changes.len()),
            });
        }
        let replaced = remove.clone();
        for path in versions.keys() {
            for parent in Path::new(path).ancestors().skip(1).filter_map(Path::to_str) {
                if replaced
                    .iter()
                    .any(|r| parent == r || parent.starts_with(&format!("{r}/")))
                    && self.workspace.pin_directory(parent).is_ok()
                {
                    remove.insert(parent.into());
                }
            }
        }
        let mut remove_dirs = remove
            .into_iter()
            .map(|p| self.workspace.pin_directory(&p))
            .collect::<Result<Vec<_>>>()?;
        remove_dirs.sort_by_key(|p| std::cmp::Reverse(p.relative_path().components().count()));
        let mut locks = Vec::new();
        let mut absent_locks = Vec::new();
        for lock in record.value["locks"]
            .as_array()
            .ok_or(DomainError::Conflict)?
        {
            let path = string(lock, "path")?;
            if let Some(actual) = self.metadata_git_snapshot(path)? {
                if actual.revision != revision(&lock["revision"])? {
                    return Err(DomainError::InvalidInput(format!(
                        "Unrecognized or changed Git lock is preserved: {path}"
                    )));
                }
                locks.push((path.to_string(), actual));
            } else {
                absent_locks.push(path.to_string());
            }
        }
        let target_lock = format!(".git/{target_branch}.lock");
        if !locks.iter().any(|(p, _)| p == &target_lock) && !absent_locks.contains(&target_lock) {
            if self.metadata_git_snapshot(&target_lock)?.is_some() {
                return Err(DomainError::InvalidInput(
                    "The recovery target branch has a foreign lock; it is preserved".into(),
                ));
            }
            absent_locks.push(target_lock);
        }
        plan.head = head;
        plan.branch = branch;
        plan.index = index;
        plan.config_revision = Some(config);
        let transition = Transition {
            target,
            branch: target_branch,
            branch_before,
            changes,
            recovery,
            remove_dirs,
            blocked_parents: blocked,
            repair: Some(Repair {
                record,
                lease: None,
                candidate,
                directory: None,
                modes,
                unmodified,
                locks,
                absent_locks,
            }),
        };
        self.transition_directories_clean(&transition, c)?;
        transition.describe(&mut plan.preview, self.root());
        plan.preview["choice"] = args["choice"].clone();
        plan.preview["recovery_warning"] = json!(
            "This is a new local repair, not a replay. Reviewed stale locks are retained, not deleted. Unknown/live/replaced locks and later file/index/ref changes are refused. Restore returns to the recorded branch/index/files but keeps any created branch and all recovery copies. No network or automatic retry."
        );
        plan.preview["retained_locks"] = json!(
            transition
                .repair
                .as_ref()
                .unwrap()
                .locks
                .iter()
                .map(|(p, _)| p)
                .collect::<Vec<_>>()
        );
        plan.revision = format!(
            "local-git-{:x}",
            Sha256::digest(
                json!([
                    scope,
                    plan.preview,
                    format!("{:?}", plan.index.as_ref().map(|(_, r)| r))
                ])
                .to_string()
            )
        );
        plan.transition = Some(transition);
        Ok(plan)
    }
    pub(in super::super) fn execute_recovery(
        &self,
        mut plan: LocalPlan,
        c: &ScanControl<'_>,
    ) -> Result<Value> {
        c.check()?;
        self.authority(true)?;self.storage_check(c)?;
        self.transition_idle()?;
        if self.state(c)? != (plan.head.clone(), plan.branch.clone())
            || self.index()?.as_ref().map(|(_, r)| r) != plan.index.as_ref().map(|(_, r)| r)
            || Some(
                self.metadata_snapshot(".git/config", 1024 * 1024)?
                    .revision,
            ) != plan.config_revision
        {
            return Err(DomainError::Conflict);
        }
        let t = plan.transition.as_mut().ok_or(DomainError::Conflict)?;
        self.transition_directories_clean(t, c)?;
        for change in &t.changes {
            let blocked = Path::new(&change.path)
                .ancestors()
                .skip(1)
                .filter_map(Path::to_str)
                .any(|p| t.blocked_parents.contains(p));
            let directory = t
                .remove_dirs
                .iter()
                .any(|p| p.relative_path() == Path::new(&change.path));
            if !blocked
                && !directory
                && self.workspace.git_snapshot(&change.path)?.as_ref() != change.before.as_ref()
            {
                return Err(DomainError::Conflict);
            }
        }
        let repair = t.repair.as_mut().ok_or(DomainError::Conflict)?;
        let lease = Arc::new(repair.record.acquire(self)?);
        let check = || -> Result<()> {
            c.check()?;
            self.authority(true)?;self.storage_check(c)?;
            self.workspace.verify_root()?;
            lease.verify()?;
            repair.record.verify(self)
        };
        for (path, snapshot) in &repair.locks {
            if self.metadata_git_snapshot(path)?.as_ref() != Some(snapshot) {
                return Err(DomainError::Conflict);
            }
        }
        for path in &repair.absent_locks {
            if self.metadata_git_snapshot(path)?.is_some() {
                return Err(DomainError::Conflict);
            }
        }
        for (path, snapshot) in &repair.unmodified {
            if self.workspace.git_snapshot(path)?.as_ref() != Some(snapshot) {
                return Err(DomainError::Conflict);
            }
        }
        // Persist the repair intent before retaining a stale lock or creating
        // any workspace recovery directory. Only this exact confirmed plan runs.
        repair.record.store.create_checked(&format!("{}/attempt-{}.json",repair.record.directory,t.recovery),&serde_json::to_vec(&json!({"record_sha256":repair.record.digest,"scope":plan.scope,"plan_revision":plan.revision,"choice":plan.args["choice"],"recovery":t.recovery,"locks":repair.locks.iter().enumerate().map(|(i,(path,s))|json!({"path":path,"revision":Revision::from(&s.revision),"backup":format!("{}/stale-lock-{i}",t.recovery)})).collect::<Vec<_>>()})).unwrap(),false,&check)?;
        #[cfg(test)]
        if let Some(hook) = &self.transition_hook {
            hook("after_recovery_intent", "");
        }
        check()?;
        let root = self.workspace.pin_directory("")?;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(root.process_cwd().join(&t.recovery))
            .map_err(io_error)?;
        let effects = || -> Result<()> {
            root.verify_identity()?;
            std::fs::File::open(root.process_cwd())
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
            for (i, (path, snapshot)) in repair.locks.iter().enumerate() {
                #[cfg(test)]
                if let Some(hook) = &self.transition_hook {
                    hook("before_recovery_lock", path);
                }
                self.publish_metadata(
                    path,
                    Some(snapshot),
                    None,
                    &format!("{}/stale-lock-{i}", t.recovery),
                    |_| check(),
                )?;
            }
            #[cfg(test)]
            if let Some(hook) = &self.transition_hook {
                hook("after_recovery_locks", "");
            }
            check()
        };
        effects().map_err(|_| DomainError::DurabilityUnknown)?;
        repair.directory = Some(self.workspace.pin_directory(&t.recovery)?);
        repair.lease = Some(lease);
        self.execute_transition(plan, c)
            .map_err(|_| DomainError::DurabilityUnknown)
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{run, setup};
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn explicit_recovery_finishes_or_restores_each_interrupted_transition_without_replay() {
        for choice in ["finish", "restore"] {
            for phase in [
                "before_files",
                "after_displace",
                "after_index",
                "after_refs",
                "after_workspace_complete",
            ] {
                let (_temp, mut local, original, target) = setup();
                let root = local.root().to_path_buf();
                let cancel = Arc::new(AtomicBool::new(false));
                let signal = cancel.clone();
                local.transition_hook = Some(Arc::new(move |current, _| {
                    if current == phase {
                        signal.store(true, Ordering::Release);
                    }
                }));
                let c = ScanControl {
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(60),
                };
                let plan = local
                    .prepare(
                        "checkout",
                        &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                        "scope",
                        &c,
                    )
                    .unwrap();
                assert_eq!(
                    local.execute(plan, "scope", &c).unwrap_err(),
                    DomainError::DurabilityUnknown,
                    "{choice}/{phase}"
                );
                cancel.store(false, Ordering::Release);
                local.transition_hook = None;
                let status = local
                    .recovery_status(&json!({"repo":"owned/repo"}), &c)
                    .unwrap();
                assert_eq!(status["outcome"], "pending");
                let args = json!({"repo":"owned/repo","recovery_id":status["recovery_id"],"choice":choice});
                let before = std::fs::read(root.join(".git/index")).unwrap();
                let head = run(&root, &["rev-parse", "HEAD"]);
                let plan = local
                    .prepare("recover", &args, "new-recovery-scope", &c)
                    .unwrap_or_else(|e| panic!("prepare {choice}/{phase}: {e:?}"));
                assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), before);
                assert_eq!(run(&root, &["rev-parse", "HEAD"]), head);
                let result = local
                    .execute(plan, "new-recovery-scope", &c)
                    .unwrap_or_else(|e| panic!("execute {choice}/{phase}: {e:?}"));
                assert_eq!(result["outcome"], "applied");
                assert_eq!(
                    run(&root, &["rev-parse", "HEAD"]),
                    if choice == "finish" {
                        target.as_str()
                    } else {
                        original.as_str()
                    }
                );
                for name in ["a", "b"] {
                    assert_eq!(
                        std::fs::read_to_string(root.join(name)).unwrap(),
                        format!(
                            "{} {name}",
                            if choice == "finish" {
                                "target"
                            } else {
                                "original"
                            }
                        )
                    );
                }
                assert!(local.transition_ready().is_ok());
                assert!(
                    local
                        .prepare("recover", &args, "new-recovery-scope", &c)
                        .is_err()
                );
                let record = journal::load(&local).unwrap().unwrap();
                assert!(record.complete);
                assert_eq!(record.value["recovery_of"]["id"], status["recovery_id"]);
                assert!(
                    Path::new(status["recovery_path"].as_str().unwrap())
                        .join("index-before")
                        .exists()
                );
            }
        }
    }

    #[test]
    fn recovery_rejects_stale_files_index_refs_scope_private_record_live_lease_and_foreign_locks() {
        for kind in [
            "file", "index", "ref", "scope", "record", "lease", "lock", "cancel", "config",
            "revoke", "expire",
        ] {
            let (_temp, mut local, _original, target) = setup();
            let root = local.root().to_path_buf();
            let cancel = Arc::new(AtomicBool::new(false));
            let signal = cancel.clone();
            local.transition_hook = Some(Arc::new(move |phase, _| {
                if phase == "before_files" {
                    signal.store(true, Ordering::Release);
                }
            }));
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(60),
            };
            let plan = local
                .prepare(
                    "checkout",
                    &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                    "scope",
                    &c,
                )
                .unwrap();
            assert_eq!(
                local.execute(plan, "scope", &c).unwrap_err(),
                DomainError::DurabilityUnknown
            );
            cancel.store(false, Ordering::Release);
            local.transition_hook = None;
            let record = journal::load(&local).unwrap().unwrap();
            let args =
                json!({"repo":"owned/repo","recovery_id":record.value["id"],"choice":"finish"});
            let access = crate::folder_access::FolderAccess::open(
                local.workspace.clone(),
                &local.state,
                "owned-recovery-grant",
            )
            .unwrap();
            let revision = access.snapshot().unwrap().selection.revision;
            local.grant = Some((access.clone(), revision, true));
            let mut plan = local.prepare("recover", &args, "repair", &c).unwrap();
            let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
            let mut held = None;
            match kind {
                "file" => std::fs::write(root.join("a"), b"later user writer").unwrap(),
                "index" => {
                    let mut bytes = std::fs::read(root.join(".git/index")).unwrap();
                    bytes.push(1);
                    std::fs::write(root.join(".git/index"), bytes).unwrap();
                }
                "ref" => {
                    run(&root, &["update-ref", "refs/heads/main", &target]);
                }
                "record" => {
                    let path = record
                        .store
                        .root()
                        .join(&record.directory)
                        .join("record.json");
                    let mut bytes = std::fs::read(&path).unwrap();
                    bytes.push(b' ');
                    std::fs::write(path, bytes).unwrap();
                }
                "lease" => held = Some(record.acquire(&local).unwrap()),
                "lock" => std::fs::write(root.join(".git/HEAD.lock"), b"foreign lock").unwrap(),
                "cancel" => cancel.store(true, Ordering::Release),
                "config" => {
                    let path = root.join(".git/config");
                    let mut bytes = std::fs::read(&path).unwrap();
                    bytes.extend_from_slice(b"\n# later user config\n");
                    std::fs::write(path, bytes).unwrap();
                }
                "revoke" => {
                    access
                        .change(crate::folder_access::Change {
                            revision,
                            action: "mode".into(),
                            path: root.clone(),
                            mode: Some(crate::folder_access::AccessMode::ReadOnly),
                        })
                        .unwrap();
                }
                "expire" => plan.expires = Instant::now(),
                _ => {}
            }
            let file = std::fs::read(root.join("a")).unwrap();
            let index = std::fs::read(root.join(".git/index")).unwrap();
            let head = run(&root, &["rev-parse", "HEAD"]);
            let error = local
                .execute(plan, if kind == "scope" { "other" } else { "repair" }, &c)
                .unwrap_err();
            assert_ne!(
                error,
                DomainError::DurabilityUnknown,
                "{kind}: pre-effect refusal"
            );
            assert!(!recovery.exists(), "{kind}");
            assert_eq!(std::fs::read(root.join("a")).unwrap(), file);
            assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(run(&root, &["rev-parse", "HEAD"]), head);
            if kind == "lock" {
                assert_eq!(
                    std::fs::read(root.join(".git/HEAD.lock")).unwrap(),
                    b"foreign lock"
                );
            }
            drop(held);
        }
    }

    #[test]
    fn a_repair_interrupted_again_requires_fresh_finish_and_preserves_earlier_originals() {
        let (_temp, mut local, _original, target) = setup();
        let root = local.root().to_path_buf();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        local.transition_hook = Some(Arc::new(move |phase, _| {
            if phase == "before_files" {
                signal.store(true, Ordering::Release);
            }
        }));
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(60),
        };
        let plan = local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &c,
            )
            .unwrap();
        assert!(local.execute(plan, "scope", &c).is_err());
        cancel.store(false, Ordering::Release);
        let record = journal::load(&local).unwrap().unwrap();
        let args = json!({"repo":"owned/repo","recovery_id":record.value["id"],"choice":"finish"});
        let signal = cancel.clone();
        local.transition_hook = Some(Arc::new(move |phase, _| {
            if phase == "after_displace" {
                signal.store(true, Ordering::Release);
            }
        }));
        let plan = local.prepare("recover", &args, "first-repair", &c).unwrap();
        assert_eq!(
            local.execute(plan, "first-repair", &c).unwrap_err(),
            DomainError::DurabilityUnknown
        );
        cancel.store(false, Ordering::Release);
        local.transition_hook = None;
        let next = journal::load(&local).unwrap().unwrap();
        assert!(!next.complete);
        assert_eq!(next.value["recovery_of"]["id"], record.value["id"]);
        assert!(local.prepare("recover", &args, "old-id", &c).is_err());
        assert!(
            local
                .prepare(
                    "recover",
                    &json!({"repo":"owned/repo","recovery_id":next.value["id"],"choice":"restore"}),
                    "no-recursive-rollback",
                    &c
                )
                .is_err()
        );
        let plan = local
            .prepare(
                "recover",
                &json!({"repo":"owned/repo","recovery_id":next.value["id"],"choice":"finish"}),
                "second-repair",
                &c,
            )
            .unwrap();
        assert_eq!(
            local.execute(plan, "second-repair", &c).unwrap()["outcome"],
            "applied"
        );
        assert_eq!(run(&root, &["rev-parse", "HEAD"]), target);
        assert!(local.transition_ready().is_ok());
        assert_eq!(
            record
                .store
                .snapshot(&format!("{}/before-0", record.directory), 1024)
                .unwrap()
                .bytes,
            b"original a"
        );
        assert_eq!(std::fs::read(root.join("a")).unwrap(), b"target a");
    }

    #[test]
    fn recovery_handles_directory_file_links_and_modes_without_removing_untracked_children() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for phase in ["after_displace", "after_index"] {
            for choice in ["finish", "restore"] {
                let (temp, mut local, original, _) = setup();
                let root = local.root().to_path_buf();
                run(&root, &["checkout", "--quiet", "target"]);
                std::fs::rename(root.join("a"), temp.path().join("owned-original-target-a"))
                    .unwrap();
                std::fs::create_dir(root.join("a")).unwrap();
                std::fs::write(root.join("a/child"), b"target child\0\xff").unwrap();
                symlink("b", root.join("link")).unwrap();
                std::fs::set_permissions(root.join("b"), std::fs::Permissions::from_mode(0o755))
                    .unwrap();
                run(&root, &["add", "--all"]);
                run(
                    &root,
                    &[
                        "commit",
                        "--quiet",
                        "-m",
                        "directory link executable fixture",
                    ],
                );
                let target = run(&root, &["rev-parse", "HEAD"]);
                run(&root, &["checkout", "--quiet", "main"]);
                let cancel = Arc::new(AtomicBool::new(false));
                let signal = cancel.clone();
                local.transition_hook = Some(Arc::new(move |current, _| {
                    if current == phase {
                        signal.store(true, Ordering::Release);
                    }
                }));
                let c = ScanControl {
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(60),
                };
                let plan = local
                    .prepare(
                        "checkout",
                        &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                        "scope",
                        &c,
                    )
                    .unwrap();
                assert_eq!(
                    local.execute(plan, "scope", &c).unwrap_err(),
                    DomainError::DurabilityUnknown
                );
                cancel.store(false, Ordering::Release);
                local.transition_hook = None;
                let status = local
                    .recovery_status(&json!({"repo":"owned/repo"}), &c)
                    .unwrap();
                let args = json!({"repo":"owned/repo","recovery_id":status["recovery_id"],"choice":choice});
                if phase == "after_index" && choice == "restore" {
                    std::fs::write(root.join("a/untracked"), b"later untracked user bytes")
                        .unwrap();
                    assert!(local.prepare("recover", &args, "repair", &c).is_err());
                    assert_eq!(
                        std::fs::read(root.join("a/untracked")).unwrap(),
                        b"later untracked user bytes"
                    );
                    std::fs::rename(root.join("a/untracked"), root.join("retained-untracked"))
                        .unwrap();
                }
                let plan = local
                    .prepare("recover", &args, "repair", &c)
                    .unwrap_or_else(|e| panic!("{phase}/{choice} prepare: {e:?}"));
                local
                    .execute(plan, "repair", &c)
                    .unwrap_or_else(|e| panic!("{phase}/{choice} execute: {e:?}"));
                if choice == "finish" {
                    assert_eq!(run(&root, &["rev-parse", "HEAD"]), target);
                    assert_eq!(
                        std::fs::read(root.join("a/child")).unwrap(),
                        b"target child\0\xff"
                    );
                    assert_eq!(
                        std::fs::read_link(root.join("link")).unwrap(),
                        Path::new("b")
                    );
                    assert_ne!(
                        std::fs::metadata(root.join("b"))
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o111,
                        0
                    );
                } else {
                    assert_eq!(run(&root, &["rev-parse", "HEAD"]), original);
                    assert_eq!(std::fs::read(root.join("a")).unwrap(), b"original a");
                    assert!(std::fs::symlink_metadata(root.join("link")).is_err());
                    assert_eq!(
                        std::fs::metadata(root.join("b"))
                            .unwrap()
                            .permissions()
                            .mode()
                            & 0o777,
                        0o644
                    );
                    if phase == "after_index" {
                        assert_eq!(
                            std::fs::read(root.join("retained-untracked")).unwrap(),
                            b"later untracked user bytes"
                        );
                    }
                }
                assert!(local.transition_ready().is_ok());
            }
        }
    }
}
