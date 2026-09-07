use super::*;
use ag1_core::storage::StoragePort;
use base64::{Engine, engine::general_purpose::STANDARD};
impl Host {
    pub(super) fn detail_page(&self, args: &Value) -> Result<Value> {
        let selected = self.registry.selected()?;
        let runtime = selected.services.runtime();
        let snapshot = self.commands.snapshot()?;
        let section = args["section"].as_str().ok_or(DomainError::NotFound)?;
        let value=match section {
            "task"=>runtime.client_records()?.iter().rev().find(|r|r["customType"]=="task_state").cloned().unwrap_or(Value::Null),
            "context"=>runtime.client_context()?,
            "memory"=>self.store.load_memory(self.workspace.id().as_str())?.map(|m|json!({"sha256":m.sha256,"bytes":m.bytes.len(),"text":String::from_utf8_lossy(&m.bytes),"diagnostic":m.diagnostic})).unwrap_or(Value::Null),
            "activity"=>json!({"observations":self.observations.lock().map_err(|_|DomainError::Conflict)?.view(&snapshot.scope),"queue":snapshot.queued,"last_terminal":snapshot.last_terminal,"jobs":selected.services.tools().client_activity()?,"leases":self.store.evidence_lease_records(snapshot.scope.session.as_str())?}),
            "artifact"=>{
                use ag1_core::evidence::{EvidencePort,EvidenceRequest};
                let id=args["artifact"].as_str().ok_or(DomainError::NotFound)?;
                self.store.record(&EvidenceRequest {requester:SessionId::new(snapshot.scope.session.as_str())?,origin:self.store.artifact_origin(snapshot.scope.session.as_str(),id)?,artifact:id.into()})?
            },
            "models"=>json!(self.config.models.iter().map(|m|json!({"provider":m["provider"],"id":m["id"],"api":m["api"],"contextWindow":m["contextWindow"],"maxTokens":m["maxTokens"],"input":m["input"],"runtime":m["runtime"],"conflict":m["runtimeConflict"]})).collect::<Vec<_>>()),
            _=>return Err(DomainError::NotFound),
        };
        let bytes = serde_json::to_vec_pretty(&value).map_err(json_error)?;
        let digest = ag1_storage::digest(&bytes);
        if let Some(expected) = args["sha256"].as_str()
            && expected != digest
        {
            return Err(DomainError::Conflict);
        }
        let offset = args["offset"]
            .as_str()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| DomainError::InvalidInput("detail offset".into()))?;
        if offset > bytes.len() {
            return Err(DomainError::InvalidInput("detail offset".into()));
        }
        let end = (offset + 32768).min(bytes.len());
        Ok(
            json!({"base64":STANDARD.encode(&bytes[offset..end]),"next":if end<bytes.len(){Some(end.to_string())}else{None},"sha256":digest,"bytes":bytes.len(),"journal_revision":snapshot.revision}),
        )
    }
}
impl Host {
    pub(super) fn user_text(&self, args: &Value) -> Result<Value> {
        let journal = self.store.journal_snapshot(&SessionId::new(
            self.commands.snapshot()?.scope.session.as_str(),
        )?)?;
        let raw = journal
            .entries
            .iter()
            .find(|e| args["id"] == e.id)
            .ok_or(DomainError::NotFound)?;
        let entry: Value = serde_json::from_slice(&raw.normalized).map_err(json_error)?;
        if entry["message"]["role"] != "user" {
            return Err(DomainError::InvalidInput("user message".into()));
        }
        let content = &entry["message"]["content"];
        let text = content.as_str().map(str::to_owned).unwrap_or_else(|| {
            content
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
        let bytes = text.as_bytes();
        let offset = args["offset"]
            .as_str()
            .unwrap_or("0")
            .parse::<usize>()
            .map_err(|_| DomainError::NotFound)?;
        if offset > bytes.len() {
            return Err(DomainError::NotFound);
        }
        let end = (offset + 32768).min(bytes.len());
        Ok(
            json!({"base64":STANDARD.encode(&bytes[offset..end]),"next":if end<bytes.len(){Some(end.to_string())}else{None}}),
        )
    }
}
