//! One bounded immutable display snapshot per native conversation owner.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) struct DiffReview {
    scope: w::Scope,
    root: PathBuf,
    sha256: String,
    bytes: Vec<u8>,
}
impl Host {
    pub(super) fn diff_page(&self, args: &Value) -> Result<Value> {
        let scope = self.commands.snapshot()?.scope;
        let workspace = self.working_workspace()?;
        let offset = args["offset"]
            .as_str()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| DomainError::NotFound)?;
        if offset == 0 && args["sha256"].is_null() {
            let tooling = self.config.tooling.as_ref().ok_or_else(|| {
                DomainError::Unavailable("Configure the native Git helper".into())
            })?;
            let spools = self.state.join("diff");
            fs::create_dir_all(&spools).map_err(crate::io_error)?;
            let diff = crate::git::GitService::new(&tooling.process_host, &tooling.git, &spools)?.with_bubblewrap(&tooling.bubblewrap)
                .status_diff(&workspace, None)?;
            let bytes = format!(
                "HEAD {}\n{}\n{}",
                diff.head,
                String::from_utf8_lossy(&diff.status),
                String::from_utf8_lossy(&diff.diff)
            )
            .into_bytes();
            if bytes.len() > 1024 * 1024 {
                return Err(DomainError::LimitExceeded(
                    "Workspace review exceeds the 1 MB display limit".into(),
                ));
            }
            *self.diff_review.lock().map_err(|_| DomainError::Conflict)? = Some(DiffReview {
                scope: scope.clone(),
                root: workspace.root().into(),
                sha256: ag1_storage::digest(&bytes),
                bytes,
            });
        }
        let cache = self.diff_review.lock().map_err(|_| DomainError::Conflict)?;
        let view = cache.as_ref().ok_or(DomainError::NotFound)?;
        if view.scope != scope
            || view.root != workspace.root()
            || offset > view.bytes.len()
            || args["sha256"]
                .as_str()
                .is_some_and(|sha| sha != view.sha256)
            || offset > 0 && args["sha256"].is_null()
        {
            return Err(DomainError::Conflict);
        }
        let end = offset.saturating_add(24000).min(view.bytes.len());
        Ok(
            json!({"base64":STANDARD.encode(&view.bytes[offset..end]),"sha256":view.sha256,"bytes":view.bytes.len(),"next":if end < view.bytes.len(){Some(end.to_string())}else{None}}),
        )
    }
}
