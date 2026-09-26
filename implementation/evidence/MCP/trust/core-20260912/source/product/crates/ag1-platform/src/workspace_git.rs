//! Exact Git working-entry custody for checkout/pull. This is not a branch
//! transaction by itself: the caller must durably record the recovery names,
//! retain the Git index/ref preimages and settle the whole operation honestly.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitEntry {
    pub bytes: Vec<u8>,
    /// Git tree mode, not an arbitrary Unix permission mask.
    pub mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitSnapshot {
    pub entry: GitEntry,
    pub revision: FileRevision,
}

impl Workspace {
    /// Retain the exact owned postimage across chmod, including its new ctime.
    /// A rediscovered entry is validation, never a new grant of ownership.
    pub(crate) fn set_git_mode_checked(
        &self,
        name: &str,
        expected: &GitSnapshot,
        mode: u32,
    ) -> Result<GitSnapshot, DomainError> {
        if mode & !0o777 != 0 || expected.entry.mode == 0o120000 {
            return Err(DomainError::PermissionDenied);
        }
        let _claim = self.claim()?;
        let file = beneath(
            &self.directory,
            relative(name)?,
            OFlags::RDONLY,
            Mode::empty(),
        )?;
        if revision(
            &file.metadata().map_err(io_error)?,
            &expected.revision.sha256,
        ) != expected.revision
        {
            return Err(DomainError::Conflict);
        }
        rfs::fchmod(&file, Mode::from_raw_mode(mode)).map_err(errno)?;
        file.sync_all()
            .map_err(|_| DomainError::DurabilityUnknown)?;
        let owned = revision(
            &file.metadata().map_err(io_error)?,
            &expected.revision.sha256,
        );
        let current = self
            .git_snapshot(name)?
            .ok_or(DomainError::DurabilityUnknown)?;
        if current.revision != owned
            || current.entry != expected.entry
            || owned.modified_ns != expected.revision.modified_ns
            || owned.mode != mode
        {
            return Err(DomainError::DurabilityUnknown);
        }
        Ok(current)
    }
    /// Capture the entry itself. Link targets may be arbitrary Git bytes, but
    /// neither those targets nor symlinked ancestors are ever followed.
    pub(crate) fn git_snapshot(&self, name: &str) -> Result<Option<GitSnapshot>, DomainError> {
        self.verify_root()?;
        let path = relative(name)?;
        let file = match beneath(&self.directory, path, OFlags::PATH, Mode::empty()) {
            Ok(file) => file,
            Err(DomainError::NotFound) => return Ok(None),
            Err(error) => return Err(error),
        };
        let before = file.metadata().map_err(io_error)?;
        if before.is_file() {
            let snapshot = self.snapshot(name, MAX_READ)?;
            if (snapshot.revision.device, snapshot.revision.inode) != (before.dev(), before.ino()) {
                return Err(DomainError::Conflict);
            }
            let mode = if snapshot.revision.mode & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            };
            return Ok(Some(GitSnapshot {
                entry: GitEntry {
                    bytes: snapshot.bytes,
                    mode,
                },
                revision: snapshot.revision,
            }));
        }
        if !before.file_type().is_symlink() {
            return Err(DomainError::Conflict);
        }
        let bytes = rfs::readlinkat(&file, "", Vec::new())
            .map_err(errno)?
            .to_bytes()
            .to_vec();
        if bytes.len() > 4096 {
            return Err(DomainError::LimitExceeded("Git link bytes".into()));
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let after = beneath(&self.directory, path, OFlags::PATH, Mode::empty())?
            .metadata()
            .map_err(io_error)?;
        if revision(&before, &hash) != revision(&after, &hash) {
            return Err(DomainError::Conflict);
        }
        self.verify_root()?;
        Ok(Some(GitSnapshot {
            entry: GitEntry {
                bytes,
                mode: 0o120000,
            },
            revision: revision(&before, &hash),
        }))
    }

