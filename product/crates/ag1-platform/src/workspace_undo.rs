//! Recoverable single-file reversal. Publication never replaces an existing
//! destination: a racing creator keeps its file and the displaced file is kept.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UndoStage {
    Prepare,
    BeforeMove,
    AfterMove,
    BeforePublish,
    AfterPublish,
}

#[derive(Clone, Debug)]
pub struct UndoOutcome {
    pub state: &'static str,
    pub recovery: Option<String>,
    pub revision: Option<FileRevision>,
    pub detail: Option<String>,
}

impl Workspace {
    /// `recovery` is a native-generated sibling path recorded durably before
    /// entry. Originals and racing writes remain recoverable even after a crash.
    pub fn undo_file_checked(
        &self,
        name: &str,
        expected: &FileRevision,
        original: Option<(&[u8], u32)>,
        recovery: &str,
        check: impl Fn(UndoStage) -> Result<(), DomainError>,
    ) -> Result<UndoOutcome, DomainError> {
        check(UndoStage::Prepare)?;
        let path = relative(name)?;
        let backup = relative(recovery)?;
        if path.parent() != backup.parent()
            || name == recovery
            || !backup
                .file_name()
                .is_some_and(|s| s.to_string_lossy().starts_with(".ag1-undo-"))
        {
            return Err(DomainError::InvalidInput(
                "native undo recovery path".into(),
            ));
        }
        if original.is_some_and(|(bytes, mode)| bytes.len() > MAX_READ || mode & !0o777 != 0) {
            return Err(DomainError::PermissionDenied);
        }
        let _claim = self.claim()?;
        if self.snapshot(name, MAX_READ)?.revision != *expected || self.exists(recovery)? {
            return Err(DomainError::Conflict);
        }
        let parent = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
            Some(p) => beneath(
                &self.directory,
                p,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
            )?,
            None => self.directory.try_clone().map_err(io_error)?,
        };
        let parent_current = || -> Result<(), DomainError> {
            self.verify_root()?;
            let current = match path.parent().filter(|p| !p.as_os_str().is_empty()) {
                Some(p) => beneath(
                    &self.directory,
                    p,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                )?,
                None => self.directory.try_clone().map_err(io_error)?,
            };
            let a = parent.metadata().map_err(io_error)?;
            let b = current.metadata().map_err(io_error)?;
            if a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(DomainError::Conflict);
            }
            Ok(())
        };
        // Prepare and sync replacement bytes before moving the current file.
        let staging = format!(
            ".ag1-undo-stage-{}-{}",
            std::process::id(),
            TEMP_SERIAL.fetch_add(1, Ordering::Relaxed)
        );
        let staged = original
            .map(|(bytes, mode)| -> Result<File, DomainError> {
                let mut file = beneath(
                    &parent,
                    Path::new(&staging),
                    OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL,
                    Mode::RUSR | Mode::WUSR,
                )?;
                file.write_all(bytes).map_err(io_error)?;
                rfs::fchmod(&file, Mode::from_raw_mode(mode)).map_err(errno)?;
                file.sync_all().map_err(io_error)?;
                Ok(file)
            })
            .transpose()?;
        let result = (|| {
            check(UndoStage::BeforeMove)?;
            parent_current()?;
            if self.snapshot(name, MAX_READ)?.revision != *expected {
                return Err(DomainError::Conflict);
            }
            rfs::renameat_with(
                &parent,
                path.file_name().unwrap(),
                &parent,
                backup.file_name().unwrap(),
                rfs::RenameFlags::NOREPLACE,
            )
            .map_err(errno)?;
            // After this boundary, never report a retryable pre-effect failure.
            let uncertain = |error: DomainError| UndoOutcome {
                state: "recovery_required",
                recovery: Some(recovery.into()),
                revision: None,
                detail: Some(format!("{error:?}")),
            };
            if parent.sync_all().is_err() {
                return Ok(uncertain(DomainError::DurabilityUnknown));
            }
            if let Err(error) = check(UndoStage::AfterMove).and_then(|_| parent_current()) {
                return Ok(uncertain(error));
            }
            let displaced = match self.snapshot(recovery, MAX_READ) {
                Ok(v) => v,
                Err(error) => return Ok(uncertain(error)),
            };
            // rename changes ctime; all other postimage identity is checked.
            let mut moved = displaced.revision.clone();
            moved.changed_ns = expected.changed_ns;
            if moved != *expected {
                // Restore only into an empty destination. Never overwrite a
                // second racing file; both versions then remain available.
                let restored = rfs::renameat_with(
                    &parent,
                    backup.file_name().unwrap(),
                    &parent,
                    path.file_name().unwrap(),
                    rfs::RenameFlags::NOREPLACE,
                )
                .is_ok();
                if restored && parent.sync_all().is_ok() {
                    return Ok(UndoOutcome {
                        state: "not_applied",
                        recovery: None,
                        revision: None,
                        detail: Some(
                            "A concurrent edit was preserved. Undo was not applied.".into(),
                        ),
                    });
                }
                return Ok(uncertain(DomainError::Conflict));
            }
            if let Err(error) = check(UndoStage::BeforePublish).and_then(|_| parent_current()) {
                return Ok(uncertain(error));
            }
            if !self
                .snapshot(recovery, MAX_READ)
                .is_ok_and(|s| s.revision == displaced.revision)
            {
                return Ok(uncertain(DomainError::Conflict));
            }
            if let Some(file) = &staged {
                let staged_metadata = match file.metadata() {
                    Ok(v) => v,
                    Err(error) => return Ok(uncertain(io_error(error))),
                };
                let current_stage =
                    beneath(&parent, Path::new(&staging), OFlags::RDONLY, Mode::empty())
                        .and_then(|f| f.metadata().map_err(io_error));
                if !current_stage.is_ok_and(|m| {
                    m.dev() == staged_metadata.dev() && m.ino() == staged_metadata.ino()
                }) {
                    return Ok(uncertain(DomainError::Conflict));
                }
                if let Err(error) = rfs::renameat_with(
                    &parent,
                    &staging,
                    &parent,
                    path.file_name().unwrap(),
                    rfs::RenameFlags::NOREPLACE,
                ) {
                    return Ok(uncertain(errno(error)));
                }
                if parent.sync_all().is_err() {
                    return Ok(uncertain(DomainError::DurabilityUnknown));
                }
            } else if self.exists(name).unwrap_or(true) {
                return Ok(uncertain(DomainError::Conflict));
            }
            if let Err(error) = check(UndoStage::AfterPublish).and_then(|_| parent_current()) {
                return Ok(uncertain(error));
            }
            let revision = if let Some((bytes, _)) = original {
                match self.snapshot(name, MAX_READ) {
                    Ok(s) if s.bytes == bytes => Some(s.revision),
                    _ => return Ok(uncertain(DomainError::DurabilityUnknown)),
                }
            } else {
                None
            };
            Ok(UndoOutcome {
                state: "completed",
                recovery: Some(recovery.into()),
                revision,
                detail: None,
            })
        })();
        // Remove only our unpublished staging inode. The recovery copy is never
        // automatically deleted, including on cancellation or uncertain results.
        if let Some(file) = staged
            && let Ok(identity) = file.metadata()
        {
            if beneath(&parent, Path::new(&staging), OFlags::PATH, Mode::empty())
                .and_then(|f| f.metadata().map_err(io_error))
                .is_ok_and(|m| m.dev() == identity.dev() && m.ino() == identity.ino())
            {
                let _ = rfs::unlinkat(&parent, &staging, rfs::AtFlags::empty());
            }
        }
        result
    }
}
