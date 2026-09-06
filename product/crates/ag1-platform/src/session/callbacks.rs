//! Request callbacks borrow a fenced native view. Their output is retained
//! before interpretation, and accepted workers outlive disconnected waiters.
use super::{NativeSession, ToolRunner};
use ag1_core::{
    DomainError,
    context::ContextArtifactPort,
    runtime::{
        SessionRuntime,
        boundary::Stamp,
        session::{Configuration, PreparationAction},
    },
    storage::StoragePort,
    tools::registry::Scope,
};
use ag1_storage::{Store, context::ContextStore};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
mod payload;
mod start;
pub(super) use payload::PayloadInput;
pub(super) use start::Start;

pub(super) struct Capture {
    state: Arc<Mutex<SessionRuntime>>,
    store: Arc<Store>,
    scope: Scope,
    stamp: Stamp,
    branch: Arc<Vec<Value>>,
    configuration: Configuration,
    pending: bool,
}
impl Capture {
    pub(super) fn request_view(&mut self, request: &PreparationAction) -> Result<(), DomainError> {
        if self.stamp != *request.stamp() || self.scope.revision != request.journal_revision() {
            return Err(DomainError::Conflict);
        }
        self.configuration = request.configuration().clone();
        Ok(())
    }
    fn validate(&self, cancel: &AtomicBool) -> Result<(), DomainError> {
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let state = self.state.lock().map_err(|_| DomainError::Conflict)?;
        if state.stamp() != &self.stamp
            || state.journal_revision() != self.scope.revision
            || state.pending_commit()
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub(super) fn call(
        &self,
        tools: &dyn ToolRunner,
        event: &Value,
        cancel: &AtomicBool,
        artifacts: &mut ContextStore<'_>,
    ) -> Result<Option<Value>, DomainError> {
        self.validate(cancel)?;
        let stored = self.store.journal_snapshot(self.stamp.session())?;
        if stored.revision != self.scope.revision {
            return Err(DomainError::Conflict);
        }
        let snapshot = super::extensions::snapshot::configuration(
            &self.configuration,
            super::extensions::snapshot::session(&stored, &self.branch)?,
            false,
            self.pending,
        );
        let source = json!({"type":"session","name":"extension-callback"});
        artifacts.put_evidence(
            &json!({"version":1,"event":event}).to_string(),
            "runtime-extension-input",
            &source,
            &[],
            None,
        )?;
        let result = tools.request_event(&self.scope, &snapshot, event, cancel)?;
        if let Some(result) = &result {
            artifacts.put_evidence(
                &json!({"version":1,"event":event["type"],"result":result}).to_string(),
                "runtime-extension-output",
                &source,
                &[],
                None,
            )?;
        }
        self.validate(cancel)?;
        Ok(result)
    }
}
impl NativeSession {
    pub(super) fn capture_callback(&self) -> Result<Capture, DomainError> {
        let state = self.state()?;
        let stamp = state.stamp().clone();
        let scope = Scope {
            workspace: self.workspace.clone(),
            session: stamp.session().as_str().into(),
            epoch: stamp.epoch(),
            revision: state.journal_revision(),
        };
        let branch = state.branch_snapshot();
        let configuration = state.configuration().clone();
        drop(state);
        let pending = self
            .command_owner
            .get()
            .and_then(std::sync::Weak::upgrade)
            .map(|commands| commands.has_pending_messages())
            .transpose()?
            .unwrap_or(false);
        Ok(Capture {
            state: self.state.clone(),
            store: self.store.clone(),
            scope,
            stamp,
            branch,
            configuration,
            pending,
        })
    }
    pub(super) async fn request_callback(
        &self,
        tools: Arc<dyn ToolRunner>,
        event: Value,
        cancel: Arc<AtomicBool>,
        operation: &str,
        timestamp: &str,
    ) -> Result<Option<Value>, DomainError> {
        if !tools.handles_session_event(event["type"].as_str().unwrap_or("")) {
            return Ok(None);
        }
        self.wait_for_commit().await?;
        let capture = self.capture_callback()?;
        let activity = self.retain_work()?;
        let origin = self.origin.clone();
        let operation = operation.to_owned();
        let timestamp = timestamp.to_owned();
        self.pool
            .submit(move || {
                let _activity = activity;
                let mut artifacts = ContextStore::new(
                    &capture.store,
                    capture.stamp.session().clone(),
                    &origin,
                    &operation,
                    &timestamp,
                    Default::default(),
                )?;
                capture.call(tools.as_ref(), &event, &cancel, &mut artifacts)
            })
            .await
    }
}

pub(super) fn images(message: &Value) -> Vec<Value> {
    message["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "image")
        .cloned()
        .collect()
}
pub(super) fn transform_input(message: &mut Value, result: &Value) -> Result<bool, DomainError> {
    match result["action"].as_str() {
        Some("handled") => Ok(true),
        Some("continue") => Ok(false),
        Some("transform") => {
            let text = result["text"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("extension input text".into()))?;
            let mut content = vec![json!({"type":"text","text":text})];
            let images = match result.get("images").filter(|v| !v.is_null()) {
                None => images(message),
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| DomainError::InvalidInput("extension input images".into()))?
                    .clone(),
            };
            if images.iter().any(|image| {
                image["type"] != "image"
                    || !image["data"].is_string()
                    || !image["mimeType"].is_string()
            }) {
                return Err(DomainError::InvalidInput("extension image content".into()));
            }
            content.extend(images);
            message["content"] = json!(content);
            ag1_core::context::validate_history(std::slice::from_ref(message))?;
            Ok(false)
        }
        _ => Err(DomainError::InvalidInput("extension input action".into())),
    }
}
