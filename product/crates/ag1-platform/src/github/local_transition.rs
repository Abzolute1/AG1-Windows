//! Reviewed checkout custody. Git constructs a private candidate index and
//! holds its reference locks; only native checked entry publication writes
//! working files. Partial transitions retain a durable marker and originals.
use super::*;
use crate::workspace::{GitEntry, GitSnapshot, PinnedDirectory};
use std::{cell::Cell, collections::BTreeSet, os::unix::fs::DirBuilderExt};
#[path = "local_journal.rs"]
mod journal;
#[path = "local_recovery.rs"]
mod recovery;

const MARKER: &str = ".git/ag1-transition.json";
const MAX_TRANSITION_BYTES: usize = 64 * 1024 * 1024;
#[derive(Clone, PartialEq, Eq)]
struct TreeEntry {
    mode: u32,
    sha: String,
}
struct Change {
    path: String,
    before: Option<GitSnapshot>,
    after: Option<GitEntry>,
    backup: String,
}
pub(super) struct Transition {
    target: String,
    branch: String,
    branch_before: String,
    changes: Vec<Change>,
    recovery: String,
    remove_dirs: Vec<PinnedDirectory>,
    blocked_parents: BTreeSet<String>,
    repair: Option<recovery::Repair>,
}
impl Transition {
    pub(super) fn describe(&self, preview: &mut Value, root: &Path) {
        preview["target_branch"] = json!(self.branch);
        preview["target_sha"] = json!(self.target);
        preview["recovery_path"] = json!(root.join(&self.recovery));
        preview["file_count"] = json!(self.changes.len());
        preview["files"]=json!(self.changes.iter().take(32).map(|c|json!({"path":c.path,"operation":if c.after.is_none(){"delete"}else if c.before.is_none(){"create"}else{"replace"},"before_sha256":c.before.as_ref().map(|s|&s.revision.sha256),"after_sha256":c.after.as_ref().map(|s|format!("{:x}",Sha256::digest(&s.bytes)))})).collect::<Vec<_>>());
        preview["files_clipped"] = json!(self.changes.len() > 32);
        preview["limits"] = json!(
            "Exact target commit; up to 1,000 changed entries / 64 MiB retained bytes / 8 MiB per file. Unrelated dirty and staged files are preserved. Conflicting dirty entries and untracked collisions are refused. No hooks, filters, signing, submodule initialization or automatic rollback. Original files/index and a durable transition record are retained for recovery."
        );
    }
}
impl LocalGit {
    fn transition_idle(&self) -> Result<()> {
        let git = self.metadata()?.git.pin_directory("")?;
        for name in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "REBASE_HEAD",
            "rebase-merge",
            "rebase-apply",
            "sequencer",
            "BISECT_LOG",
        ] {
            match std::fs::symlink_metadata(git.process_cwd().join(name)) {
                Ok(_)=>return Err(DomainError::Unavailable("Finish or explicitly abort the existing Git merge, rebase, cherry-pick, revert or bisect before checkout/pull; AG1 has not changed that operation".into())),
                Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},
                Err(e)=>return Err(io_error(e)),
            }
        }
        git.verify_identity()
    }
    pub(super) fn transition_ready(&self) -> Result<()> {
        journal::ready(self)?;
        let marker = match self.metadata_snapshot(MARKER, 1024 * 1024) {
            Ok(v) => v,
            Err(DomainError::NotFound) => return Ok(()),
            Err(e) => return Err(e),
        };
        let value: Value =
            serde_json::from_slice(&marker.bytes).map_err(|_| DomainError::Conflict)?;
        let dir = value["recovery"]
            .as_str()
            .filter(|s| s.starts_with(".ag1-git-recovery-") && !s.contains('/'))
            .ok_or(DomainError::Conflict)?;
        let complete = self
            .workspace
            .snapshot(&format!("{dir}/completed.json"), 4096)
            .ok()
            .and_then(|s| serde_json::from_slice::<Value>(&s.bytes).ok());
        if complete
            .as_ref()
            .is_some_and(|c| c["outcome"] == "applied" && c["target_sha"] == value["target_sha"])
        {
            return Ok(());
        }
        Err(DomainError::Unavailable(format!(
            "A Git transition needs recovery before another local change. Inspect {} and {MARKER}; no automatic replay.",
            self.root().join(dir).display()
        )))
    }
    fn tree(&self, sha: &str, c: &ScanControl<'_>) -> Result<BTreeMap<String, TreeEntry>> {
        let (bytes, ok) = self.command(
            &["ls-tree".into(), "-r".into(), "-z".into(), sha.into()],
            None,
            None,
            BTreeMap::new(),
            c,
        )?;
        if !ok {
            return Err(DomainError::Conflict);
        }
        let mut result = BTreeMap::new();
        for record in bytes.split(|b| *b == 0).filter(|b| !b.is_empty()) {
            let offset = record
                .iter()
                .position(|b| *b == b'\t')
                .ok_or(DomainError::Conflict)?;
            let fields = std::str::from_utf8(&record[..offset])
                .map_err(|_| DomainError::Conflict)?
                .split_whitespace()
                .collect::<Vec<_>>();
            let path = std::str::from_utf8(&record[offset + 1..]).map_err(|_| {
                DomainError::InvalidInput("UTF-8 Git filenames are required for checkout".into())
            })?;
            if path.is_empty()
                || path.len() > 2048
                || path
                    .split('/')
                    .any(|p| p.is_empty() || [".", "..", ".git"].contains(&p))
                || fields.len() != 3
                || !oid(fields[2])
            {
                return Err(DomainError::Conflict);
            }
            let mode = u32::from_str_radix(fields[0], 8).map_err(|_| DomainError::Conflict)?;
            if ![0o100644, 0o100755, 0o120000, 0o160000].contains(&mode) {
                return Err(DomainError::Conflict);
            }
            if result
                .insert(
                    path.into(),
                    TreeEntry {
                        mode,
                        sha: fields[2].into(),
                    },
                )
                .is_some()
                || result.len() > 20000
            {
                return Err(DomainError::LimitExceeded("checkout tree entries".into()));
            }
        }
        Ok(result)
    }
    fn blob(&self, entry: &TreeEntry, c: &ScanControl<'_>) -> Result<GitEntry> {
        let (bytes, ok) = self.command(
            &["cat-file".into(), "blob".into(), entry.sha.clone()],
            None,
            None,
            BTreeMap::new(),
            c,
        )?;
        if !ok {
            return Err(DomainError::Conflict);
        }
        Ok(GitEntry {
            bytes,
            mode: entry.mode,
        })
    }
    fn branch_available(&self, target: &str, current: &str, c: &ScanControl<'_>) -> Result<()> {
        if target == current {
            return Ok(());
        }
        let text = self.run(&["worktree", "list", "--porcelain"], c)?;
        if text.lines().any(|l| l == format!("branch {target}")) {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    fn transition_directories_clean(&self, t: &Transition, c: &ScanControl<'_>) -> Result<()> {
        let files = t
            .changes
            .iter()
            .filter(|v| v.before.is_some())
            .map(|v| v.path.as_str())
            .collect::<BTreeSet<_>>();
        let dirs = t
            .remove_dirs
            .iter()
            .map(|p| p.relative_path().to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>();
        let mut count = 0;
        for dir in &t.remove_dirs {
            dir.verify_identity()?;
            for entry in std::fs::read_dir(dir.process_cwd()).map_err(io_error)? {
                c.check()?;
                self.authority(true)?;
                self.storage_check(c)?;
                count += 1;
                if count > 20000 {
                    return Err(DomainError::LimitExceeded(
                        "checkout directory inventory".into(),
                    ));
                }
                let entry = entry.map_err(io_error)?;
                let name = dir
                    .relative_path()
                    .join(entry.file_name())
                    .to_str()
                    .ok_or(DomainError::Conflict)?
                    .to_string();
                if entry.file_type().map_err(io_error)?.is_dir() {
                    if !dirs.contains(&name) {
                        return Err(DomainError::Conflict);
                    }
                } else if !files.contains(name.as_str()) {
                    return Err(DomainError::InvalidInput(format!(
                        "Untracked entry blocks directory replacement: {name}"
                    )));
                }
            }
            dir.verify_identity()?;
        }
        Ok(())
    }
    pub(super) fn prepare_transition(
        &self,
        action: &str,
        args: &Value,
        head: &str,
        current: &str,
        c: &ScanControl<'_>,
    ) -> Result<Transition> {
        self.transition_idle()?;
        let pulling = action == "pull";
        if pulling && args.get("target_sha").is_some() {
            return Err(DomainError::InvalidInput(
                "Pull uses expected_remote_sha, not a separate target_sha".into(),
            ));
        }
        let target=args[if pulling{"expected_remote_sha"}else{"target_sha"}].as_str().filter(|s|s.len()==40&&oid(s)).ok_or_else(||DomainError::InvalidInput("An exact reviewed target commit (40 characters), already fetched into this repository, is required".into()))?;
        if head.is_empty() || self.run(&["cat-file", "-t", target], c)?.trim() != "commit" {
            return Err(DomainError::Conflict);
        }
        let branch = format!(
            "refs/heads/{}",
            args["branch"]
                .as_str()
                .ok_or(DomainError::PermissionDenied)?
        );
        let branch_before = self.reference(&branch, c)?;
        if pulling {
            if branch != current || branch_before != head {
                return Err(DomainError::InvalidInput("Pull fast-forwards only the current same-named branch; switch branches explicitly first".into()));
            }
            let (_, ancestor) = self.command(
                &[
                    "merge-base".into(),
                    "--is-ancestor".into(),
                    head.into(),
                    target.into(),
                ],
                None,
                None,
                BTreeMap::new(),
                c,
            )?;
            if !ancestor {
                return Err(DomainError::InvalidInput("The local and reviewed remote branches are not a fast-forward. No merge, rebase or discard was performed".into()));
            }
        } else if branch_before != "0".repeat(40) && branch_before != target {
            return Err(DomainError::Conflict);
        }
        self.branch_available(&branch, current, c)?;
        let before = self.tree(head, c)?;
        let after = self.tree(target, c)?;
        let paths = before
            .keys()
            .chain(after.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let changed = paths
            .into_iter()
            .filter(|p| before.get(p) != after.get(p))
            .collect::<Vec<_>>();
        if changed.len() > 1000 {
            return Err(DomainError::LimitExceeded(
                "checkout changed entries".into(),
            ));
        }
        let recovery = format!(".ag1-git-recovery-{}", crate::client::nonce()?);
        let mut changes = Vec::new();
        let mut total = 0usize;
        let mut remove_dirs = Vec::new();
        let mut blocked_parents = BTreeSet::new();
        for path in &changed {
            c.check()?;
            self.authority(true)?;
            self.storage_check(c)?;
            let old = before.get(path);
            let new = after.get(path);
            // Updating a gitlink never traverses or initializes its checkout.
            // File/submodule directory conversions need separate authority.
            if old.is_some_and(|e| e.mode == 0o160000) || new.is_some_and(|e| e.mode == 0o160000) {
                if old.is_some_and(|e| e.mode != 0o160000)
                    || new.is_some_and(|e| e.mode != 0o160000)
                {
                    return Err(DomainError::InvalidInput(
                        "File/submodule conversion requires explicit submodule handling".into(),
                    ));
                }
                continue;
            }
            let ancestor = Path::new(path)
                .ancestors()
                .skip(1)
                .filter_map(Path::to_str)
                .find(|p| before.contains_key(*p) && !after.contains_key(*p));
            let directory_replaced =
                old.is_none() && before.keys().any(|p| p.starts_with(&format!("{path}/")));
            let actual = if let Some(parent) = ancestor {
                blocked_parents.insert(parent.into());
                None
            } else if directory_replaced {
                remove_dirs.push(self.workspace.pin_directory(path)?);
                None
            } else {
                self.workspace.git_snapshot(path)?
            };
            let old_blob = old.map(|e| self.blob(e, c)).transpose()?;
            if actual.as_ref().map(|s| &s.entry) != old_blob.as_ref() {
                return Err(DomainError::InvalidInput(format!(
                    "Checkout conflicts with working entry {path}; preserve or commit it first"
                )));
            }
            let new_blob = new.map(|e| self.blob(e, c)).transpose()?;
            total += actual.as_ref().map(|s| s.entry.bytes.len()).unwrap_or(0)
                + new_blob.as_ref().map(|s| s.bytes.len()).unwrap_or(0);
            if total > MAX_TRANSITION_BYTES {
                return Err(DomainError::LimitExceeded("checkout retained bytes".into()));
            }
            changes.push(Change {
                path: path.clone(),
                before: actual,
                after: new_blob,
                backup: format!("{recovery}/entry-{}", changes.len()),
            });
        }
        // Include nested old directories for D/F transitions. Only empty,
        // pinned directories can later be removed; untracked bytes survive.
        let replaced = remove_dirs
            .iter()
            .map(|p| p.relative_path().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let mut directory_names = replaced.iter().cloned().collect::<BTreeSet<_>>();
        for path in before.keys() {
            for parent in Path::new(path).ancestors().skip(1).filter_map(Path::to_str) {
                if replaced
                    .iter()
                    .any(|r| parent == r || parent.starts_with(&format!("{r}/")))
                {
                    directory_names.insert(parent.into());
                }
            }
        }
        remove_dirs = directory_names
            .into_iter()
            .map(|p| self.workspace.pin_directory(&p))
            .collect::<Result<Vec<_>>>()?;
        remove_dirs.sort_by_key(|p| std::cmp::Reverse(p.relative_path().components().count()));
        let transition = Transition {
            target: target.into(),
            branch,
            branch_before,
            changes,
            recovery,
            remove_dirs,
            blocked_parents,
            repair: None,
        };
        self.transition_directories_clean(&transition, c)?;
        Ok(transition)
    }
    pub(super) fn execute_transition(
        &self,
        mut plan: LocalPlan,
        c: &ScanControl<'_>,
    ) -> Result<Value> {
        let t = plan
            .transition
            .take()
            .ok_or(DomainError::PermissionDenied)?;
        let lock = IndexLock::acquire(&self.metadata()?.git)?;
        let pulling = plan.action == "pull";
        let advancing = pulling || t.repair.is_some() && t.branch == plan.branch;
        let tracking = if pulling {
            Some(
                plan.preview["tracking_ref"]
                    .as_str()
                    .ok_or(DomainError::PermissionDenied)?,
            )
        } else {
            None
        };
        let recheck = || -> Result<()> {
            c.check()?;
            self.authority(true)?;
            self.storage_check(c)?;
            self.workspace.verify_root()?;
            lock.verify()?;
            self.transition_idle()?;
            if let Some(repair) = &t.repair {
                repair.record.verify_repair(self, &t.recovery)?;
                repair
                    .lease
                    .as_ref()
                    .ok_or(DomainError::Conflict)?
                    .verify()?;
                for (path, snapshot) in &repair.unmodified {
                    if self.workspace.git_snapshot(path)?.as_ref() != Some(snapshot) {
                        return Err(DomainError::Conflict);
                    }
                }
            }
            if self.state(c)? != (plan.head.clone(), plan.branch.clone())
                || self.index()?.as_ref().map(|(_, r)| r) != plan.index.as_ref().map(|(_, r)| r)
                || Some(self.metadata_snapshot(".git/config", 1024 * 1024)?.revision)
                    != plan.config_revision
                || self.reference(&t.branch, c)? != t.branch_before
            {
                return Err(DomainError::Conflict);
            }
            self.branch_available(&t.branch, &plan.branch, c)?;
            if let Some(tracking) = tracking {
                if self.reference(tracking, c)? != t.target {
                    return Err(DomainError::Conflict);
                }
            }
            for change in &t.changes {
                let blocked = Path::new(&change.path)
                    .ancestors()
                    .skip(1)
                    .filter_map(Path::to_str)
                    .any(|p| t.blocked_parents.contains(p));
                let dir = t
                    .remove_dirs
                    .iter()
                    .any(|p| p.relative_path() == Path::new(&change.path));
                if !blocked
                    && !dir
                    && self.workspace.git_snapshot(&change.path)?.as_ref() != change.before.as_ref()
                {
                    return Err(DomainError::Conflict);
                }
            }
            self.transition_directories_clean(&t, c)?;
            Ok(())
        };
        recheck()?;
        if t.repair.is_none() && t.target == plan.head && t.branch == plan.branch {
            return Ok(
                json!({"operation":plan.action,"outcome":"unchanged","branch":t.branch,"commit":t.target,"working_files_changed":false}),
            );
        }
        let temp = tempfile::Builder::new()
            .prefix("git-transition-index-")
            .tempdir_in(&self.state)
            .map_err(io_error)?;
        let old_bytes = plan
            .index
            .as_ref()
            .map(|(bytes, _)| bytes.as_slice())
            .unwrap_or_default();
        if plan.index.is_none() && t.repair.is_none() {
            return Err(DomainError::Conflict);
        }
        std::fs::write(temp.path().join("index"), old_bytes).map_err(io_error)?;
        let private = WorkspaceService::new()?.register(temp.path())?;
        let env = BTreeMap::from([("GIT_INDEX_FILE".into(), "/git-private/index".into())]);
        let ok = if t.repair.is_some() {
            true
        } else {
            self.command(
                &[
                    "read-tree".into(),
                    "-m".into(),
                    "-i".into(),
                    "--no-sparse-checkout".into(),
                    plan.head.clone(),
                    t.target.clone(),
                ],
                None,
                Some(&private),
                env.clone(),
                c,
            )?
            .1
        };
        if !ok {
            return Err(DomainError::InvalidInput(
                "The Git transition conflicts with staged changes; the original index is unchanged"
                    .into(),
            ));
        }
        let candidate = if let Some(repair) = &t.repair {
            repair.candidate.clone()
        } else {
            private.snapshot("index", 8 * 1024 * 1024)?.bytes
        };
        let index_before = self.metadata_git_snapshot(".git/index")?;
        if index_before.as_ref().map(|s| &s.revision) != plan.index.as_ref().map(|(_, r)| r) {
            return Err(DomainError::Conflict);
        }
        let head_before = self.metadata_snapshot(".git/HEAD", 4096)?;
        let marker_before = self.metadata_git_snapshot(MARKER)?;
        let began = Cell::new(false);
        let custody = std::cell::RefCell::new(None);
        let guard = |child: &crate::process::ProcessIdentity| -> Result<()> {
            recheck()?;
            if let Some(repair) = &t.repair {
                repair.record.verify(self)?;
            } else {
                self.transition_ready()?;
            }
            if self.metadata_git_snapshot(MARKER)?.as_ref() != marker_before.as_ref() {
                return Err(DomainError::Conflict);
            }
            let root = self.workspace.pin_directory("")?;
            let check = || -> Result<()> {
                c.check()?;
                self.authority(true)?;
                self.storage_check(c)?;
                self.workspace.verify_root()?;
                root.verify_identity()?;
                lock.verify()?;
                self.transition_idle()?;
                Ok(())
            };
            let journal = journal::Journal::begin(
                self,
                &plan,
                &t,
                &candidate,
                &head_before.bytes,
                child,
                &check,
            )?;
            began.set(true);
            *custody.borrow_mut() = Some(journal);
            #[cfg(test)]
            if let Some(hook) = &self.transition_hook {
                hook("after_custody", "");
            }
            // Retaining a large candidate may take time. Approval's exact
            // preimages/config/index must still hold before workspace effects.
            recheck()?;
            if let Some(repair) = &t.repair {
                repair
                    .directory
                    .as_ref()
                    .ok_or(DomainError::Conflict)?
                    .verify_identity()?;
            } else {
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(root.process_cwd().join(&t.recovery))
                    .map_err(io_error)?;
            }
            std::fs::File::open(root.process_cwd())
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
            let custody = custody.borrow();
            let journal = custody.as_ref().ok_or(DomainError::Conflict)?;
            self.workspace.create_checked(
                &format!("{}/index-before", t.recovery),
                old_bytes,
                false,
                &check,
            )?;
            self.workspace.create_checked(
                &format!("{}/head-before", t.recovery),
                &head_before.bytes,
                false,
                &check,
            )?;
            self.workspace.create_checked(
                &format!("{}/index-after", t.recovery),
                &candidate,
                false,
                &check,
            )?;
            let manifest = json!({"version":1,"operation":plan.action,"recovery":t.recovery,"from_branch":plan.branch,"from_sha":plan.head,"target_branch":t.branch,"target_sha":t.target,"entries":t.changes.iter().map(|v|json!({"path":v.path,"backup":v.backup,"before_sha256":v.before.as_ref().map(|s|&s.revision.sha256),"after_sha256":v.after.as_ref().map(|s|format!("{:x}",Sha256::digest(&s.bytes))),"after_mode":v.after.as_ref().map(|s|s.mode)})).collect::<Vec<_>>()});
            let bytes = serde_json::to_vec(&manifest).map_err(|_| DomainError::Conflict)?;
            self.workspace.create_checked(
                &format!("{}/manifest.json", t.recovery),
                &bytes,
                false,
                &check,
            )?;
            self.publish_metadata(
                MARKER,
                marker_before.as_ref(),
                Some(&GitEntry {
                    bytes,
                    mode: 0o100644,
                }),
                &format!("{}/previous-marker", t.recovery),
                |_| check(),
            )?;
            recheck()?;
            #[cfg(test)]
            if let Some(hook) = &self.transition_hook {
                hook("before_files", "");
            }
            // Remove old entries first, keeping their inodes in central recovery.
            // Publication of replacement bytes is a second exclusive operation.
            for (i, change) in t
                .changes
                .iter()
                .enumerate()
                .filter(|(_, v)| v.before.is_some())
            {
                self.workspace.publish_git_entry_checked(
                    &change.path,
                    change.before.as_ref(),
                    None,
                    &change.backup,
                    |_| check(),
                )?;
                let displaced = self
                    .workspace
                    .git_snapshot(&change.backup)?
                    .ok_or(DomainError::Conflict)?;
                let original = change.before.as_ref().ok_or(DomainError::Conflict)?;
                let mut expected = original.revision.clone();
                expected.changed_ns = displaced.revision.changed_ns;
                if displaced.revision != expected || displaced.entry != original.entry {
                    return Err(DomainError::Conflict);
                }
                journal.checkpoint(&format!("entry-{i}-displaced"), Some(&displaced), &check)?;
                #[cfg(test)]
                if let Some(hook) = &self.transition_hook {
                    hook("after_displace", &change.path);
                }
            }
            for directory in &t.remove_dirs {
                directory.verify_identity()?;
                let pin = self.workspace.pin_directory(
                    directory
                        .relative_path()
                        .to_str()
                        .ok_or(DomainError::Conflict)?,
                )?;
                if !self.workspace.remove_empty_directory_checked(pin, &check)? {
                    return Err(DomainError::Conflict);
                }
            }
            for (i, change) in t
                .changes
                .iter()
                .enumerate()
                .filter(|(_, v)| v.after.is_some())
            {
                if let Some(parent) = Path::new(&change.path)
                    .parent()
                    .and_then(Path::to_str)
                    .filter(|s| !s.is_empty())
                {
                    self.workspace.create_directories_checked(parent, &check)?;
                }
                let mut published = self.workspace.publish_git_entry_checked(
                    &change.path,
                    None,
                    change.after.as_ref(),
                    &format!("{}/new-{i}", t.recovery),
                    |_| check(),
                )?;
                if let (Some(old), Some(new)) = (&change.before, published.as_mut()) {
                    if old.entry.mode != 0o120000 && new.entry.mode != 0o120000 {
                        let access = old.revision.mode & 0o666;
                        let mode = access
                            | if new.entry.mode == 0o100755 {
                                (access & 0o444) >> 2
                            } else {
                                0
                            };
                        check()?;
                        *new = self
                            .workspace
                            .set_git_mode_checked(&change.path, new, mode)?;
                    }
                }
                if let (Some(mode), Some(new)) = (
                    t.repair.as_ref().and_then(|r| r.modes.get(&change.path)),
                    published.as_mut(),
                ) {
                    if new.entry.mode != 0o120000 {
                        *new = self
                            .workspace
                            .set_git_mode_checked(&change.path, new, *mode)?;
                    }
                }
                journal.checkpoint(&format!("entry-{i}-published"), published.as_ref(), &check)?;
            }
            // HEAD/branch locks are held by the prepared Git child throughout.
            check()?;
            let published_index = if index_before
                .as_ref()
                .is_some_and(|s| s.entry.bytes == candidate)
            {
                index_before.clone()
            } else {
                self.publish_metadata(
                    ".git/index",
                    index_before.as_ref(),
                    Some(&GitEntry {
                        bytes: candidate.clone(),
                        mode: 0o100644,
                    }),
                    &format!("{}/index-displaced", t.recovery),
                    |_| check(),
                )?
            };
            journal.checkpoint("index-published", published_index.as_ref(), &check)?;
            #[cfg(test)]
            if let Some(hook) = &self.transition_hook {
                hook("after_index", "");
            }
            check()?;
            if Some(self.metadata_snapshot(".git/config", 1024 * 1024)?.revision)
                != plan.config_revision
            {
                return Err(DomainError::Conflict);
            }
            if self.state(c)? != (plan.head.clone(), plan.branch.clone()) {
                return Err(DomainError::Conflict);
            }
            Ok(())
        };
        let branch_command = if t.branch_before == "0".repeat(40) {
            format!("create {} {}\n", t.branch, t.target)
        } else {
            format!("verify {} {}\n", t.branch, t.target)
        };
        // Verifying the old referent in this transaction would implicitly
        // update HEAD a second time, which Git rejects. The symbolic expected
        // value locks HEAD; native checks retain its exact old commit identity.
        let transaction = if advancing {
            format!(
                "start\nupdate HEAD {} {}\n{}prepare\n",
                t.target,
                plan.head,
                tracking
                    .map(|r| format!("verify {r} {}\n", t.target))
                    .unwrap_or_default()
            )
        } else {
            format!(
                "start\nsymref-update HEAD {} ref {}\n{}prepare\n",
                t.branch, plan.branch, branch_command
            )
        };
        // Pull dereferences HEAD so Git locks both its symbolic identity and
        // current branch. The callback checks that identity before file effects.
        // Checkout instead updates the symbolic HEAD itself with no-deref.
        let mut command = vec!["update-ref".into()];
        if !advancing {
            command.push("--no-deref".into());
        }
        command.push("--stdin".into());
        let result = self.command_guarded(
            &command,
            Some(transaction.as_bytes()),
            Some(&private),
            env,
            c,
            Some(&guard),
            false,
        );
        match result {
            Ok((_, true)) => {}
            Ok((_, false)) => {
                return Err(if began.get() {
                    DomainError::DurabilityUnknown
                } else {
                    DomainError::InvalidInput("Git cannot prepare this exact branch transaction; no working files changed".into())
                });
            }
            Err(e) => {
                return Err(if began.get() {
                    DomainError::DurabilityUnknown
                } else {
                    e
                });
            }
        }
        let finish = || -> Result<Value> {
            #[cfg(test)]
            if let Some(hook) = &self.transition_hook {
                hook("after_refs", "");
            }
            c.check()?;
            self.authority(true)?;
            self.storage_check(c)?;
            lock.verify()?;
            if self.state(c)? != (t.target.clone(), t.branch.clone()) {
                return Err(DomainError::Conflict);
            }
            let result = json!({"operation":plan.action,"outcome":"applied","branch":t.branch,"commit":t.target,"file_count":t.changes.len(),"recovery_path":self.root().join(&t.recovery),"automatic_retry":false,"submodules_initialized":false,"fast_forward":pulling,"recovery_choice":plan.args.get("choice")});
            self.workspace.create_checked(
                &format!("{}/completed.json", t.recovery),
                &serde_json::to_vec(&json!({"outcome":"applied","target_sha":t.target})).unwrap(),
                false,
                || {
                    c.check()?;
                    self.authority(true)
                },
            )?;
            #[cfg(test)]
            if let Some(hook) = &self.transition_hook {
                hook("after_workspace_complete", "");
            }
            custody
                .borrow()
                .as_ref()
                .ok_or(DomainError::Conflict)?
                .complete(|| {
                    c.check()?;
                    self.authority(true)
                })?;
            Ok(result)
        };
        finish().map_err(|_| DomainError::DurabilityUnknown)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{
        process::Command,
        sync::atomic::{AtomicBool, Ordering},
    };
    fn git(root: &Path, args: &[&str]) -> std::process::Output {
        Command::new("/usr/bin/git")
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .current_dir(root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Owned")
            .env("GIT_COMMITTER_NAME", "Owned")
            .env("GIT_AUTHOR_EMAIL", "owned@example.invalid")
            .env("GIT_COMMITTER_EMAIL", "owned@example.invalid")
            .output()
            .unwrap()
    }
    pub(super) fn run(root: &Path, args: &[&str]) -> String {
        let result = git(root, args);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().into()
    }
    pub(in super::super) fn setup() -> (tempfile::TempDir, LocalGit, String, String) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("work");
        std::fs::create_dir(&root).unwrap();
        run(&root, &["init", "--quiet", "-b", "main"]);
        run(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owned/repo.git",
            ],
        );
        for name in ["a", "b"] {
            std::fs::write(root.join(name), format!("original {name}")).unwrap();
        }
        run(&root, &["add", "--all"]);
        run(&root, &["commit", "--quiet", "-m", "baseline"]);
        let original = run(&root, &["rev-parse", "HEAD"]);
        run(&root, &["checkout", "--quiet", "-b", "target"]);
        for name in ["a", "b"] {
            std::fs::write(root.join(name), format!("target {name}")).unwrap();
        }
        run(&root, &["add", "--all"]);
        run(&root, &["commit", "--quiet", "-m", "target"]);
        let target = run(&root, &["rev-parse", "HEAD"]);
        run(&root, &["checkout", "--quiet", "main"]);
        let helper = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ag1-process-host");
        let local = LocalGit::new(
            WorkspaceService::new().unwrap().register(&root).unwrap(),
            &helper,
            Path::new("/usr/bin/git"),
            Path::new("/usr/bin/bwrap"),
            &temp.path().join("state"),
        )
        .unwrap();
        (temp, local, original, target)
    }
    #[test]
    fn interrupted_checkout_holds_git_locks_preserves_racer_and_blocks_replay_after_reopen() {
        let (_temp, mut local, original, target) = setup();
        let root = local.root().to_path_buf();
        let observed = root.clone();
        let target_copy = target.clone();
        local.transition_hook = Some(Arc::new(move |phase, path| {
            if !["after_refs", "after_workspace_complete"].contains(&phase) {
                assert!(observed.join(".git/index.lock").exists());
                assert!(observed.join(".git/HEAD.lock").exists());
                assert!(observed.join(".git/refs/heads/target.lock").exists());
            }
            if phase == "before_files" {
                assert!(
                    !git(&observed, &["update-ref", "refs/heads/main", &target_copy])
                        .status
                        .success()
                );
            }
            if phase == "after_displace" && path == "a" {
                std::fs::write(observed.join("a"), b"later user writer").unwrap();
            }
        }));
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let plan = local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &c,
            )
            .unwrap();
        let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
        let index = std::fs::read(root.join(".git/index")).unwrap();
        assert_eq!(
            local.execute(plan, "scope", &c).unwrap_err(),
            DomainError::DurabilityUnknown
        );
        assert_eq!(run(&root, &["rev-parse", "HEAD"]), original);
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read(root.join("a")).unwrap(), b"later user writer");
        assert_eq!(
            std::fs::read(recovery.join("entry-0")).unwrap(),
            b"original a"
        );
        assert_eq!(
            std::fs::read(recovery.join("entry-1")).unwrap(),
            b"original b"
        );
        assert!(recovery.join("manifest.json").exists());
        assert!(!recovery.join("completed.json").exists());
        assert!(!root.join(".git/index.lock").exists());
        assert!(!root.join(".git/HEAD.lock").exists());
        assert!(!root.join(".git/refs/heads/target.lock").exists());
        let helper = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ag1-process-host");
        let reopened = LocalGit::new(
            WorkspaceService::new().unwrap().register(&root).unwrap(),
            &helper,
            Path::new("/usr/bin/git"),
            Path::new("/usr/bin/bwrap"),
            &local.state,
        )
        .unwrap();
        let error = reopened
            .prepare(
                "stage",
                &json!({"repo":"owned/repo","paths":["a"]}),
                "scope",
                &c,
            )
            .err()
            .unwrap();
        assert!(format!("{error:?}").contains("needs recovery"));
        assert!(
            reopened
                .prepare("status", &json!({"repo":"owned/repo"}), "scope", &c)
                .is_ok()
        );
        // Untrusted working-folder files can neither settle nor erase native
        // custody. A later model/user edit must not enable blind replay.
        std::fs::write(
            recovery.join("completed.json"),
            serde_json::to_vec(&json!({"outcome":"applied","target_sha":target})).unwrap(),
        )
        .unwrap();
        assert!(reopened.transition_ready().is_err());
        std::fs::remove_file(root.join(MARKER)).unwrap();
        assert!(reopened.transition_ready().is_err());
        assert_eq!(std::fs::read(root.join("a")).unwrap(), b"later user writer");
    }
    #[test]
    fn cancelled_checkout_retains_durable_marker_without_misreporting_completion() {
        let (_temp, mut local, original, target) = setup();
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = cancel.clone();
        local.transition_hook = Some(Arc::new(move |phase, _| {
            if phase == "before_files" {
                signal.store(true, Ordering::Release)
            }
        }));
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let plan = local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &c,
            )
            .unwrap();
        let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
        assert_eq!(
            local.execute(plan, "scope", &c).unwrap_err(),
            DomainError::DurabilityUnknown
        );
        assert_eq!(run(local.root(), &["rev-parse", "HEAD"]), original);
        assert_eq!(
            std::fs::read(local.root().join("a")).unwrap(),
            b"original a"
        );
        assert!(recovery.join("manifest.json").exists());
        assert!(!recovery.join("completed.json").exists());
    }

    #[test]
    fn transition_crash_worker() {
        let Ok(root) = std::env::var("AG1_OWNED_TRANSITION_CRASH_ROOT") else {
            return;
        };
        let state = std::env::var("AG1_OWNED_TRANSITION_CRASH_STATE").unwrap();
        let target = std::env::var("AG1_OWNED_TRANSITION_CRASH_TARGET").unwrap();
        let phase = std::env::var("AG1_OWNED_TRANSITION_CRASH_PHASE").unwrap();
        let helper = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ag1-process-host");
        let mut local = LocalGit::new(
            WorkspaceService::new()
                .unwrap()
                .register(Path::new(&root))
                .unwrap(),
            &helper,
            Path::new("/usr/bin/git"),
            Path::new("/usr/bin/bwrap"),
            Path::new(&state),
        )
        .unwrap();
        local.transition_hook = Some(Arc::new(move |current, _| {
            if current == phase {
                std::process::exit(74);
            }
        }));
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let choice = std::env::var("AG1_OWNED_TRANSITION_REPAIR_CHOICE").ok();
        let (action, args) = if let Some(choice) = choice {
            let status = local
                .recovery_status(&json!({"repo":"owned/repo"}), &c)
                .unwrap();
            (
                "recover",
                json!({"repo":"owned/repo","recovery_id":status["recovery_id"],"choice":choice}),
            )
        } else {
            (
                "checkout",
                json!({"repo":"owned/repo","branch":"target","target_sha":target}),
            )
        };
        let plan = local
            .prepare(action, &args, "owned-crash-scope", &c)
            .unwrap();
        local.execute(plan, "owned-crash-scope", &c).unwrap();
        panic!("owned crash boundary was not reached");
    }

    #[test]
    fn transition_rechecks_after_private_retention_before_any_workspace_effect() {
        let (_temp, mut local, original, target) = setup();
        let root = local.root().to_path_buf();
        let observed = root.clone();
        local.transition_hook = Some(Arc::new(move |phase, _| {
            if phase == "after_custody" {
                std::fs::write(observed.join("a"), b"changed while retaining private bytes")
                    .unwrap();
            }
        }));
        let cancel = AtomicBool::new(false);
        let c = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let plan = local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &c,
            )
            .unwrap();
        let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
        let index = std::fs::read(root.join(".git/index")).unwrap();
        assert_eq!(
            local.execute(plan, "scope", &c).unwrap_err(),
            DomainError::DurabilityUnknown
        );
        assert_eq!(
            std::fs::read(root.join("a")).unwrap(),
            b"changed while retaining private bytes"
        );
        assert_eq!(std::fs::read(root.join("b")).unwrap(), b"original b");
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(run(&root, &["rev-parse", "HEAD"]), original);
        assert!(!recovery.exists());
        assert!(!root.join(MARKER).exists());
        assert!(local.transition_ready().is_err());
    }

    #[test]
    fn native_transition_crashes_reopen_private_custody_without_trusting_workspace_completion() {
        for phase in [
            "after_custody",
            "after_displace",
            "after_index",
            "after_refs",
            "after_workspace_complete",
        ] {
            let (_temp, local, original, target) = setup();
            let index = std::fs::read(local.root().join(".git/index")).unwrap();
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "github::local::transition::tests::transition_crash_worker",
                    "--nocapture",
                ])
                .env("AG1_OWNED_TRANSITION_CRASH_ROOT", local.root())
                .env("AG1_OWNED_TRANSITION_CRASH_STATE", &local.state)
                .env("AG1_OWNED_TRANSITION_CRASH_TARGET", &target)
                .env("AG1_OWNED_TRANSITION_CRASH_PHASE", phase)
                .output()
                .unwrap();
            assert_eq!(
                result.status.code(),
                Some(74),
                "{phase}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let bucket = std::fs::read_dir(&local.state)
                .unwrap()
                .map(|v| v.unwrap().path())
                .find(|p| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("transition-")
                })
                .unwrap();
            let active: Value =
                serde_json::from_slice(&std::fs::read(bucket.join("active.json")).unwrap())
                    .unwrap();
            let id = active["id"].as_str().unwrap();
            let private = bucket.join(id);
            let recovery = local.root().join(id);
            let record: Value =
                serde_json::from_slice(&std::fs::read(private.join("record.json")).unwrap())
                    .unwrap();
            assert_eq!(record["scope"], "owned-crash-scope");
            assert_eq!(record["root"]["path"], json!(local.root()));
            assert_eq!(record["target_sha"], target);
            assert_eq!(record["from_sha"], original);
            assert_eq!(std::fs::read(private.join("index-before")).unwrap(), index);
            assert_eq!(
                std::fs::read(private.join("before-0")).unwrap(),
                b"original a"
            );
            assert_eq!(std::fs::read(private.join("after-0")).unwrap(), b"target a");
            let candidate = std::fs::read(private.join("index-after")).unwrap();
            assert_eq!(
                record["index_after_sha256"],
                format!("{:x}", Sha256::digest(&candidate))
            );
            assert!(!private.join("completed.json").exists());
            assert_eq!(record["locks"].as_array().unwrap().len(), 3);
            let child = record["child"]["pid"].as_u64().unwrap() as u32;
            let until = Instant::now() + Duration::from_secs(3);
            while crate::process::observe(child).is_ok() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                crate::process::observe(child).is_err(),
                "owned child was reaped/stopped after {phase}"
            );
            if phase == "after_custody" {
                assert!(!recovery.exists());
                assert_eq!(
                    std::fs::read(local.root().join("a")).unwrap(),
                    b"original a"
                );
            }
            if ["after_index", "after_refs", "after_workspace_complete"].contains(&phase) {
                assert_eq!(
                    std::fs::read(local.root().join(".git/index")).unwrap(),
                    candidate
                );
                assert!(private.join("index-published.json").exists());
            }
            assert_eq!(
                run(local.root(), &["rev-parse", "HEAD"]),
                if ["after_refs", "after_workspace_complete"].contains(&phase) {
                    target.as_str()
                } else {
                    original.as_str()
                }
            );
            let helper = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("ag1-process-host");
            let reopened = LocalGit::new(
                WorkspaceService::new()
                    .unwrap()
                    .register(local.root())
                    .unwrap(),
                &helper,
                Path::new("/usr/bin/git"),
                Path::new("/usr/bin/bwrap"),
                &local.state,
            )
            .unwrap();
            assert!(
                reopened.transition_ready().is_err(),
                "pending after {phase}"
            );
            if recovery.exists() {
                std::fs::write(
                    recovery.join("completed.json"),
                    serde_json::to_vec(&json!({"outcome":"applied","target_sha":target})).unwrap(),
                )
                .unwrap();
            }
            if local.root().join(MARKER).exists() {
                std::fs::remove_file(local.root().join(MARKER)).unwrap();
            }
            assert!(
                reopened.transition_ready().is_err(),
                "workspace forgery cannot settle {phase}"
            );
            let cancel = AtomicBool::new(false);
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(10),
            };
            assert!(
                reopened
                    .prepare(
                        "stage",
                        &json!({"repo":"owned/repo","paths":["a"]}),
                        "other-scope",
                        &c
                    )
                    .is_err()
            );
            assert!(
                reopened
                    .prepare("status", &json!({"repo":"owned/repo"}), "other-scope", &c)
                    .is_ok()
            );
            // Recovery is a newly approved native transition. Exact stale
            // locks from the stopped child are retained under its new folder.
            let status = reopened
                .recovery_status(&json!({"repo":"owned/repo"}), &c)
                .unwrap();
            let repair=reopened.prepare("recover",&json!({"repo":"owned/repo","recovery_id":status["recovery_id"],"choice":"finish"}),"reopened-repair",&c).unwrap_or_else(|e|panic!("repair preview after {phase}: {e:?}"));
            let repaired = reopened
                .execute(repair, "reopened-repair", &c)
                .unwrap_or_else(|e| panic!("repair execute after {phase}: {e:?}"));
            assert_eq!(repaired["outcome"], "applied");
            assert_eq!(run(local.root(), &["rev-parse", "HEAD"]), target);
            assert!(reopened.transition_ready().is_ok());
            assert_eq!(std::fs::read(local.root().join("a")).unwrap(), b"target a");
            assert_eq!(std::fs::read(local.root().join("b")).unwrap(), b"target b");
        }
    }

    #[test]
    fn recovery_process_crashes_reopen_again_with_fresh_choice_and_retained_originals() {
        for choice in ["finish", "restore"] {
            for phase in [
                "after_recovery_intent",
                "after_recovery_locks",
                "after_custody",
                "after_displace",
                "after_index",
                "after_refs",
                "after_workspace_complete",
            ] {
                let (_temp, local, original, target) = setup();
                let crash = |phase: &str, choice: Option<&str>| {
                    let mut worker = Command::new(std::env::current_exe().unwrap());
                    worker
                        .args([
                            "--exact",
                            "github::local::transition::tests::transition_crash_worker",
                            "--nocapture",
                        ])
                        .env("AG1_OWNED_TRANSITION_CRASH_ROOT", local.root())
                        .env("AG1_OWNED_TRANSITION_CRASH_STATE", &local.state)
                        .env("AG1_OWNED_TRANSITION_CRASH_TARGET", &target)
                        .env("AG1_OWNED_TRANSITION_CRASH_PHASE", phase);
                    if let Some(choice) = choice {
                        worker.env("AG1_OWNED_TRANSITION_REPAIR_CHOICE", choice);
                    }
                    let result = worker.output().unwrap();
                    assert_eq!(
                        result.status.code(),
                        Some(74),
                        "{choice:?}/{phase}: {} {}",
                        String::from_utf8_lossy(&result.stdout),
                        String::from_utf8_lossy(&result.stderr)
                    );
                    let record = journal::load(&local).unwrap().unwrap();
                    let child = record.value["child"]["pid"].as_u64().unwrap() as u32;
                    let until = Instant::now() + Duration::from_secs(3);
                    while crate::process::observe(child).is_ok() && Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    assert!(crate::process::observe(child).is_err());
                    record
                };
                // Finish starts before files; restore starts after their publication.
                let first = crash(
                    if choice == "finish" {
                        "after_custody"
                    } else {
                        "after_index"
                    },
                    None,
                );
                let next = crash(phase, Some(choice));
                assert!(!next.complete);
                let was_activated =
                    !["after_recovery_intent", "after_recovery_locks"].contains(&phase);
                if was_activated {
                    assert_eq!(next.value["recovery_of"]["id"], first.value["id"]);
                } else {
                    assert_eq!(next.value["id"], first.value["id"]);
                }
                let cancel = AtomicBool::new(false);
                let c = ScanControl {
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(30),
                };
                let args = json!({"repo":"owned/repo","recovery_id":next.value["id"],"choice":if was_activated {"finish"}else{choice}});
                let plan = local
                    .prepare("recover", &args, "fresh-after-second-crash", &c)
                    .unwrap_or_else(|e| panic!("{choice}/{phase} prepare: {e:?}"));
                assert_eq!(
                    local
                        .execute(plan, "fresh-after-second-crash", &c)
                        .unwrap_or_else(|e| panic!("{choice}/{phase} execute: {e:?}"))["outcome"],
                    "applied"
                );
                assert_eq!(
                    run(local.root(), &["rev-parse", "HEAD"]),
                    if choice == "finish" {
                        &target
                    } else {
                        &original
                    }
                    .as_str()
                );
                assert_eq!(
                    std::fs::read(local.root().join("a")).unwrap(),
                    if choice == "finish" {
                        b"target a".as_slice()
                    } else {
                        b"original a".as_slice()
                    }
                );
                assert_eq!(
                    first
                        .store
                        .snapshot(&format!("{}/before-0", first.directory), 1024)
                        .unwrap()
                        .bytes,
                    b"original a"
                );
                assert!(local.transition_ready().is_ok());
            }
        }
    }

    #[test]
    fn recovery_lock_handoff_preserves_replacements_and_honors_cancellation_and_revocation() {
        for kind in ["lock", "file", "cancel", "revoke"] {
            let (_temp, mut local, original, target) = setup();
            let result = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "github::local::transition::tests::transition_crash_worker",
                    "--nocapture",
                ])
                .env("AG1_OWNED_TRANSITION_CRASH_ROOT", local.root())
                .env("AG1_OWNED_TRANSITION_CRASH_STATE", &local.state)
                .env("AG1_OWNED_TRANSITION_CRASH_TARGET", &target)
                .env("AG1_OWNED_TRANSITION_CRASH_PHASE", "after_custody")
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(74));
            let record = journal::load(&local).unwrap().unwrap();
            let child = record.value["child"]["pid"].as_u64().unwrap() as u32;
            let until = Instant::now() + Duration::from_secs(3);
            while crate::process::observe(child).is_ok() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(crate::process::observe(child).is_err());
            let root = local.root().to_path_buf();
            let index = std::fs::read(root.join(".git/index")).unwrap();
            let access = crate::folder_access::FolderAccess::open(
                local.workspace.clone(),
                &local.state,
                "owned-recovery-race",
            )
            .unwrap();
            let revision = access.snapshot().unwrap().selection.revision;
            local.grant = Some((access.clone(), revision, true));
            let cancel = Arc::new(AtomicBool::new(false));
            let c = ScanControl {
                cancel: &cancel,
                deadline: Instant::now() + Duration::from_secs(30),
            };
            let args =
                json!({"repo":"owned/repo","recovery_id":record.value["id"],"choice":"finish"});
            let plan = local.prepare("recover", &args, "repair", &c).unwrap();
            let observed = root.clone();
            let signal = cancel.clone();
            let once = AtomicBool::new(false);
            let changed = Arc::new(std::sync::Mutex::new(String::new()));
            let capture = changed.clone();
            local.transition_hook = Some(Arc::new(move |phase, path| {
                if phase == "before_recovery_lock" && !once.swap(true, Ordering::AcqRel) {
                    *capture.lock().unwrap() = path.into();
                    match kind {
                        "lock" => {
                            std::fs::rename(
                                observed.join(path),
                                observed.join("retained-stale-lock"),
                            )
                            .unwrap();
                            std::fs::write(observed.join(path), b"later foreign lock").unwrap();
                        }
                        "file" => std::fs::write(observed.join("a"), b"later user writer").unwrap(),
                        "cancel" => signal.store(true, Ordering::Release),
                        "revoke" => {
                            access
                                .change(crate::folder_access::Change {
                                    revision,
                                    action: "mode".into(),
                                    path: observed.clone(),
                                    mode: Some(crate::folder_access::AccessMode::ReadOnly),
                                })
                                .unwrap();
                        }
                        _ => unreachable!(),
                    }
                }
            }));
            assert_eq!(
                local.execute(plan, "repair", &c).unwrap_err(),
                DomainError::DurabilityUnknown,
                "{kind}"
            );
            let changed = changed.lock().unwrap().clone();
            assert!(!changed.is_empty());
            if kind == "lock" {
                assert_eq!(
                    std::fs::read(root.join(changed)).unwrap(),
                    b"later foreign lock"
                );
            }
            assert_eq!(
                std::fs::read(root.join("a")).unwrap(),
                if kind == "file" {
                    b"later user writer".as_slice()
                } else {
                    b"original a".as_slice()
                }
            );
            assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
            assert_eq!(run(&root, &["rev-parse", "HEAD"]), original);
            assert!(!journal::load(&local).unwrap().unwrap().complete);
        }
    }
}
