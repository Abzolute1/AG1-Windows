//! Client effects use native intent/receipt custody. An uncertain effect is
//! inspectable and never re-executed merely because an acknowledgement was lost.
use super::*;
pub(super) fn is_effect(command: &w::Command) -> bool {
    match command {
        w::Command::Files { action, .. } => matches!(
            action.as_str(),
            "save" | "change_undo" | "change_undo_recover" | "git_confirm"
        ),
        w::Command::Session { action, .. } => action == "export",
        w::Command::Project { action, .. } => {
            matches!(action.as_str(), "start" | "stop" | "discover")
        }
        w::Command::Artifacts { action, .. } => action.starts_with("upload_"),
        w::Command::Runtime { action, args } => {
            (action == "approval_policy" && args.get("mode").is_some())
                || action == "extension_answer"
                || (action.starts_with("terminal_") && action != "terminal_status")
        }
        _ => false,
    }
}
fn key(scope: &w::Scope, id: &str) -> String {
    format!(
        "native-client-{}",
        ag1_storage::digest(
            json!([scope.workspace, scope.session, id])
                .to_string()
                .as_bytes()
        )
    )
}
impl Host {
    pub(super) fn client_effect(&self, envelope: &w::CommandEnvelope) -> Result<w::BridgeFrame> {
        let key = key(&envelope.scope, envelope.id.as_str());
        let payload = json!({"kind":"client-effect","envelope":envelope});
        let admission = self.store.admit_client_command(
            envelope.scope.session.as_str(),
            &key,
            envelope.expected_revision.get(),
            &payload,
        )?;
        if !admission.inserted {
            return match admission.record.result {
                Some(bytes) => serde_json::from_slice(&bytes).map_err(json_error),
                None => Err(DomainError::DurabilityUnknown),
            };
        }
        let result = (match &envelope.command {
            w::Command::Files { action, args } if action == "git_confirm" => {
                self.local_git_confirm(args).map(Some)
            }
            w::Command::Files { action, args }
                if action == "change_undo" || action == "change_undo_recover" =>
            {
                self.change_undo(args, action == "change_undo_recover")
                    .map(Some)
            }
            _ => self.query(&envelope.command),
        })
        .and_then(|v| v.ok_or(DomainError::NotFound))
        .and_then(|data| {
            if serde_json::to_vec(&data).map_err(json_error)?.len() > 48000 {
                Err(DomainError::LimitExceeded("Client effect receipt".into()))
            } else {
                Ok(data)
            }
        });
        let (status, data) = match result {
            Ok(data) => ("completed", data),
            Err(error) => ("rejected", json!({"error":format!("{error:?}")})),
        };
        let frame = w::BridgeFrame::Reply {
            id: envelope.id.clone(),
            scope: envelope.scope.clone(),
            revision: self.commands.snapshot()?.revision,
            durable: true,
            status: status.into(),
            data,
        };
        self.store.settle_intent(
            &key,
            &admission.record.intent,
            &serde_json::to_vec(&frame).map_err(json_error)?,
        )?;
        Ok(frame)
    }
    pub(super) fn client_effect_result(&self, args: &Value) -> Result<Value> {
        let id = args["command"]
            .as_str()
            .filter(|s| s.len() <= 128)
            .ok_or(DomainError::NotFound)?;
        let snapshot = self.commands.snapshot()?;
        let record = self
            .store
            .command_result(&key(&snapshot.scope, id))?
            .ok_or(DomainError::NotFound)?;
        Ok(
            json!({"state":record.state,"receipt":record.result.map(|bytes|serde_json::from_slice::<Value>(&bytes).map_err(json_error)).transpose()?,"replay_allowed":false}),
        )
    }
}
