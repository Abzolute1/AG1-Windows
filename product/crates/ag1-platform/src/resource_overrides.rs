//! Native embedding callbacks for derived resource views. These are not an RPC
//! registration surface. Original data remains alongside the derived projection.
use crate::{
    resource_fs::{ScanControl, validate_diagnostics},
    resources::ContextFile,
};
use ag1_core::{
    DomainError,
    resources::catalog::{self, Catalog, Kind, Trust},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
#[derive(Clone, Serialize)]
pub struct View {
    pub catalog: Catalog,
    pub agents_files: Vec<ContextFile>,
    pub system: Option<String>,
    pub append_system: Vec<String>,
}
/// Reload exposes all groups. Extension refresh exposes only its changed groups;
/// changes to other fields are rejected so captured context remains stable.
pub trait Overrides {
    fn apply(
        &self,
        view: &mut View,
        changed: &[Kind],
        full_reload: bool,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError>;
}
pub struct RegisteredOverride {
    pub(crate) name: String,
    pub(crate) callback: Arc<dyn Overrides>,
}
impl RegisteredOverride {
    pub fn new(name: &str, callback: Arc<dyn Overrides>) -> Result<Self, DomainError> {
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(DomainError::InvalidInput(
                "resource override identity".into(),
            ));
        }
        Ok(Self {
            name: name.into(),
            callback,
        })
    }
}
#[derive(Clone)]
pub struct Evidence {
    pub name: String,
    pub before_sha256: String,
    pub after_sha256: String,
    /// Kept as captured input, independently of callback-modified resource metadata.
    pub original: Arc<View>,
}
impl View {
    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        catalog::validate_view(&self.catalog)?;
        validate_diagnostics(self.catalog.diagnostics.iter())?;
        if self.agents_files.len() > 4096 || self.append_system.len() > 4096 {
            return Err(DomainError::LimitExceeded(
                "resource override context entries".into(),
            ));
        }
        let mut bytes = self.system.as_ref().map_or(0, String::len);
        for file in &self.agents_files {
            if file.path.len() > 8192
                || file.path.contains('\0')
                || file.content.len() > 1024 * 1024
            {
                return Err(DomainError::LimitExceeded(
                    "resource override context file".into(),
                ));
            }
            bytes = bytes.saturating_add(file.path.len() + file.content.len());
        }
        for append in &self.append_system {
            bytes = bytes.saturating_add(append.len());
        }
        if bytes > 32 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "resource override context bytes".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn hash(&self) -> Result<String, DomainError> {
        self.validate()?;
        Ok(format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(self)
                    .map_err(|_| DomainError::InvalidInput("resource override view".into()))?
            )
        ))
    }
}
pub(crate) fn apply(
    registration: &RegisteredOverride,
    original: View,
    baseline: Option<View>,
    changed: &[Kind],
    full: bool,
    control: &ScanControl<'_>,
) -> Result<(View, Evidence), DomainError> {
    control.check()?;
    let before = original.hash()?;
    let mut next = original.clone();
    registration
        .callback
        .apply(&mut next, changed, full, control)?;
    control.check()?;
    next.catalog.fingerprint = original.catalog.fingerprint.clone();
    next.validate()?;
    // Callback metadata never admits executable code. The native ExtensionService
    // still recaptures entries and applies canonical path trust before launching.
    for extension in &mut next.catalog.extensions {
        extension.trust = Trust::UntrustedData;
    }
    if !full {
        let mut allowed = serde_json::to_value(&original)
            .map_err(|_| DomainError::InvalidInput("resource override view".into()))?;
        let actual = serde_json::to_value(&next)
            .map_err(|_| DomainError::InvalidInput("resource override view".into()))?;
        for kind in changed {
            allowed["catalog"][kind.plural()] = actual["catalog"][kind.plural()].clone();
        }
        // Diagnostic changes are part of the affected resource refresh.
        allowed["catalog"]["diagnostics"] = actual["catalog"]["diagnostics"].clone();
        if allowed != actual {
            return Err(DomainError::PermissionDenied);
        }
    }
    next.catalog.fingerprint = original.catalog.fingerprint.clone();
    let after = next.hash()?;
    next.catalog.fingerprint = format!(
        "{:x}",
        Sha256::digest(format!(
            "host-override-v1:{}:{before}:{after}",
            registration.name
        ))
    );
    Ok((
        next,
        Evidence {
            name: registration.name.clone(),
            before_sha256: before,
            after_sha256: after,
            original: Arc::new(baseline.unwrap_or(original)),
        },
    ))
}
