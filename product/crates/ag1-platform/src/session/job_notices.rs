//! Parent journal custody precedes the job registry's notification acknowledgement.
use super::{Listener, NativeSession, ToolRunner};
use ag1_core::{DomainError, tools::registry::Scope};
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

impl NativeSession {
    pub(super) async fn deliver_job_notices(
        &self,
        tools: Arc<dyn ToolRunner>,
        operation: &str,
        timestamp: &str,
        cancelled: &AtomicBool,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        let scope = {
            let state = self.state()?;
            Scope {
                workspace: self.workspace.clone(),
                session: state.stamp().session().as_str().into(),
                epoch: state.stamp().epoch(),
                revision: state.journal_revision(),
            }
        };
        let capture_scope = scope.clone();
        let capture_tools = tools.clone();
        let operation = format!("{operation}-job-notices");
        // Refresh may finish an owned process and save its full output. Its
        // lifecycle lease stays on the worker even if the caller disappears.
        let activity = self.retain_work()?;
        let notices = self
            .pool
            .submit(move || {
                let _activity = activity;
                capture_tools.job_notices(&capture_scope, &operation)
            })
            .await?;
        let mut ids = BTreeSet::new();
        if notices.len() > 200
            || notices.iter().any(|notice| {
                notice.id.is_empty()
                    || notice.id.len() > 256
                    || notice.id.contains('\0')
                    || notice.text.len() > 64 * 1024
                    || !ids.insert(notice.id.as_str())
            })
        {
            return Err(DomainError::InvalidInput("background job notices".into()));
        }
        let millis = chrono::DateTime::parse_from_rfc3339(timestamp)
            .map_err(|_| DomainError::InvalidInput("job notice timestamp".into()))?
            .timestamp_millis();
        for notice in notices {
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            let id = format!(
                "job-notice-{}",
                ag1_storage::digest(json!([scope.session, notice.id]).to_string().as_bytes())
            );
            let message = json!({"role":"custom","customType":"background_job_notice","content":notice.text,"display":false,"details":{"noticeId":notice.id},"timestamp":millis});
            let append = loop {
                self.wait_for_commit().await?;
                let mut state = self.state()?;
                if state.pending_commit() {
                    continue;
                }
                if let Some(existing) = state.branch().iter().find(|entry| entry["id"] == id) {
                    // A prior attempt saved the parent message but lost the job
                    // metadata receipt. Retry only that acknowledgement.
                    let prior = &existing["message"];
                    if prior["customType"] != message["customType"]
                        || prior["content"] != message["content"]
                        || prior["details"] != message["details"]
                    {
                        return Err(DomainError::Conflict);
                    }
                    break None;
                }
                break Some(state.append_message(&id, &id, timestamp, message)?);
            };
            if let Some(append) = append {
                self.commit(append, listener).await?;
            }
            let tools = tools.clone();
            let scope = scope.clone();
            let activity = self.retain_work()?;
            self.pool
                .submit(move || {
                    let _activity = activity;
                    tools.acknowledge_job_notice(&scope, &notice.id)
                })
                .await?;
        }
        Ok(())
    }
}
