//! Native tree navigation: drain, retain the abandoned trajectory, optionally
//! summarize through the shared scheduler, then atomically select and publish.
use super::*;
use ag1_core::{
    context::{ContextArtifactPort, compiler::Compilation},
    runtime::{session::CompactionCapture, tree},
    storage::StoragePort,
};
use ag1_storage::context::ContextStore;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TreeOptions {
    pub target: String,
    #[serde(default)]
    pub summarize: bool,
    pub custom_instructions: Option<String>,
    #[serde(default)]
    pub replace_instructions: bool,
    pub label: Option<String>,
}
pub struct TreeInput {
    pub command: String,
    pub timestamp: String,
    pub options: TreeOptions,
    pub reserve_tokens: u64,
    pub summary_timeout_ms: u64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeResult {
    pub editor_text: Option<String>,
    pub summary_entry: Option<Value>,
    pub leaf: Option<String>,
    pub journal_revision: u64,
}
impl TreeOptions {
    pub fn validate(&self) -> Result<(), DomainError> {
        ag1_core::OperationId::new(&self.target)?;
        if self
            .custom_instructions
            .as_ref()
            .is_some_and(|s| s.len() > 1024 * 1024)
            || self.label.as_ref().is_some_and(|s| s.len() > 65536)
        {
            return Err(DomainError::LimitExceeded("tree options".into()));
        }
        Ok(())
    }
}
impl NativeSession {
    /// Accepted navigation is retained by the host even when a client detaches.
    pub async fn navigate_tree(
        self: &Arc<Self>,
        input: TreeInput,
        resolver: Arc<dyn ProviderResolver>,
        tools: Arc<dyn ToolRunner>,
        listener: Arc<dyn Listener>,
    ) -> Result<TreeResult, DomainError> {
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _activity = activity;
            let result = runtime
                .navigate_tree_for_command(
                    input,
                    resolver.as_ref(),
                    tools,
                    Arc::new(AtomicBool::new(false)),
                    listener.as_ref(),
                )
                .await;
            let _ = send.send(result);
        });
        receive.await.map_err(|_| DomainError::DurabilityUnknown)?
    }
    pub(super) async fn navigate_tree_for_command(
        &self,
        input: TreeInput,
        resolver: &dyn ProviderResolver,
        tools: Arc<dyn ToolRunner>,
        cancelled: Arc<AtomicBool>,
        listener: &dyn Listener,
    ) -> Result<TreeResult, DomainError> {
        let _activity = self.enter()?;
        input.options.validate()?;
        ag1_core::OperationId::new(&input.command)?;
        if chrono::DateTime::parse_from_rfc3339(&input.timestamp).is_err()
            || input.summary_timeout_ms == 0
            || input.summary_timeout_ms > 86_400_000
            || input.reserve_tokens > 1_000_000_000
        {
            return Err(DomainError::InvalidInput("tree options".into()));
        }
        {
            let state = self.state()?;
            if state
                .branch()
                .last()
                .is_some_and(|e| e["id"] == input.options.target)
            {
                return Ok(TreeResult {
                    editor_text: None,
                    summary_entry: None,
                    leaf: Some(input.options.target),
                    journal_revision: state.journal_revision(),
                });
            }
        }
        let (source, mut cancel) = crate::provider::http::cancellation();
        let token = {
            let mut compacting = self.compacting.lock().map_err(|_| DomainError::Conflict)?;
            if compacting.is_some() {
                return Err(DomainError::Conflict);
            }
            let mut state = self.state()?;
            let token = state.reserve_compaction()?;
            *compacting = Some(ActiveTurn {
                cancelled: cancelled.clone(),
                source: Arc::new(source),
            });
            token
        };
        let _lease = super::compaction::CompactLease {
            session: self,
            token: token.clone(),
        };
        if let Some(parent) = self
            .active
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
        {
            parent.cancelled.store(true, Ordering::Release);
            parent.source.cancel();
            self.changed.notify_waiters();
        }
        let capture = loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if cancelled.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            match self.state()?.capture_compaction(&token) {
                Ok(capture) => break capture,
                Err(DomainError::Unavailable(_)) => {}
                Err(error) => return Err(error),
            }
            changed.await;
        };
        let session = self.state()?.stamp().session().clone();
        let store = self.store.clone();
        let origin = self.origin.clone();
        let target = input.options.target.clone();
        let operation = format!(
            "tree-{}-{}",
            self.incarnation,
            ag1_storage::digest(input.command.as_bytes())
        );
        let op = operation.clone();
        let timestamp = input.timestamp.clone();
        let activity = self.retain_work()?;
        let (journal,navigation,trajectory)=self.pool.submit(move ||{
            let _activity=activity;
            let journal=store.journal_snapshot(&session)?;
            let navigation=tree::prepare(&journal,&target)?;
            let mut artifacts=ContextStore::new(&store,session,&origin,&op,&timestamp,Default::default())?;
            let raw=journal.entries.iter().map(|e|std::str::from_utf8(&e.raw).map_err(|_|DomainError::InvalidInput("journal UTF-8".into()))).collect::<Result<Vec<_>,_>>()?;
            let trajectory=artifacts.put_evidence(&json!({"version":1,"kind":"branch-trajectory","journalRevision":journal.revision,"oldLeafId":journal.leaf,"rawEntries":raw,"targetId":target,"entries":navigation.entries}).to_string(),"runtime-branch-trajectory",&json!({"type":"session","name":"tree-navigation"}),&[],None)?;
            Ok((journal,navigation,trajectory))
        }).await?;
        self.check_tree_capture(&capture)?;
        if journal.revision != capture.journal_revision() {
            return Err(DomainError::Conflict);
        }
        let mut options = input.options.clone();
        let event = json!({"type":"session_before_tree","preparation":{"targetId":options.target,"oldLeafId":journal.leaf,"commonAncestorId":navigation.common_ancestor,"entriesToSummarize":navigation.entries,"userWantsSummary":options.summarize,"customInstructions":options.custom_instructions,"replaceInstructions":options.replace_instructions,"label":options.label}});
        let callback = self
            .request_callback(
                tools.clone(),
                event,
                cancelled.clone(),
                &format!("{operation}-before"),
                &input.timestamp,
            )
            .await?;
        if callback.as_ref().is_some_and(|r| r["cancel"] == true) {
            return Err(DomainError::Cancelled);
        }
        if let Some(result) = &callback {
            for (key, slot) in [
                ("customInstructions", &mut options.custom_instructions),
                ("label", &mut options.label),
            ] {
                if let Some(value) = result.get(key) {
                    *slot = Some(
                        value
                            .as_str()
                            .ok_or_else(|| {
                                DomainError::InvalidInput("tree callback options".into())
                            })?
                            .into(),
                    );
                }
            }
            if let Some(value) = result.get("replaceInstructions") {
                options.replace_instructions = value
                    .as_bool()
                    .ok_or_else(|| DomainError::InvalidInput("tree callback options".into()))?;
            }
        }
        options.validate()?;
        let override_summary = callback
            .as_ref()
            .and_then(|r| r.get("summary"))
            .filter(|_| options.summarize);
        let from_extension = override_summary.is_some();
        let summary = if let Some(result) = override_summary {
            Some(result.clone())
        } else if options.summarize && !navigation.entries.is_empty() {
            let window = capture.configuration().context_limit();
            let prepared = tree::summary(
                &navigation.entries,
                window.min(i64::MAX as u64) as i64 - input.reserve_tokens as i64,
            )?;
            if prepared.messages.is_empty() {
                Some(json!({"summary":"No content to summarize"}))
            } else {
                let request = prepared.request(
                    options.custom_instructions.as_deref(),
                    options.replace_instructions,
                    self.state()?.stamp().session().as_str(),
                    capture.configuration().model["maxTokens"]
                        .as_u64()
                        .unwrap_or(0),
                )?;
                let (text, output) = self
                    .tree_summary(
                        summary::Request {
                            capture: &capture,
                            input: &input,
                            operation: &operation,
                            request,
                            cancelled: &cancelled,
                            cancel: &mut cancel,
                        },
                        resolver,
                    )
                    .await?;
                let mut result = prepared.finish(&text);
                result["details"]["summaryOutput"] = json!(output);
                Some(result)
            }
        } else {
            None
        };
        if cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        self.check_tree_capture(&capture)?;
        let mut records = vec![];
        if let Some(summary) = summary {
            if summary["summary"]
                .as_str()
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(DomainError::InvalidInput("tree summary".into()));
            }
            records.push(json!({"type":"branch_summary","id":operation,"timestamp":input.timestamp,"fromId":journal.leaf,"summary":summary["summary"],"fromHook":from_extension,"details":summary.get("details").cloned().unwrap_or(Value::Null),"nativeTrajectory":trajectory}));
        }
        if let Some(label) = options.label.filter(|label| !label.is_empty()) {
            records.push(json!({"type":"label","id":format!("{operation}-label"),"timestamp":input.timestamp,"targetId":if records.is_empty(){options.target.as_str()}else{operation.as_str()},"label":label}));
        }
        let summary_id = summary_marker(&records);
        let action = self.state()?.navigate_tree(
            &capture,
            &journal,
            &options.target,
            &input.command,
            records,
        )?;
        self.commit_branch(action, listener).await?;
        let (leaf, summary_entry, journal_revision) = {
            let state = self.state()?;
            (
                state
                    .branch()
                    .last()
                    .and_then(|e| e["id"].as_str())
                    .map(str::to_owned),
                summary_id.and_then(|id| state.branch().iter().find(|e| e["id"] == id).cloned()),
                state.journal_revision(),
            )
        };
        let event = json!({"type":"session_tree","newLeafId":leaf,"oldLeafId":journal.leaf,"summaryEntry":summary_entry,"fromExtension":summary_entry.as_ref().map(|_|from_extension)});
        if let Err(error) = self
            .request_callback(
                tools,
                event,
                cancelled,
                &format!("{operation}-after"),
                &input.timestamp,
            )
            .await
        {
            Self::emit(
                listener,
                SessionEvent::Recovery(
                    json!({"type":"extension_notification_error","event":"session_tree","error":format!("{error:?}")}),
                ),
            );
        }
        Ok(TreeResult {
            editor_text: navigation.editor_text,
            summary_entry,
            leaf,
            journal_revision,
        })
    }
    fn check_tree_capture(&self, capture: &CompactionCapture) -> Result<(), DomainError> {
        let state = self.state()?;
        state.check_compaction_capture(capture)?;
        if state.journal_revision() != capture.journal_revision() {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}
fn summary_marker(records: &[Value]) -> Option<String> {
    records
        .iter()
        .find(|e| e["type"] == "branch_summary")
        .and_then(|e| e["id"].as_str())
        .map(str::to_owned)
}

mod summary;