    /// Publish one admitted entry, retaining the exact displaced inode at a
    /// native-generated sibling name or root recovery directory. Once publication
    /// occurs, errors are uncertain unless the displaced concurrent entry was
    /// verifiably restored into an empty destination. No broad rollback occurs.
    /// The caller must journal `recovery` durably BEFORE entering this method.
    pub(crate) fn publish_git_entry_checked(
        &self,
        name: &str,
        expected: Option<&GitSnapshot>,
        replacement: Option<&GitEntry>,
        recovery: &str,
        check: impl Fn(UndoStage) -> Result<(), DomainError>,
    ) -> Result<Option<GitSnapshot>, DomainError> {
        self.publish_git_entry_to_checked(name, expected, replacement, self, recovery, check)
    }
    /// Same native custody protocol with a separately authorized backup root.
    /// Both roots must share a filesystem; nothing is copied across devices.
    pub(crate) fn publish_git_entry_to_checked(
        &self,
        name: &str,
        expected: Option<&GitSnapshot>,
        replacement: Option<&GitEntry>,
        backup_workspace: &Workspace,
        recovery: &str,
        check: impl Fn(UndoStage) -> Result<(), DomainError>,
    ) -> Result<Option<GitSnapshot>, DomainError> {
        check(UndoStage::Prepare)?;
        let path = relative(name)?;
        let backup = relative(recovery)?;
        let sibling = path.parent() == backup.parent()
            && backup
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".ag1-git-recovery-"));
        let central = backup.components().count() == 2
            && backup.components().next().is_some_and(|n| {
                n.as_os_str()
                    .to_string_lossy()
                    .starts_with(".ag1-git-recovery-")
            });
        let same_root = self.directory_identity() == backup_workspace.directory_identity();
        if (same_root && name == recovery) || !(central || (same_root && sibling)) || self.directory_identity().0 != backup_workspace.directory_identity().0 {
            return Err(DomainError::PermissionDenied);
        }
        if let Some(next) = replacement {
            if ![0o100644, 0o100755, 0o120000].contains(&next.mode)
                || next.bytes.len() > MAX_READ
                || next.mode == 0o120000
                    && (next.bytes.is_empty() || next.bytes.len() > 4096 || next.bytes.contains(&0))
            {
                return Err(DomainError::InvalidInput(
                    "Bounded regular Git file or symbolic link required".into(),
                ));
            }
        }
        let _claim = self.claim()?;
        let _backup_claim = if same_root { None } else { Some(backup_workspace.claim()?) };
        if self.git_snapshot(name)?.as_ref() != expected || backup_workspace.exists(recovery)? {
            return Err(DomainError::Conflict);
        }
        let parent = self.pin_directory(path.parent().and_then(Path::to_str).unwrap_or(""))?;
        let recovery_parent =
            backup_workspace.pin_directory(backup.parent().and_then(Path::to_str).unwrap_or(""))?;
        let staging = format!(
            ".ag1-git-stage-{}-{}",
            std::process::id(),
            TEMP_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let stage_path = path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&staging)
            .to_string_lossy()
            .into_owned();
        let stage = if let Some(next) = replacement {
            if next.mode == 0o120000 {
                rfs::symlinkat(
                    std::ffi::OsStr::from_bytes(&next.bytes),
                    &parent.directory,
                    staging.as_str(),
                )
                .map_err(errno)?;
                // symlinkat does not return an owned descriptor. A subsequently
                // opened path is sufficient for validation, but is not custody
                // to delete a possible concurrent replacement on cleanup.
                Some((
                    beneath(
                        &parent.directory,
                        Path::new(&staging),
                        OFlags::PATH,
                        Mode::empty(),
                    )?,
                    false,
                ))
            } else {
                let mut file = beneath(
                    &parent.directory,
                    Path::new(&staging),
                    OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                    Mode::RUSR | Mode::WUSR,
                )?;
                // On a preparation failure leave the named staging file in place;
                // never delete a path whose identity has not been captured.
                file.write_all(&next.bytes).map_err(io_error)?;
                rfs::fchmod(
                    &file,
                    // Newly created files are owner-only. Replacements retain
                    // prior read/write access, without granting new readers.
                    Mode::from_raw_mode({
                        let access = expected
                            .filter(|s| s.entry.mode != 0o120000)
                            .map(|s| s.revision.mode & 0o666)
                            .unwrap_or(0o600);
                        access
                            | if next.mode == 0o100755 {
                                (access & 0o444) >> 2
                            } else {
                                0
                            }
                    }),
                )
                .map_err(errno)?;
                file.sync_all().map_err(io_error)?;
                // Retain the descriptor returned by exclusive creation, not a
                // reopened path which could belong to a later writer.
                Some((file, true))
            }
        } else {
            None
        };
        let stage_current = || -> Result<(), DomainError> {
            if let Some((file, _)) = &stage {
                let identity = file.metadata().map_err(io_error)?;
                let current = beneath(
                    &parent.directory,
                    Path::new(&staging),
                    OFlags::PATH,
                    Mode::empty(),
                )?
                .metadata()
                .map_err(io_error)?;
                if identity.dev() != current.dev()
                    || identity.ino() != current.ino()
                    || self.git_snapshot(&stage_path)?.as_ref().map(|s| &s.entry) != replacement
                {
                    return Err(DomainError::Conflict);
                }
            }
            Ok(())
        };
        let result = (|| {
            check(UndoStage::BeforeMove)?;
            parent.verify_identity()?;
            recovery_parent.verify_identity()?;
            stage_current()?;
            if self.git_snapshot(name)?.as_ref() != expected {
                return Err(DomainError::Conflict);
            }
            let mut displaced_revision = None;
            if let Some(previous) = expected {
                rfs::renameat_with(
                    &parent.directory,
                    path.file_name().unwrap(),
                    &recovery_parent.directory,
                    backup.file_name().unwrap(),
                    rfs::RenameFlags::NOREPLACE,
                )
                .map_err(errno)?;
                let moved = (|| {
                    parent.directory.sync_all().map_err(io_error)?;
                    recovery_parent.directory.sync_all().map_err(io_error)?;
                    check(UndoStage::AfterMove)?;
                    parent.verify_identity()?;
                    recovery_parent.verify_identity()?;
                    backup_workspace.git_snapshot(recovery)?.ok_or(DomainError::Conflict)
                })()
                .map_err(|_| DomainError::DurabilityUnknown)?;
                let mut comparable = moved.clone();
                comparable.revision.changed_ns = previous.revision.changed_ns;
                if comparable != *previous {
                    // The rename raced with a writer. Restore only this exact
                    // displaced entry, only if the intended path remains absent.
                    if parent.verify_identity().is_ok()
                        && recovery_parent.verify_identity().is_ok()
                        && backup_workspace
                            .git_snapshot(recovery)
                            .map_err(|_| DomainError::DurabilityUnknown)?
                            .as_ref()
                            == Some(&moved)
                        && rfs::renameat_with(
                            &recovery_parent.directory,
                            backup.file_name().unwrap(),
                            &parent.directory,
                            path.file_name().unwrap(),
                            rfs::RenameFlags::NOREPLACE,
                        )
                        .is_ok()
                        && parent.directory.sync_all().is_ok()
                        && recovery_parent.directory.sync_all().is_ok()
                    {
                        return Err(DomainError::Conflict);
                    }
                    return Err(DomainError::DurabilityUnknown);
                }
                displaced_revision = Some(moved);
            }
            let publish = (|| {
                check(UndoStage::BeforePublish)?;
                parent.verify_identity()?;
                recovery_parent.verify_identity()?;
                if let Some(moved) = displaced_revision.as_ref() {
                    if backup_workspace.git_snapshot(recovery)?.as_ref() != Some(moved) {
                        return Err(DomainError::Conflict);
                    }
                }
                if stage.is_some() {
                    stage_current()?;
                    rfs::renameat_with(
                        &parent.directory,
                        staging.as_str(),
                        &parent.directory,
                        path.file_name().unwrap(),
                        rfs::RenameFlags::NOREPLACE,
                    )
                    .map_err(errno)?;
                } else if self.exists(name)? {
                    return Err(DomainError::Conflict);
                }
                parent.directory.sync_all().map_err(io_error)?;
                check(UndoStage::AfterPublish)?;
                parent.verify_identity()?;
                let result = self.git_snapshot(name)?;
                if result.as_ref().map(|s| &s.entry) != replacement {
                    return Err(DomainError::Conflict);
                }
                Ok(result)
            })();
            // A new entry might already have been published even without a
            // previous entry. The conservative terminal classification avoids
            // replay across either boundary.
            publish.map_err(|_| DomainError::DurabilityUnknown)
        })();
        if let Some((file, true)) = stage
            && let Ok(ours) = file.metadata()
        {
            if beneath(
                &parent.directory,
                Path::new(&staging),
                OFlags::PATH,
                Mode::empty(),
            )
            .and_then(|f| f.metadata().map_err(io_error))
            .is_ok_and(|m| m.dev() == ours.dev() && m.ino() == ours.ino())
            {
                let _ = rfs::unlinkat(&parent.directory, staging.as_str(), rfs::AtFlags::empty());
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    fn setup() -> (tempfile::TempDir, Arc<Workspace>) {
        let dir = tempfile::tempdir().unwrap();
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        (dir, workspace)
    }
    #[test]
    fn git_metadata_publication_retains_exact_inode_in_separately_bound_worktree_root() {
        let temp=tempfile::tempdir().unwrap();
        for name in ["metadata","worktree"] { std::fs::create_dir(temp.path().join(name)).unwrap(); }
        let service=WorkspaceService::new().unwrap();let metadata=service.register(&temp.path().join("metadata")).unwrap();let worktree=service.register(&temp.path().join("worktree")).unwrap();
        std::fs::create_dir(worktree.root().join(".ag1-git-recovery-owned")).unwrap();
        std::fs::write(metadata.root().join("index"),b"original index").unwrap();let before=metadata.git_snapshot("index").unwrap().unwrap();
        let after=GitEntry{bytes:b"reviewed index".to_vec(),mode:0o100644};
        metadata.publish_git_entry_to_checked("index",Some(&before),Some(&after),&worktree,".ag1-git-recovery-owned/index",|_|Ok(())).unwrap();
        let retained=worktree.git_snapshot(".ag1-git-recovery-owned/index").unwrap().unwrap();assert_eq!(retained.entry,before.entry);assert_eq!(retained.revision.inode,before.revision.inode);assert_eq!(metadata.git_snapshot("index").unwrap().unwrap().entry,after);
        let current=metadata.git_snapshot("index").unwrap().unwrap();
        assert!(metadata.publish_git_entry_to_checked("index",Some(&current),None,&worktree,".ag1-git-recovery-owned/index",|_|Ok(())).is_err());
        assert_eq!(metadata.git_snapshot("index").unwrap().unwrap(),current);
        let result=metadata.publish_git_entry_to_checked("index",Some(&current),None,&worktree,".ag1-git-recovery-owned/later",|stage| {if stage==UndoStage::BeforeMove {std::fs::write(metadata.root().join("index"),b"later writer").unwrap();}Ok(())});
        assert_eq!(result,Err(DomainError::Conflict));assert_eq!(std::fs::read(metadata.root().join("index")).unwrap(),b"later writer");assert!(!worktree.root().join(".ag1-git-recovery-owned/later").exists());
    }
    #[test]
    fn checked_git_chmod_retains_owned_postimage_and_rejects_later_writers() {
        let (dir, w) = setup();
        std::fs::write(dir.path().join("file"), b"owned bytes\0\xff").unwrap();
        std::fs::set_permissions(
            dir.path().join("file"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        let original = w.git_snapshot("file").unwrap().unwrap();
        let after = w.set_git_mode_checked("file", &original, 0o750).unwrap();
        assert_eq!(after.revision.mode, 0o750);
        assert_eq!(after.revision.inode, original.revision.inode);
        assert_eq!(after.entry, original.entry);
        assert_eq!(Some(after.clone()), w.git_snapshot("file").unwrap());
        assert!(w.set_git_mode_checked("file", &original, 0o755).is_err());
        std::fs::write(dir.path().join("file"), b"later user bytes").unwrap();
        assert!(w.set_git_mode_checked("file", &after, 0o755).is_err());
        assert_eq!(
            std::fs::read(dir.path().join("file")).unwrap(),
            b"later user bytes"
        );
        symlink("file", dir.path().join("link")).unwrap();
        let link = w.git_snapshot("link").unwrap().unwrap();
        assert!(w.set_git_mode_checked("link", &link, 0o777).is_err());
        assert_eq!(
            std::fs::metadata(dir.path().join("file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o750
        );
    }
    #[test]
    fn git_entry_publication_preserves_displaced_binary_modes_and_links() {
        let (dir, w) = setup();
        std::fs::write(dir.path().join("file"), b"old\0\xff\r\n").unwrap();
        std::fs::set_permissions(
            dir.path().join("file"),
            std::fs::Permissions::from_mode(0o751),
        )
        .unwrap();
        let before = w.git_snapshot("file").unwrap().unwrap();
        let next = GitEntry {
            bytes: b"new\r\n\0\xff".to_vec(),
            mode: 0o100755,
        };
        let after = w
            .publish_git_entry_checked(
                "file",
                Some(&before),
                Some(&next),
                ".ag1-git-recovery-one",
                |_| Ok(()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(after.entry, next);
        assert_eq!(
            w.git_snapshot(".ag1-git-recovery-one")
                .unwrap()
                .unwrap()
                .entry,
            before.entry
        );
        let link = GitEntry {
            bytes: b"/absolute/outside\xff".to_vec(),
            mode: 0o120000,
        };
        let after_link = w
            .publish_git_entry_checked(
                "file",
                Some(&after),
                Some(&link),
                ".ag1-git-recovery-two",
                |_| Ok(()),
            )
            .unwrap()
            .unwrap();
        assert_eq!(after_link.entry, link);
        assert!(
            w.publish_git_entry_checked(
                "file",
                Some(&after_link),
                None,
                ".ag1-git-recovery-three",
                |_| Ok(())
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(
            w.git_snapshot(".ag1-git-recovery-three")
                .unwrap()
                .unwrap()
                .entry,
            link
        );
        let created = w
            .publish_git_entry_checked("file", None, Some(&next), ".ag1-git-recovery-four", |_| {
                Ok(())
            })
            .unwrap()
            .unwrap();
        assert_eq!(created.entry, next);
        assert!(!dir.path().join(".ag1-git-recovery-four").exists());
    }
    #[test]
    fn git_entry_publication_never_overwrites_later_writers_and_retains_recovery() {
        for stage in [
            UndoStage::BeforeMove,
            UndoStage::AfterMove,
            UndoStage::BeforePublish,
            UndoStage::AfterPublish,
        ] {
            let (dir, w) = setup();
            std::fs::write(dir.path().join("file"), b"original").unwrap();
            let before = w.git_snapshot("file").unwrap().unwrap();
            let next = GitEntry {
                bytes: b"candidate".to_vec(),
                mode: 0o100644,
            };
            let result = w.publish_git_entry_checked(
                "file",
                Some(&before),
                Some(&next),
                ".ag1-git-recovery-race",
                |at| {
                    if at == stage {
                        std::fs::write(dir.path().join("file"), b"later writer").unwrap();
                    }
                    Ok(())
                },
            );
            assert!(result.is_err());
            assert_eq!(
                std::fs::read(dir.path().join("file")).unwrap(),
                b"later writer"
            );
            if stage != UndoStage::BeforeMove {
                assert_eq!(
                    std::fs::read(dir.path().join(".ag1-git-recovery-race")).unwrap(),
                    b"original"
                );
            }
        }
    }
    #[test]
    fn git_entry_publication_rejects_symlinked_parents_and_cancelled_authority() {
        let (dir, w) = setup();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), dir.path().join("link")).unwrap();
        let next = GitEntry {
            bytes: b"candidate".to_vec(),
            mode: 0o100644,
        };
        assert!(w.git_snapshot("link/file").is_err());
        assert!(
            w.publish_git_entry_checked(
                "link/file",
                None,
                Some(&next),
                "link/.ag1-git-recovery-x",
                |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(
            w.publish_git_entry_checked("file", None, Some(&next), ".ag1-git-recovery-x", |_| Err(
                DomainError::Cancelled
            ))
            .unwrap_err(),
            DomainError::Cancelled
        );
        assert!(!dir.path().join("file").exists());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
        std::fs::write(dir.path().join("file"), b"original").unwrap();
        let previous = w.git_snapshot("file").unwrap().unwrap();
        assert_eq!(
            w.publish_git_entry_checked(
                "file",
                Some(&previous),
                Some(&next),
                ".ag1-git-recovery-x",
                |at| if at == UndoStage::AfterMove {
                    Err(DomainError::Cancelled)
                } else {
                    Ok(())
                }
            )
            .unwrap_err(),
            DomainError::DurabilityUnknown
        );
        assert!(!dir.path().join("file").exists());
        assert_eq!(
            std::fs::read(dir.path().join(".ag1-git-recovery-x")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn git_entry_central_recovery_keeps_originals_out_of_directory_transitions() {
        let (dir, w) = setup();
        std::fs::create_dir(dir.path().join("folder")).unwrap();
        std::fs::create_dir(dir.path().join(".ag1-git-recovery-batch")).unwrap();
        std::fs::write(dir.path().join("folder/file"), b"original").unwrap();
        let previous = w.git_snapshot("folder/file").unwrap().unwrap();
        w.publish_git_entry_checked(
            "folder/file",
            Some(&previous),
            None,
            ".ag1-git-recovery-batch/0",
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join(".ag1-git-recovery-batch/0")).unwrap(),
            b"original"
        );
        assert!(
            w.remove_empty_directory_checked(w.pin_directory("folder").unwrap(), || Ok(()))
                .unwrap()
        );
        let next = GitEntry {
            bytes: b"directory became a file".to_vec(),
            mode: 0o100644,
        };
        w.publish_git_entry_checked(
            "folder",
            None,
            Some(&next),
            ".ag1-git-recovery-batch/1",
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("folder")).unwrap(),
            next.bytes
        );
    }

    #[test]
    fn git_entry_cleanup_never_deletes_a_replaced_staging_name() {
        for mode in [0o100644, 0o120000] {
            let (dir, w) = setup();
            std::fs::write(dir.path().join("file"), b"original").unwrap();
            let before = w.git_snapshot("file").unwrap().unwrap();
            let next = GitEntry {
                bytes: b"new-target".to_vec(),
                mode,
            };
            let replaced = std::cell::RefCell::new(None);
            assert_eq!(
                w.publish_git_entry_checked(
                    "file",
                    Some(&before),
                    Some(&next),
                    ".ag1-git-recovery-cleanup",
                    |at| {
                        if at == UndoStage::BeforeMove {
                            let name = std::fs::read_dir(dir.path())
                                .unwrap()
                                .map(|e| e.unwrap().path())
                                .find(|p| {
                                    p.file_name()
                                        .unwrap()
                                        .to_string_lossy()
                                        .starts_with(".ag1-git-stage-")
                                })
                                .unwrap();
                            std::fs::rename(&name, dir.path().join("retained-native-stage"))
                                .unwrap();
                            std::fs::write(&name, b"later staging writer").unwrap();
                            *replaced.borrow_mut() = Some(name);
                        }
                        Ok(())
                    }
                )
                .unwrap_err(),
                DomainError::Conflict
            );
            assert_eq!(
                std::fs::read(replaced.into_inner().unwrap()).unwrap(),
                b"later staging writer"
            );
            assert_eq!(std::fs::read(dir.path().join("file")).unwrap(), b"original");
            assert!(!dir.path().join(".ag1-git-recovery-cleanup").exists());
        }
    }

    #[test]
    fn git_entry_abrupt_exit_worker() {
        let Some(root) = std::env::var_os("AG1_OWNED_GIT_ENTRY_CRASH_ROOT") else {
            return;
        };
        let stage = std::env::var("AG1_OWNED_GIT_ENTRY_CRASH_STAGE").unwrap();
        let w = WorkspaceService::new()
            .unwrap()
            .register(Path::new(&root))
            .unwrap();
        let before = w.git_snapshot("file").unwrap().unwrap();
        let next = GitEntry {
            bytes: b"new\0\xff".to_vec(),
            mode: 0o100755,
        };
        let _ = w.publish_git_entry_checked(
            "file",
            Some(&before),
            Some(&next),
            ".ag1-git-recovery-crash",
            |at| {
                if format!("{at:?}") == stage {
                    std::process::exit(86);
                }
                Ok(())
            },
        );
        panic!("owned crash seam was not reached");
    }

    #[test]
    fn git_entry_abrupt_exit_retains_originals_across_reopen_without_replay() {
        for stage in ["AfterMove", "BeforePublish", "AfterPublish"] {
            let (dir, w) = setup();
            symlink("unfollowed-original-target", dir.path().join("file")).unwrap();
            let original = w.git_snapshot("file").unwrap().unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "workspace::git_publication::tests::git_entry_abrupt_exit_worker",
                    "--nocapture",
                ])
                .env("AG1_OWNED_GIT_ENTRY_CRASH_ROOT", dir.path())
                .env("AG1_OWNED_GIT_ENTRY_CRASH_STAGE", stage)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(86));
            let reopened = WorkspaceService::new()
                .unwrap()
                .register(dir.path())
                .unwrap();
            assert_eq!(
                reopened
                    .git_snapshot(".ag1-git-recovery-crash")
                    .unwrap()
                    .unwrap()
                    .entry,
                original.entry
            );
            let now = reopened.git_snapshot("file").unwrap();
            if stage == "AfterPublish" {
                assert_eq!(now.unwrap().entry.bytes, b"new\0\xff");
            } else {
                assert!(now.is_none());
            }
            let next = GitEntry {
                bytes: b"not replayed".to_vec(),
                mode: 0o100644,
            };
            assert_eq!(
                reopened
                    .publish_git_entry_checked(
                        "file",
                        Some(&original),
                        Some(&next),
                        ".ag1-git-recovery-crash",
                        |_| Ok(())
                    )
                    .unwrap_err(),
                DomainError::Conflict
            );
        }
    }
}
