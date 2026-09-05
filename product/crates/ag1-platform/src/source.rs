//! Byte-consistent source snapshots through the P03 root capability.
use crate::{
    io_error,
    workspace::{MAX_READ, Workspace},
};
use ag1_core::{
    DomainError,
    evidence::{Freshness, SourceReference, SourceSnapshot, SourceValidation},
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
pub struct SourceService {
    workspace: Arc<Workspace>,
    references: BTreeMap<String, SourceReference>,
}
impl SourceService {
    pub fn new(workspace: Arc<Workspace>) -> Self {
        Self {
            workspace,
            references: BTreeMap::new(),
        }
    }
    fn canonical(&self, input: &str) -> Result<(PathBuf, String), DomainError> {
        self.workspace.verify_root()?;
        if input.len() > 4096 || input.contains(['\r', '\n', '\0']) {
            return Err(DomainError::InvalidInput("source path".into()));
        }
        let absolute = self
            .workspace
            .root()
            .join(input)
            .canonicalize()
            .map_err(io_error)?;
        let relative = absolute
            .strip_prefix(self.workspace.root())
            .map_err(|_| DomainError::PermissionDenied)?
            .to_str()
            .ok_or_else(|| DomainError::InvalidInput("source UTF-8 path".into()))?
            .to_owned();
        SourceReference::new(&relative, &"0".repeat(64))?;
        Ok((absolute, relative))
    }
    pub fn capture(&self, input: &str) -> Result<SourceSnapshot, DomainError> {
        self.capture_with_barrier(input, || {})
    }
    pub fn capture_with_barrier(
        &self,
        input: &str,
        after_read: impl FnOnce(),
    ) -> Result<SourceSnapshot, DomainError> {
        let (path, relative) = self.canonical(input)?;
        let snapshot = self
            .workspace
            .snapshot_with_barrier(&relative, MAX_READ, after_read)?;
        if self.canonical(input)?.0 != path {
            return Err(DomainError::Conflict);
        }
        SourceSnapshot::new(&relative, snapshot)
    }
    pub fn validate(&mut self, reference: &str) -> SourceValidation {
        let result = if !reference.starts_with("file:") {
            return SourceValidation {
                reference: reference.into(),
                status: Freshness::Unverified,
                current: None,
            };
        } else {
            self.references
                .get(reference)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| SourceReference::parse(reference))
        };
        let parsed = match result {
            Ok(r) => r,
            Err(_) => {
                return SourceValidation {
                    reference: reference.into(),
                    status: Freshness::Invalid,
                    current: None,
                };
            }
        };
        // Always re-read bytes through validated descriptors. Metadata-only caching cannot certify freshness.
        match self.capture(parsed.path()) {
            Ok(snapshot) => {
                let current = snapshot.reference();
                let status =
                    if current.path() == parsed.path() && current.digest() == parsed.digest() {
                        Freshness::Current
                    } else {
                        Freshness::Stale
                    };
                if self.references.len() >= 256 && !self.references.contains_key(reference) {
                    let key = self.references.keys().next().cloned().unwrap();
                    self.references.remove(&key);
                }
                self.references.insert(reference.into(), parsed.clone());
                SourceValidation {
                    reference: reference.into(),
                    status,
                    current: Some(current.reference()),
                }
            }
            Err(error) => {
                self.references.remove(reference);
                SourceValidation {
                    reference: reference.into(),
                    status: match error {
                        DomainError::NotFound => Freshness::Missing,
                        DomainError::Conflict => Freshness::Stale,
                        _ => Freshness::Invalid,
                    },
                    current: None,
                }
            }
        }
    }
}
