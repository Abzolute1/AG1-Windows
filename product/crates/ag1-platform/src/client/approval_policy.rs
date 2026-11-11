//! User-selected approval mode, shared by cached session runners. Changes are
//! admitted only while the host is idle; a tool batch uses one immutable snapshot.
use super::*;
use ag1_core::tools::policy::Mode;
use std::io::Write;
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub mode: Mode,
    pub revision: u64,
}
pub struct ApprovalPolicy {
    value: Mutex<Selection>,
    path: PathBuf,
}
impl ApprovalPolicy {
    fn persist(path: &Path, next: Selection) -> Result<()> {
        let parent = path.parent().ok_or(DomainError::Conflict)?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(crate::io_error)?;
        file.write_all(&serde_json::to_vec(&next).map_err(json_error)?)
            .map_err(crate::io_error)?;
        file.as_file().sync_all().map_err(crate::io_error)?;
        file.persist(path).map_err(|e| crate::io_error(e.error))?;
        fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| DomainError::DurabilityUnknown)
    }
    pub fn open(state: &Path, default: &str) -> Result<Arc<Self>> {
        let path = state.join("approval-policy.json");
        let default = serde_json::from_value(json!(default)).map_err(json_error)?;
        let mut value = match fs::read(&path) {
            Ok(bytes) if bytes.len() <= 1024 => {
                serde_json::from_slice(&bytes).map_err(json_error)?
            }
            Ok(_) => return Err(DomainError::LimitExceeded("Approval preferences".into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Selection {
                mode: default,
                revision: 1,
            },
            Err(e) => return Err(crate::io_error(e)),
        };
        if value.revision == 0 {
            return Err(DomainError::Conflict);
        }
        // Bypass is an in-memory acknowledgement, never a durable preference.
        // Clear a record written by older builds before publishing the policy.
        if value.mode == Mode::Disabled && default != Mode::Disabled {
            value = Selection {
                mode: default,
                revision: value.revision.checked_add(1).ok_or(DomainError::Conflict)?,
            };
            Self::persist(&path, value)?;
        }
        Ok(Arc::new(Self {
            value: Mutex::new(value),
            path,
        }))
    }
    pub fn snapshot(&self) -> Result<Selection> {
        Ok(*self.value.lock().map_err(|_| DomainError::Conflict)?)
    }
    pub(super) fn set(&self, mode: Mode) -> Result<Selection> {
        if !matches!(mode, Mode::Interactive | Mode::Autonomous | Mode::Disabled) {
            return Err(DomainError::InvalidInput(
                "Choose interactive, autonomous or disabled approvals".into(),
            ));
        }
        let mut current = self.value.lock().map_err(|_| DomainError::Conflict)?;
        if current.mode == mode {
            return Ok(*current);
        }
        let next = Selection {
            mode,
            revision: current
                .revision
                .checked_add(1)
                .ok_or(DomainError::Conflict)?,
        };
        if mode != Mode::Disabled {
            Self::persist(&self.path, next)?;
        }
        *current = next;
        Ok(next)
    }
}
impl Host {
    pub(super) fn approval_policy(&self, args: &Value) -> Result<Value> {
        if let Some(mode) = args.get("mode") {
            let snapshot = self.commands.snapshot()?;
            if !snapshot.queued.is_empty()
                || !self.registry.selected()?.services.runtime().settled()?
            {
                return Err(DomainError::Unavailable(
                    "Approval settings apply after the current response finishes".into(),
                ));
            }
            self.approval_policy
                .set(serde_json::from_value(mode.clone()).map_err(json_error)?)?;
        }
        serde_json::to_value(self.approval_policy.snapshot()?).map_err(json_error)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_is_persisted_and_rejects_non_ui_modes() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ApprovalPolicy::open(dir.path(), "interactive").unwrap();
        assert_eq!(policy.set(Mode::Autonomous).unwrap().revision, 2);
        assert!(policy.set(Mode::Planning).is_err());
        assert_eq!(
            ApprovalPolicy::open(dir.path(), "interactive")
                .unwrap()
                .snapshot()
                .unwrap()
                .mode,
            Mode::Autonomous
        );
        assert_eq!(policy.set(Mode::Autonomous).unwrap().revision, 2);
    }
    #[test]
    fn bypass_is_memory_only_and_legacy_bypass_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let policy = ApprovalPolicy::open(dir.path(), "interactive").unwrap();
        assert_eq!(policy.set(Mode::Disabled).unwrap().mode, Mode::Disabled);
        assert_eq!(
            ApprovalPolicy::open(dir.path(), "interactive")
                .unwrap()
                .snapshot()
                .unwrap()
                .mode,
            Mode::Interactive
        );
        ApprovalPolicy::persist(
            &dir.path().join("approval-policy.json"),
            Selection {
                mode: Mode::Disabled,
                revision: 8,
            },
        )
        .unwrap();
        let reopened = ApprovalPolicy::open(dir.path(), "interactive").unwrap();
        assert_eq!(reopened.snapshot().unwrap().mode, Mode::Interactive);
        assert_eq!(reopened.snapshot().unwrap().revision, 9);
    }
}
