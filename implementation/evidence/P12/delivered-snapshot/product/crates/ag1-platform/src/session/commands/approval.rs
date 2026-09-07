use super::*;
use ag1_core::tools::approval::ApprovalRequest;

/// Bounded public request, projected from the actual prepared P09 capability.
/// The frontend receives no authority to change registry, policy or target fences.
#[derive(Clone, Debug)]
pub struct PendingApproval {
    pub binding: w::ApprovalBinding,
    pub display: serde_json::Value,
}

impl SessionCommands {
    fn project_approval(
        &self,
        request: &ApprovalRequest,
        operation: &w::OperationId,
    ) -> Result<PendingApproval, DomainError> {
        let native = &request.binding;
        let state = self.runtime().state()?;
        if native.workspace != self.scope.workspace.as_str()
            || native.session != self.scope.session.as_str()
            || native.epoch != state.stamp().epoch()
            || native.revision != state.journal_revision()
            || native.expires_ms <= self.clock.now_ms()
        {
            return Err(DomainError::Conflict);
        }
        Ok(PendingApproval {
            binding: w::ApprovalBinding {
                id: native
                    .id
                    .clone()
                    .try_into()
                    .map_err(DomainError::InvalidInput)?,
                operation: operation.clone(),
                argument_sha256: native.argument_hash.clone(),
                scope: self.scope.clone(),
                revision: w::Sequence::new(native.revision),
                expires_ms: w::Sequence::new(native.expires_ms),
            },
            display: request.display.clone(),
        })
    }

    pub fn pending_approvals(&self) -> Result<Vec<PendingApproval>, DomainError> {
        let _activity = self.runtime().enter_owned()?;
        let book = self.book()?;
        let Some(active) = book.active.as_ref().filter(|a| !a.recovery) else {
            return Ok(vec![]);
        };
        let Some(handler) = self.selection.services.tools().approval_handler() else {
            return Ok(vec![]);
        };
        handler
            .pending()?
            .iter()
            .filter_map(
                |request| match self.project_approval(request, &active.operation) {
                    Ok(view) => Some(Ok(view)),
                    Err(DomainError::Conflict) => None,
                    Err(error) => Some(Err(error)),
                },
            )
            .collect()
    }

    pub(super) fn approve_worker(
        &self,
        envelope: &w::CommandEnvelope,
        key: &str,
        binding: &w::ApprovalBinding,
        allow: bool,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        let handler = self
            .selection
            .services
            .tools()
            .approval_handler()
            .ok_or_else(|| unavailable("native_approval"))?;
        let request = {
            let book = self.book().map_err(domain)?;
            let active = book
                .active
                .as_ref()
                .filter(|a| !a.recovery && a.operation == binding.operation)
                .ok_or(w::ProtocolError::InvalidApproval)?;
            book.sequence
                .checked_add(2)
                .ok_or(w::ProtocolError::Capacity {
                    resource: "event_sequence".into(),
                })?;
            handler
                .pending()
                .map_err(domain)?
                .into_iter()
                .find(|request| {
                    self.project_approval(request, &active.operation)
                        .is_ok_and(|view| view.binding == *binding)
                })
                .ok_or(w::ProtocolError::InvalidApproval)?
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
        let operation = Self::operation(key);
        if admitted.inserted {
            // Recheck the host's current binding after the durable intent. The
            // eventual P09 consume also checks registry/policy/source revisions.
            self.project_approval(&request, &binding.operation)
                .map_err(|_| w::ProtocolError::InvalidApproval)?;
            handler
                .answer(&request.binding, allow)
                .map_err(|_| w::ProtocolError::InvalidApproval)?;
            let done = w::OperationView {
                operation: operation.clone(),
                status: w::OperationStatus::Completed,
            };
            let evidence = {
                let state = self.runtime().state().map_err(domain)?;
                serde_json::to_vec(&Terminal {
                    version: 1,
                    operation: done.clone(),
                    journal_revision: state.journal_revision(),
                    context_epoch: state.stamp().epoch(),
                    execution_settled: true,
                    error: None,
                    generations: None,
                })
                .map_err(|_| unavailable("native_command_recovery"))?
            };
            self.runtime()
                .store
                .settle_intent(key, &admitted.record.intent, &evidence)
                .map_err(storage)?;
            let mut book = self.book().map_err(domain)?;
            self.publish(&mut book, done.clone()).map_err(domain)?;
            book.last_terminal = Some(done);
        }
        Ok(w::CommandResult::Accepted { operation })
    }
}
