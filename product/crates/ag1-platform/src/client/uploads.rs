use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
impl Host {
    pub(super) fn upload(&self, action: &str, args: &Value) -> Result<Value> {
        let session = self.commands.snapshot()?.scope.session;
        let mut uploads = self.uploads.lock().map_err(|_| DomainError::Conflict)?;
        if action == "upload_begin" {
            if uploads.len() >= 16 {
                return Err(DomainError::QueueFull);
            }
            let id = nonce()?;
            uploads.insert(id.clone(), (session.as_str().into(), vec![]));
            return Ok(json!({"upload":id,"offset":"0"}));
        }
        let id = args["upload"].as_str().ok_or(DomainError::NotFound)?;
        let total: usize = uploads.values().map(|(_, bytes)| bytes.len()).sum();
        let (owner, bytes) = uploads.get_mut(id).ok_or(DomainError::NotFound)?;
        if owner != session.as_str() {
            return Err(DomainError::PermissionDenied);
        }
        match action {
            "upload_append" => {
                let offset = args["offset"]
                    .as_str()
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or(DomainError::Conflict)?;
                if offset != bytes.len() {
                    return Err(DomainError::Conflict);
                }
                let chunk = STANDARD
                    .decode(args["base64"].as_str().ok_or(DomainError::Conflict)?)
                    .map_err(|_| DomainError::InvalidInput("upload base64".into()))?;
                if chunk.len() > 32768
                    || bytes.len() + chunk.len() > 4 * 1024 * 1024
                    || total + chunk.len() > 16 * 1024 * 1024
                {
                    return Err(DomainError::LimitExceeded("attachment upload".into()));
                }
                bytes.extend(chunk);
                Ok(json!({"offset":bytes.len().to_string()}))
            }
            "upload_finish" => {
                if args["sha256"] != ag1_storage::digest(bytes) {
                    return Err(DomainError::Conflict);
                }
                std::str::from_utf8(bytes).map_err(|_| {
                    DomainError::InvalidInput(
                        "Upload a UTF-8 message document; image blocks carry base64".into(),
                    )
                })?;
                let artifact = format!("art_{id}");
                let record = json!({"version":1,"id":artifact,"sessionId":session,"kind":"client-message","producer":{"name":"client-upload","type":"user"},"encoding":"utf8","mimeType":"text/plain","sha256":ag1_storage::digest(bytes),"bytes":bytes.len(),"createdAt":timestamp()});
                self.store.put_artifact(&ag1_storage::ArtifactInput {
                    origin: session.as_str().into(),
                    session: session.as_str().into(),
                    record: serde_json::to_vec(&record).map_err(json_error)?,
                    bytes: bytes.clone(),
                })?;
                uploads.remove(id);
                Ok(json!({"artifact":artifact}))
            }
            "upload_cancel" => {
                uploads.remove(id);
                Ok(json!({"cancelled":true}))
            }
            _ => Err(DomainError::InvalidInput("upload action".into())),
        }
    }
}
