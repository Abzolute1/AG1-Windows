use super::*;
use ag1_core::storage::StoragePort;
fn bounded(value: Value, limit: usize) -> Value {
    let text = value.to_string();
    if text.len() <= limit {
        value
    } else {
        json!({"display":clipped(&text,limit),"bytes":text.len(),"paged":true})
    }
}

fn clipped(text: &str, limit: usize) -> String {
    let end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= limit)
        .last()
        .unwrap_or(0);
    if text.len() <= limit {
        text.into()
    } else {
        format!("{}… [open journal page]", &text[..end])
    }
}
pub fn message(entry: &Value) -> Option<w::ClientMessage> {
    if entry["type"] != "message" {
        return None;
    }
    let m = &entry["message"];
    let mut blocks = if let Some(text) = m["content"].as_str() {
        vec![w::MessageBlock::Text {
            text: clipped(text, 2048),
        }]
    } else {
        m["content"]
            .as_array()
            .into_iter()
            .flatten()
            .take(8)
            .map(|b| match b["type"].as_str() {
                Some("text") => w::MessageBlock::Text {
                    text: clipped(b["text"].as_str().unwrap_or(""), 2048),
                },
                Some("thinking") => w::MessageBlock::Thinking {
                    text: clipped(b["thinking"].as_str().unwrap_or(""), 2048),
                },
                Some("toolCall") => w::MessageBlock::Tool {
                    name: clipped(b["name"].as_str().unwrap_or(""),128),
                    call_id: clipped(b["id"].as_str().unwrap_or(""),128),
                    arguments: if b["arguments"].to_string().len() < 2048 {
                        b["arguments"].clone()
                    } else {
                        json!({"display":"Open journal page for full arguments"})
                    },
                },
                Some("image") => w::MessageBlock::Attachment {
                    media_type: clipped(b["mimeType"].as_str().unwrap_or("image/unknown"),128),
                    reference: entry["id"].as_str().unwrap_or("").into(),
                },
                _ => w::MessageBlock::Unknown {
                    value: json!({"type":clipped(b["type"].as_str().unwrap_or("unknown"),128),"display":"Preserved in journal"}),
                },
            })
            .collect()
    };
    if m["role"] == "toolResult" && m["toolName"].as_str().is_some_and(|n| n.starts_with("mcp_")) {
        let details = &m["details"]["mcp"];
        if let Ok(identity) = serde_json::from_value::<ag1_core::tools::mcp::Identity>(details["source"].clone()) {
            if identity.wire_name() == m["toolName"].as_str().unwrap_or("") {
                let original = serde_json::from_value::<ag1_core::context::ContextArtifact>(details["originalResponse"].clone()).ok();
                blocks.insert(0,w::MessageBlock::Unknown { value:json!({"kind":"mcp_result","server":identity.binding.server_id,"capability":clipped(&identity.original_name,256),"outcome":clipped(details["outcome"].as_str().unwrap_or("unknown"),64),"complete":details["completeResponseRetained"]==true,"original":original,"references":bounded(details["references"].clone(),2048)}) });
            }
        }
        // A mixed result may place its first image after the eight preview rows.
        // One authenticated journal reference loads all bounded native rasters.
        if !blocks.iter().any(|b|matches!(b,w::MessageBlock::Attachment{..})) {
            if let Some(image) = m["content"].as_array().into_iter().flatten().take(1024).find(|b|b["type"]=="image") {
                blocks.insert(0,w::MessageBlock::Attachment{media_type:clipped(image["mimeType"].as_str().unwrap_or("image/unknown"),128),reference:entry["id"].as_str().unwrap_or("").into()});
            }
        }
    }
    // Terminal provider failures must remain visible after polling/reopening.
    // This is a display projection only; the durable model transcript is untouched.
    if m["role"] == "assistant" && matches!(m["stopReason"].as_str(), Some("error" | "aborted")) {
        blocks.retain(|b| !matches!(b, w::MessageBlock::Thinking { text } if text.trim().is_empty()));
        blocks.insert(0, w::MessageBlock::Unknown { value: json!({"kind":"provider_error","cancelled":m["stopReason"]=="aborted","message":clipped(m["errorMessage"].as_str().filter(|s| !s.is_empty()).unwrap_or(if m["stopReason"]=="aborted" {"The response was stopped."} else {"The model response ended with an error."}),512)}) });
    }
    let mut message = w::ClientMessage {
        id: entry["id"].as_str().unwrap_or("").into(),
        role: clipped(m["role"].as_str().unwrap_or("unknown"), 128),
        blocks,
        tool: (m["role"] == "toolResult").then(|| w::ToolResultIdentity {
            call_id: clipped(m["toolCallId"].as_str().unwrap_or(""), 128),
            name: clipped(m["toolName"].as_str().unwrap_or(""), 128),
            failed: m["isError"].as_bool().unwrap_or(false),
        }),
    };
    if serde_json::to_vec(&message).ok()?.len() > 6000 {
        for block in &mut message.blocks {
            match block {
                w::MessageBlock::Text { text } | w::MessageBlock::Thinking { text } => {
                    *text = clipped(text, 256)
                }
                w::MessageBlock::Tool { arguments, .. } => {
                    *arguments = json!({"display":"Open journal page for full arguments"})
                }
                _ => {}
            }
        }
        while message.blocks.len() > 1 && serde_json::to_vec(&message).ok()?.len() > 6000 {
            message.blocks.pop();
        }
    }
    Some(message)
}
impl Host {
    /// Read-only resnapshot across a concurrent registry publication. A client
    /// must not treat a transient selection fence as failure of its accepted action.
    pub async fn settled_view(&self) -> Result<w::ClientView> {
        for _ in 0..200 {
            match self.view() {
                Err(DomainError::Conflict | DomainError::QueueFull) => {
                    tokio::time::sleep(Duration::from_millis(10)).await
                }
                Err(DomainError::Unavailable(_))
                    if self.commands.client_transition_pending()?
                        && !self.commands.recovery_required()? =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                result => return result,
            }
        }
        self.view()
    }
    pub fn view(&self) -> Result<w::ClientView> {
        // Retry if a lifecycle command publishes a different session while a
        // projection is being assembled; never mix scope with another journal.
        for _ in 0..3 {
            let snapshot = self.commands.snapshot()?;
            let selected = match self.registry.selected() {
                Ok(selected) => selected,
                Err(error) if self.commands.recovery_required()? => {
                    return self.recovery_view(snapshot, format!("{error:?}"));
                }
                Err(error) => return Err(error),
            };
            let runtime = selected.services.runtime();
            let records = runtime.client_records()?;
            let configuration = runtime.configuration()?;
            let pending = self.commands.pending_approvals()?;
            let after = self.commands.snapshot()?;
            if snapshot.scope != after.scope || snapshot.revision != after.revision {
                continue;
            }
            let all: Vec<_> = records.iter().filter_map(message).collect();
            let mut messages = Vec::new();
            let mut budget = 8000;
            for m in all.iter().rev().take(100) {
                let bytes = serde_json::to_vec(m).map_err(json_error)?.len();
                if bytes > budget {
                    break;
                }
                budget -= bytes;
                messages.push(m.clone());
            }
            messages.reverse();
            let bridge_messages = messages
                .iter()
                .map(|m| w::BridgeMessage {
                    id: m.id.clone(),
                    role: m.role.clone(),
                    text: m
                        .blocks
                        .iter()
                        .filter_map(|b| {
                            if let w::MessageBlock::Text { text } = b {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                    evidence: m.id.clone(),
                })
                .collect();
            let pending = pending
                .into_iter()
                .take(1)
                .map(|p| w::BridgeQuestion {
                    binding: p.binding,
                    title: p.display.to_string(),
                    kind: "approval".into(),
                    options: vec![],
                    evidence: String::new(),
                    truncated: false,
                })
                .collect::<Vec<_>>();
            let task = records
                .iter()
                .rev()
                .find(|r| r["customType"] == "task_state")
                .map(|r| r["data"].clone())
                .unwrap_or(Value::Null);
            let context = runtime.client_context()?;
            let artifacts = self
                .store
                .evidence_catalog(snapshot.scope.session.as_str(), 20)?
                .into_iter()
                .map(|r| bounded(r.record, 1024))
                .collect();
            let memory=self.store.load_memory(self.workspace.id().as_str())?.map(|d|json!({"sha256":d.sha256,"diagnostic":d.diagnostic,"bytes":d.bytes.len(),"text":clipped(&String::from_utf8_lossy(&d.bytes),4096)})).unwrap_or(Value::Null);
            let failure = self
                .failure
                .lock()
                .unwrap()
                .clone()
                .or(if snapshot.queued.is_empty() {
                    self.commands.last_failure()?.filter(|e| e != "Cancelled")
                } else {
                    None
                });
            let status = if !runtime.persistence_healthy()? {
                "persistence_failure"
            } else if self.commands.recovery_required()? {
                "recovery_required"
            } else if !pending.is_empty() {
                "awaiting_input"
            } else if !snapshot.queued.is_empty() {
                "streaming"
            } else {
                "idle"
            };
            let (live_revision, live_observations) = {
                let observations = self
                    .observations
                    .lock()
                    .map_err(|_| DomainError::Conflict)?;
                (
                    self.live_revision.load(Ordering::Acquire),
                    observations.view(&snapshot.scope),
                )
            };
            let details=w::ClientDetails {authority:"rust-isolated".into(),messages,task:bounded(task,4096),context:json!({"thinking":configuration.thinking,"selected_model":{"provider":configuration.model["provider"],"id":configuration.model["id"]},"approval_policy":self.approval_policy.snapshot()?,"folder_access":self.folder_selection().ok().map(|f|f.selection),"model_context_limit":configuration.model["contextWindow"],"working_context_limit":if configuration.compiler.context_window==0 {configuration.model["contextWindow"].as_u64().unwrap_or(0)}else{configuration.compiler.context_window},"journal_revision":snapshot.revision,"last_compilation":bounded(context,2048),"measurement":"Prepared compilation statistics; source freshness belongs to the recorded revision"}),artifacts,memory,activity:json!({"inference_waiting":runtime.waiting_for_inference(),"runtime_preparation":self.local_runtime.as_ref().and_then(|r| r.progress()),"live_revision":live_revision,"observations":live_observations,"background":bounded(selected.services.tools().client_activity()?,2048),"latest_assistant_usage":latest_usage(&records),"extension_ui":self.ui.view()?,"queued":snapshot.queued,"last_terminal":snapshot.last_terminal,"recovery_required":self.commands.recovery_required()?,"leases":bounded(json!(self.store.evidence_lease_records(snapshot.scope.session.as_str())?),2048)}),models:self.config.models.iter().take(8).map(|m|json!({"id":m["id"],"provider":m["provider"],"api":m["api"],"contextWindow":m["contextWindow"],"runtime":m["runtime"],"conflict":m["runtimeConflict"]})).collect(),capabilities:self.commands.handshake()?.capabilities};
            let mut view = w::ClientView {
                snapshot: w::BridgeSnapshot {
                    version: Default::default(),
                    scope: snapshot.scope,
                    revision: snapshot.revision,
                    watermark: snapshot.watermark,
                    workspace_path: self.workspace.root().display().to_string(),
                    status: status.into(),
                    active_operation: snapshot.queued.first().cloned(),
                    messages: bridge_messages,
                    message_count: all.len() as u32,
                    pending,
                    model: configuration.model["id"]
                        .as_str()
                        .unwrap_or("unknown")
                        .into(),
                    diagnostic: failure.clone(),
                    session_file: None,
                    fixture_provider: self.config.fixture,
                },
                error: failure,
                progress: self.progress.lock().unwrap().clone(),
                details,
            };
            while serde_json::to_vec(&view).map_err(json_error)?.len() > w::MAX_ENVELOPE - 64 {
                if !view.details.messages.is_empty() {
                    view.details.messages.remove(0);
                    view.snapshot.messages.remove(0);
                } else if !view.details.artifacts.is_empty() {
                    view.details.artifacts.pop();
                } else if !view.details.models.is_empty() {
                    view.details.models.pop();
                } else {
                    view.progress.clear();
                    view.details.task = json!({"paged":true});
                    view.details.context = json!({"paged":true});
                    view.details.memory = json!({"paged":true});
                    view.details.activity = json!({"paged":true});
                    if serde_json::to_vec(&view).map_err(json_error)?.len() > w::MAX_ENVELOPE - 64 {
                        return Err(DomainError::LimitExceeded("client view".into()));
                    }
                }
            }
            let final_snapshot = self.commands.snapshot()?;
            if final_snapshot.scope != view.snapshot.scope
                || final_snapshot.revision != view.snapshot.revision
            {
                continue;
            }
            *self.last_view.lock().map_err(|_| DomainError::Conflict)? = Some(view.clone());
            return Ok(view);
        }
        Err(DomainError::Conflict)
    }
}

impl Host {
    fn recovery_view(&self, snapshot: w::Snapshot, error: String) -> Result<w::ClientView> {
        let mut view = self
            .last_view
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .clone()
            .ok_or(DomainError::DurabilityUnknown)?;
        if view.snapshot.scope != snapshot.scope {
            return Err(DomainError::DurabilityUnknown);
        }
        view.snapshot.revision = snapshot.revision;
        view.snapshot.watermark = snapshot.watermark;
        view.snapshot.status = "recovery_required".into();
        view.snapshot.active_operation = snapshot.queued.first().cloned();
        view.snapshot.pending.clear();
        view.snapshot.diagnostic = Some(error.clone());
        view.error = Some(error.clone());
        view.progress.clear();
        view.details.task = json!({"unavailable":error});
        view.details.context =
            json!({"unavailable":"Selection is fenced; reconcile the stored acknowledgement"});
        view.details.memory = Value::Null;
        view.details.artifacts.clear();
        view.details.models.clear();
        view.details.activity = json!({"recovery_required":true,"queued":snapshot.queued,"last_terminal":snapshot.last_terminal,"conversation_projection":"Last acknowledged view; other projections are unavailable until recovery"});
        Ok(view)
    }
}

#[cfg(test)]
mod tool_result_projection_tests {
    use super::*;
    #[test]
    fn preserves_execution_identity_and_failure_without_parsing_result_text() {
        let projected = message(&json!({"type":"message","id":"r","message":{"role":"toolResult","toolCallId":"call-b","toolName":"read","isError":true,"content":[{"type":"text","text":"<html>\n    source\n</html>"}]}})).unwrap();
        let identity = projected.tool.unwrap();
        assert_eq!(identity.call_id, "call-b");
        assert_eq!(identity.name, "read");
        assert!(identity.failed);
        assert!(
            matches!(&projected.blocks[0], w::MessageBlock::Text { text } if text == "<html>\n    source\n</html>")
        );
    }
}

// A zero-usage cancellation is a terminal marker, not a replacement measurement.
fn latest_usage(records: &[Value]) -> Option<Value> {
    records.iter().rev().take_while(|r| r["message"]["role"] != "user").find(|r| {
        let m=&r["message"];
        m["role"] == "assistant" && (!matches!(m["stopReason"].as_str(),Some("aborted"|"error")) || ["input","output","cacheRead","cacheWrite"].iter().any(|key|m["usage"][key].as_u64().unwrap_or(0)>0))
    }).map(|r|json!({"entry":r["id"],"usage":r["message"]["usage"],"stopReason":r["message"]["stopReason"]}))
}
#[cfg(test)]
mod cancelled_usage_tests {
    use super::*;
    #[test]
    fn cancellation_preserves_reported_usage_only_within_its_turn() {
        let user = json!({"message":{"role":"user"}});
        let measured = json!({"id":"measured","message":{"role":"assistant","stopReason":"toolUse","usage":{"input":12,"output":4}}});
        let cancelled = json!({"message":{"role":"assistant","stopReason":"aborted","usage":{"input":0,"output":0}}});
        assert_eq!(
            latest_usage(&[user.clone(), measured.clone(), cancelled.clone()]).unwrap()["usage"]["input"],
            12
        );
        assert!(latest_usage(&[measured, user, cancelled]).is_none());
    }
}

#[cfg(test)]
mod provider_failure_tests {
    use super::*;
    #[test]
    fn provider_errors_and_cancellations_survive_display_projection() {
        for reason in ["error", "aborted"] {
            let entry=json!({"id":"failed","type":"message","message":{"role":"assistant","stopReason":reason,"errorMessage":"unknown Responses stream event","content":[{"type":"thinking","thinking":""}]}});
            let projected=message(&entry).unwrap();
            assert_eq!(projected.blocks.len(),1);
            let w::MessageBlock::Unknown{value}=&projected.blocks[0] else {panic!("failure missing")};
            assert_eq!(value["kind"],"provider_error");
            assert_eq!(value["cancelled"],reason=="aborted");
            assert_eq!(entry["message"]["content"][0]["type"],"thinking");
        }
    }
}
