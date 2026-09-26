use super::*;
use crate::{
    change_receipts::{self, ChangeReceipt},
    workspace::UndoStage,
};

impl Host {
    pub(super) fn undo_receipt(
        &self,
        args: &Value,
        write: bool,
    ) -> Result<(ChangeReceipt, Arc<Workspace>)> {
        let scope = self.commands.snapshot()?.scope;
        let id = args["id"].as_str().ok_or(DomainError::NotFound)?;
        let receipt: ChangeReceipt =
            serde_json::from_value(self.store.native_file_change(scope.session.as_str(), id)?)
                .map_err(json_error)?;
        let records = self
            .registry
            .selected()?
            .services
            .runtime()
            .client_records()?;
        change_receipts::verify_branch(
            &receipt,
            &records,
            scope.session.as_str(),
            scope.workspace.as_str(),
        )?;
        let path = Path::new(&receipt.root).join(&receipt.path);
        let root = self
            .folder_selection()?
            .resolve(path.to_str().ok_or(DomainError::NotFound)?, write)?;
        if root.root() != Path::new(&receipt.root) {
            return Err(DomainError::PermissionDenied);
        }
        Ok((receipt, root))
    }

    pub(super) fn change_undo_status(&self, args: &Value) -> Result<Value> {
        let (receipt, root) = self.undo_receipt(args, false)?;
        if let Some(value) = self.store.native_file_undo(&receipt.session, &receipt.id)? {
            let recoverable = matches!(
                value["state"].as_str(),
                Some("prepared" | "recovery_required")
            ) && self.undo_receipt(args, true).is_ok()
                && !root.exists(&receipt.path).unwrap_or(true)
                && value["recovery"]
                    .as_str()
                    .is_some_and(|path| root.exists(path).unwrap_or(false));
            return Ok(json!({"allowed":false,"outcome":value,"recoverable":recoverable}));
        }
        let scope = self.commands.snapshot()?;
        let reason = if receipt.state != "committed" {
            Some("The native effect is not confirmed. Review its outcome before changing files.")
        } else if !scope.queued.is_empty() {
            Some("Finish or stop the current response before Undo.")
        } else if self.undo_receipt(args, true).is_err() {
            Some("This folder is read-only or its grant was revoked.")
        } else if !receipt
            .after_revision
            .as_ref()
            .and_then(|r| ag1_core::FileRevision::try_from(r).ok())
            .is_some_and(|expected| {
                root.snapshot(&receipt.path, crate::workspace::MAX_READ)
                    .is_ok_and(|s| s.revision == expected)
            })
        {
            Some(
                "This file changed after the recorded operation. Undo would conflict with later work.",
            )
        } else {
            None
        };
        Ok(
            json!({"allowed":reason.is_none(),"reason":reason,"id":receipt.id,"path":receipt.path,"operation":if receipt.before.is_some(){"restore_original"}else{"remove_created"},"scope":"one_native_operation"}),
        )
    }

    /// Dispatch already holds workspace and host admission locks. Check all
    /// conversation owners, including background work, before admitting Undo.
    pub(super) async fn ensure_undo_idle(&self, args: &Value) -> Result<()> {
        let (_, root) = self.undo_receipt(args, true)?;
        self.ensure_folder_idle(root.root()).await
    }
    pub(super) async fn ensure_folder_idle(&self,root:&Path)->Result<()> {
        for host in self
            .shared_hosts
            .lock()
            .await
            .iter()
            .filter_map(std::sync::Weak::upgrade)
        {
            let overlapping = host.folder_selection().map_or(true, |f| {
                f.selection
                    .folders
                    .iter()
                    .any(|g| root.starts_with(&g.path) || g.path.starts_with(root))
            });
            if !overlapping {
                continue;
            }
            host.undo_activity_check()?;
        }
        Ok(())
    }

    pub(super) fn undo_activity_check(&self) -> Result<()> {
        let selected = self.registry.selected()?;
        if !self.commands.snapshot()?.queued.is_empty()
            || !selected.services.runtime().settled()?
            || self.commands.client_transition_pending()?
            || self
                .terminal
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .is_some()
            || selected.services.tools().client_activity()?["jobs"]
                .as_array()
                .is_some_and(|jobs| {
                    jobs.iter().any(|j| {
                        !matches!(
                            j["status"].as_str(),
                            Some("completed" | "failed" | "cancelled" | "exited")
                        )
                    })
                })
            || self.working_project().ok().is_some_and(|p| {
                p.lock().map_or(true, |slot| {
                    slot.as_ref()
                        .is_some_and(|runtime| !runtime.observed_pids().is_empty())
                })
            })
        {
            return Err(DomainError::Unavailable("Finish running conversations, terminals and background work in this folder before Undo.".into()));
        }
        Ok(())
    }

    pub(super) fn change_undo(&self, args: &Value, recover: bool) -> Result<Value> {
        let (receipt, root) = self.undo_receipt(args, true)?;
        let scope = self.commands.snapshot()?.scope;
        let records = self
            .registry
            .selected()?
            .services
            .runtime()
            .client_records()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let check = |_: UndoStage| -> Result<()> {
            if self.closing.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if std::time::Instant::now() >= deadline {
                return Err(DomainError::TimedOut);
            }
            self.check_authority()?;
            if self.commands.snapshot()?.scope != scope {
                return Err(DomainError::Conflict);
            }
            self.undo_receipt(args, true)?;
            self.undo_activity_check()
        };
        let apply = if recover {
            crate::change_undo::recover
        } else {
            crate::change_undo::execute
        };
        apply(
            &self.store,
            &root,
            &receipt,
            &records,
            scope.session.as_str(),
            scope.workspace.as_str(),
            check,
        )
    }
}
