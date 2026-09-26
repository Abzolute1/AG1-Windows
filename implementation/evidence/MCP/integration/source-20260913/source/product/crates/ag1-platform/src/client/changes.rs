//! Read-only review of native receipts, bound to the current journal branch.
use super::*;
use crate::change_receipts::{self, ChangeReceipt};
use base64::{Engine, engine::general_purpose::STANDARD};

impl Host {
    pub(super) fn change_list(&self, args: &Value) -> Result<Value> {
        let selected = self.registry.selected()?;
        let records = selected.services.runtime().client_records()?;
        let scope = self.commands.snapshot()?.scope;
        let turn = args["turn"].as_str().ok_or(DomainError::NotFound)?;
        let calls = change_receipts::turn_calls(&records, turn)?;
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        if offset > calls.len() {
            return Err(DomainError::NotFound);
        }
        let end = offset.saturating_add(16).min(calls.len());
        let mut changes = Vec::new();
        for value in self
            .store
            .native_file_changes(scope.session.as_str(), &calls[offset..end])?
        {
            let receipt: ChangeReceipt =
                serde_json::from_value(value.clone()).map_err(json_error)?;
            if change_receipts::verify_branch(
                &receipt,
                &records,
                scope.session.as_str(),
                scope.workspace.as_str(),
            )
            .is_err()
            {
                continue;
            }
            self.change_folder(&receipt)?;
            changes.push(change_receipts::summary(&value)?);
        }
        Ok(
            json!({"turn":turn,"changes":changes,"next":if end < calls.len(){Some(end)}else{None},
            "scope":"selected_turn","notice":"Native write/edit receipts only. Shell, rename and delete changes remain workspace changes."}),
        )
    }

    fn change_folder(&self, receipt: &ChangeReceipt) -> Result<()> {
        let target = Path::new(&receipt.root).join(&receipt.path);
        let root = self
            .folder_selection()?
            .resolve(target.to_str().ok_or(DomainError::NotFound)?, false)?;
        if root.root() != Path::new(&receipt.root) {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }

    pub(super) fn change_read(&self, args: &Value) -> Result<Value> {
        let scope = self.commands.snapshot()?.scope;
        let records = self
            .registry
            .selected()?
            .services
            .runtime()
            .client_records()?;
        let id = args["id"].as_str().ok_or(DomainError::NotFound)?;
        let receipt: ChangeReceipt =
            serde_json::from_value(self.store.native_file_change(scope.session.as_str(), id)?)
                .map_err(json_error)?;
        change_receipts::verify_branch(
            &receipt,
            &records,
            scope.session.as_str(),
            scope.workspace.as_str(),
        )?;
        self.change_folder(&receipt)?;
        let side = args["side"].as_str().unwrap_or("diff");
        let bytes = match side {
            "before" => receipt
                .before
                .as_ref()
                .map(|s| change_receipts::read_snapshot(&self.store, scope.session.as_str(), s))
                .transpose()?
                .unwrap_or_default(),
            "after" => {
                change_receipts::read_snapshot(&self.store, scope.session.as_str(), &receipt.after)?
            }
            "diff" => {
                if receipt.before.as_ref().map_or(0, |s| s.bytes) + receipt.after.bytes > 256 * 1024
                {
                    return Ok(
                        json!({"unavailable":"This change is too large for an inline diff. Its exact before and after bytes are available in pages."}),
                    );
                }
                let before = receipt
                    .before
                    .as_ref()
                    .map(|s| change_receipts::read_snapshot(&self.store, scope.session.as_str(), s))
                    .transpose()?
                    .unwrap_or_default();
                let after = change_receipts::read_snapshot(
                    &self.store,
                    scope.session.as_str(),
                    &receipt.after,
                )?;
                match (std::str::from_utf8(&before), std::str::from_utf8(&after)) {
                    (Ok(a), Ok(b)) if !a.contains('\0') && !b.contains('\0') => {
                        match ag1_core::tools::diff::generate(a, b, 3) {
                            Ok(diff) => diff.diff.into_bytes(),
                            Err(DomainError::LimitExceeded(_)) => {
                                return Ok(
                                    json!({"unavailable":"This diff exceeds the display budget. Review the before and after source pages."}),
                                );
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    _ => {
                        return Ok(
                            json!({"unavailable":"Binary change. Exact before and after bytes are available for download."}),
                        );
                    }
                }
            }
            _ => return Err(DomainError::InvalidInput("change view".into())),
        };
        let offset = args["offset"]
            .as_str()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| DomainError::NotFound)?;
        if offset > bytes.len() {
            return Err(DomainError::NotFound);
        }
        let end = offset.saturating_add(24000).min(bytes.len());
        Ok(
            json!({"base64":STANDARD.encode(&bytes[offset..end]),"next":if end < bytes.len(){Some(end.to_string())}else{None},
            "bytes":bytes.len(),"sha256":ag1_storage::digest(&bytes),"side":side,"id":id,"state":receipt.state}),
        )
    }
}
