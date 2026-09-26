//! Approved, contained structured Git diff. Every user value is one argv element.
use crate::{
    file_tools::{FileExecution, FileOutcome, FileService},
    git::GitService,
    workspace::Workspace,
};
use ag1_core::{
    DomainError,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        git_diff,
        registry::Origin,
    },
};
use serde_json::Value;
use std::sync::{Arc, atomic::Ordering};
pub struct GitTools {
    workspace: Arc<Workspace>,
    git: GitService,
}
impl GitTools {
    pub fn new(workspace: Arc<Workspace>, git: GitService) -> Self {
        Self { workspace, git }
    }
    pub fn target_revision(&self) -> String {
        format!("git-diff:{}", self.workspace.id().as_str())
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<FileOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if scope.workspace != self.workspace.id().as_str()
            || authorization.call().name() != "diff"
            || authorization.call().origin() != Origin::BuiltIn
        {
            return Err(DomainError::PermissionDenied);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let target = self.target_revision();
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &target,
                now: now(),
            },
        )?;
        let args = call.args();
        let text = |key| {
            args.get(key)
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        };
        let files = FileService::new(self.workspace.clone());
        let path = |input: &str| -> Result<String, DomainError> {
            let relative = files.relative_path(input)?;
            // exists rejects symlinks and is valid for tracked-but-deleted paths.
            self.workspace.exists(&relative)?;
            Ok(if relative.is_empty() {
                ".".into()
            } else {
                relative
            })
        };
        let reference = text("againstBranch").or_else(|| text("againstCommit"));
        if reference.is_some_and(|s| {
            s.len() > 240
                || s.starts_with('-')
                || s.contains(['\0', '\r', '\n'])
                || s.contains("..")
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_/.-~^".contains(c))
        }) {
            return Err(DomainError::InvalidInput("bounded Git reference".into()));
        }
        let mut argv = vec![];
        if let Some(branch) = text("againstBranch") {
            argv.extend([format!("HEAD...{branch}"), "--".into(), ".".into()]);
        } else if let Some(commit) = text("againstCommit") {
            argv.extend([commit.into(), "--".into()]);
            if let Some(paths) = args.get("paths") {
                let paths = paths
                    .as_array()
                    .ok_or_else(|| DomainError::InvalidInput("Git paths".into()))?;
                if paths.len() > 128 {
                    return Err(DomainError::LimitExceeded("Git paths".into()));
                }
                for p in paths {
                    argv.push(path(
                        p.as_str()
                            .ok_or_else(|| DomainError::InvalidInput("Git path".into()))?,
                    )?);
                }
            }
            if argv.len() == 2 {
                argv.push(".".into());
            }
        } else {
            // Preserve the executable reference: from/to select working-tree paths
            // against the index. This is not a no-index comparison of two file bodies.
            argv.extend(["--".into(), path(text("from").unwrap_or("undefined"))?]);
            if let Some(to) = text("to").filter(|_| text("from").is_some()) {
                argv.push(path(to)?);
            }
        }
        let max = args
            .get("maxChanges")
            .map(|v| {
                v.as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(|| DomainError::InvalidInput("integer maxChanges".into()))
            })
            .transpose()?
            .unwrap_or(500);
        if max == 0 || max > 100000 {
            return Err(DomainError::LimitExceeded("Git changes".into()));
        }
        let (raw, success) = self
            .git
            .tool_diff(&self.workspace, &argv, cancel, deadline)?;
        let result = if success {
            git_diff::project(&raw, max)?
        } else if (raw.contains("fatal: ambiguous argument") || raw.contains("fatal: bad revision"))
            && let Some(reference) = reference
        {
            git_diff::message(&format!("Git ref not found: {reference}"))
        } else {
            git_diff::message(&raw)
        };
        Ok(FileOutcome {
            result,
            published: None,
        })
    }
}
