//! Cancellation records intent before signalling a particular admitted prompt.
//! Its receipt confirms delivery, not settlement of the target's native effects.
use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: u32,
    kind: String,
    target_key: String,
    target: w::OperationId,
}

fn target_record(
    store: &ag1_storage::Store,
    scope: &w::Scope,
    key: &str,
) -> Result<StoredCommand, w::ProtocolError> {
    let record = store
        .command_result(key)
        .map_err(storage)?
        .ok_or(w::ProtocolError::NotFound)?;
    let intent: serde_json::Value = serde_json::from_slice(&record.intent)
        .map_err(|_| unavailable("native_command_recovery"))?;
    let envelope: w::CommandEnvelope = serde_json::from_value(intent["payload"].clone())
        .map_err(|_| w::ProtocolError::NotFound)?;
    let expected_key = format!(
        "native-command-{}",
        ag1_storage::digest(
            json!([scope.workspace, scope.session, envelope.id])
                .to_string()
                .as_bytes()
        )
    );
    if envelope.scope.workspace != scope.workspace
        || envelope.scope.session != scope.session
        || super::messages::input(&envelope.command).is_err()
        || expected_key != key
    {
        return Err(w::ProtocolError::NotFound);
    }
    Ok(record)
}

fn receipt(old: &StoredCommand) -> Result<Option<Receipt>, w::ProtocolError> {
    let intent: serde_json::Value =
        serde_json::from_slice(&old.intent).map_err(|_| unavailable("native_command_recovery"))?;
    if intent["payload"]["command"]["kind"] != "cancel" {
        return Ok(None);
    }
    if old.state != "completed" {
        return Err(unavailable("native_command_recovery"));
    }
    let receipt: Receipt = serde_json::from_slice(
        old.result
            .as_deref()
            .ok_or_else(|| unavailable("native_command_recovery"))?,
    )
    .map_err(|_| unavailable("native_command_recovery"))?;
    if receipt.version != 1
        || receipt.kind != "cancellation_delivered"
        || SessionCommands::operation(&receipt.target_key) != receipt.target
        || intent["payload"]["command"]["operation"] != receipt.target.as_str()
    {
        return Err(unavailable("native_command_recovery"));
    }
    Ok(Some(receipt))
}

impl SessionCommands {
    /// Resolve an immutable historical receipt without reconstructing a queue
    /// or treating delivery of cancellation as settlement of its target.
    pub(in crate::session) fn historical_terminal(
        store: &ag1_storage::Store,
        scope: &w::Scope,
        key: &str,
        old: &StoredCommand,
    ) -> Result<w::OperationView, w::ProtocolError> {
        if let Some(receipt) = receipt(old)? {
            let target = target_record(store, scope, &receipt.target_key)?;
            if target.state != "completed" {
                return Err(unavailable("native_command_recovery"));
            }
            Self::terminal(&receipt.target_key, &target)
        } else {
            Self::terminal(key, old)
        }
    }

    pub(super) fn cancel_worker(
        &self,
        envelope: &w::CommandEnvelope,
        key: &str,
        target: &w::OperationId,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        let (target_key, cancelled) = {
            let book = self.book().map_err(domain)?;
            if let Some(active) = book.active.as_ref().filter(|a| &a.operation == target) {
                if active.recovery {
                    return Err(unavailable("native_command_recovery"));
                }
                (active.key.clone(), Some(active.cancelled.clone()))
            } else if let Some(queued) = book.queue.iter().find(|q| &q.operation == target) {
                (queued.key.clone(), Some(queued.cancelled.clone()))
            } else {
                // IDs alone cannot authorize another session's operation or an
                // unfinished admission recovered without a live execution claim.
                let target_key = target.as_str().to_owned();
                drop(book);
                let old = target_record(&self.runtime().store, &self.scope, &target_key)?;
                if old.state != "completed" {
                    return Err(unavailable("native_command_recovery"));
                }
                Self::terminal(&target_key, &old)?;
                (target_key, None)
            }
        };
        let payload =
            serde_json::to_value(envelope).map_err(|_| unavailable("native_command_recovery"))?;
        let admitted = self
            .runtime()
            .store
            .admit_session_command(
                self.scope.session.as_str(),
                key,
                envelope.expected_revision.get(),
                &payload,
            )
            .map_err(storage)?;
        if admitted.inserted {
            if let Some(cancelled) = cancelled {
                self.runtime().cancel_command(&cancelled).map_err(domain)?;
            }
            let receipt = serde_json::to_vec(&Receipt {
                version: 1,
                kind: "cancellation_delivered".into(),
                target_key,
                target: target.clone(),
            })
            .map_err(|_| unavailable("native_command_recovery"))?;
            self.runtime()
                .store
                .settle_intent(key, &admitted.record.intent, &receipt)
                .map_err(storage)?;
        }
        Ok(w::CommandResult::Accepted {
            operation: target.clone(),
        })
    }

    pub(super) fn cancel_lookup(
        &self,
        old: &StoredCommand,
    ) -> Result<Option<w::CommandResult>, w::ProtocolError> {
        let Some(receipt) = receipt(old)? else {
            return Ok(None);
        };
        let target = target_record(&self.runtime().store, &self.scope, &receipt.target_key)?;
        if target.state == "completed" {
            return Ok(Some(w::CommandResult::Settled {
                operation: Self::terminal(&receipt.target_key, &target)?,
            }));
        }
        let book = self.book().map_err(domain)?;
        if book.queue.iter().any(|q| q.key == receipt.target_key)
            || book
                .active
                .as_ref()
                .is_some_and(|a| a.key == receipt.target_key && !a.recovery)
        {
            Ok(Some(w::CommandResult::Accepted {
                operation: receipt.target,
            }))
        } else {
            Err(unavailable("native_command_recovery"))
        }
    }
}
