//! Host session operations above atomic bootstrap and durable selection. Wire
//! admission must persist the request before invoking this execution boundary.
use super::*;
use ag1_core::OperationId;
use ag1_storage::{SessionBootstrap, SessionSeed};
use serde_json::{Value, json};
use std::io::Read;

#[derive(Clone, Debug)]
pub enum SessionAction {
    New,
    Resume,
    Fork { entry: String },
}
/// Stable host-assigned command/session IDs and timestamp. Reuse this request
/// on bootstrap acknowledgement recovery; never generate a different target
/// merely because a response was lost. A wire payload supplies no cursor key.
#[derive(Clone, Debug)]
pub struct SessionRequest {
    pub command: String,
    pub target: SessionId,
    pub expected_revision: u64,
    pub timestamp: String,
    pub action: SessionAction,
}
pub struct TransitionOutcome {
    pub selected: Selection,
    /// Forked user text goes back to the input editor. It is not automatically
    /// appended or sent to the model in the new branch.
    pub selected_text: Option<String>,
}
enum Seed {
    Resume,
    New {
        header: Vec<u8>,
        records: Vec<Vec<u8>>,
    },
    Fork {
        leaf: Option<String>,
    },
}
struct BootstrapFactory<'a> {
    source: Selection,
    request: SessionRequest,
    seed: Seed,
    delegate: &'a dyn RuntimeFactory,
}
impl RuntimeFactory for BootstrapFactory<'_> {
    fn prepare<'a>(&'a self, target: &'a Target, epoch: u64) -> PortFuture<'a, PreparedRuntime> {
        Box::pin(async move {
            let runtime = self.source.services.runtime();
            let store = runtime.store.clone();
            let parent = runtime.state()?.stamp().session().as_str().to_owned();
            let request = self.request.clone();
            let seed = match &self.seed {
                Seed::Resume => Seed::Resume,
                Seed::New { header, records } => Seed::New {
                    header: header.clone(),
                    records: records.clone(),
                },
                Seed::Fork { leaf } => Seed::Fork { leaf: leaf.clone() },
            };
            let cwd = runtime.root.root().to_owned();
            runtime
                .pool
                .submit(move || {
                    if let Seed::Resume = seed {
                        let saved = store.journal_snapshot(&request.target)?;
                        let header: Value = serde_json::from_slice(&saved.header)
                            .map_err(|_| DomainError::InvalidInput("session header".into()))?;
                        if header["cwd"].as_str().map(std::path::Path::new) != Some(cwd.as_path()) {
                            return Err(DomainError::PermissionDenied);
                        }
                        return Ok(());
                    }
                    let mut key = [0u8; 32];
                    std::fs::File::open("/dev/urandom")
                        .and_then(|mut file| file.read_exact(&mut key))
                        .map_err(crate::io_error)?;
                    let seed = match &seed {
                        Seed::New { header, records } => SessionSeed::Fresh { header, records },
                        Seed::Fork { leaf } => SessionSeed::RuntimeFork {
                            parent: &parent,
                            expected_revision: request.expected_revision,
                            leaf: leaf.as_deref(),
                            timestamp: &request.timestamp,
                        },
                        Seed::Resume => unreachable!(),
                    };
                    store.bootstrap_session(SessionBootstrap {
                        command: &request.command,
                        session: request.target.as_str(),
                        origin: request.target.as_str(),
                        cursor_key: &key,
                        seed,
                    })?;
                    Ok(())
                })
                .await?;
            self.delegate.prepare(target, epoch).await
        })
    }
}
impl RuntimeRegistry {
    /// Prepare a new/resumed/forked session, deliberately cancel unstarted old
    /// prompts, drain prior ownership and select the prepared runtime. Caller
    /// loss never replays effects; pending_selection/reconcile_selection handle
    /// the durable cutover separately from command outcome publication.
    pub async fn change_session(
        &self,
        expected_epoch: u64,
        request: SessionRequest,
        factory: &dyn RuntimeFactory,
    ) -> Result<TransitionOutcome, DomainError> {
        let _replacement = self
            .replacing
            .try_lock()
            .map_err(|_| DomainError::Conflict)?;
        OperationId::new(&request.command)?;
        if request.timestamp.len() > 64
            || chrono::DateTime::parse_from_rfc3339(&request.timestamp).is_err()
        {
            return Err(DomainError::InvalidInput("session timestamp".into()));
        }
        let source = self.selected()?;
        if source.epoch != expected_epoch {
            return Err(DomainError::Conflict);
        }
        let runtime = source.services.runtime();
        let event = match &request.action {
            SessionAction::New => json!({"type":"session_before_switch","reason":"new"}),
            SessionAction::Resume => {
                json!({"type":"session_before_switch","reason":"resume","targetSessionId":request.target.as_str()})
            }
            SessionAction::Fork { entry } => json!({"type":"session_before_fork","entryId":entry}),
        };
        if runtime
            .lifecycle_event(source.services.tools.clone(), event)
            .await?
            .is_some_and(|value| value["cancel"] == true)
        {
            return Err(DomainError::Cancelled);
        }
        let (session, configuration) = {
            let state = runtime.state()?;
            if state.journal_revision() != request.expected_revision {
                return Err(DomainError::Conflict);
            }
            (
                state.stamp().session().clone(),
                state.configuration().clone(),
            )
        };
        if !matches!(request.action, SessionAction::Resume) && request.target == session {
            return Err(DomainError::Conflict);
        }
        let (seed, selected_text) = match &request.action {
            SessionAction::Resume => (Seed::Resume, None),
            SessionAction::New => {
                let header = serde_json::to_vec(&json!({"type":"session","version":3,"id":request.target.as_str(),"cwd":runtime.root.root(),"timestamp":request.timestamp}))
                    .map_err(|_|DomainError::Conflict)?;
                let model_id = format!("model-{}", ag1_storage::digest(request.command.as_bytes()));
                let thinking_id = format!(
                    "thinking-{}",
                    ag1_storage::digest(request.command.as_bytes())
                );
                let records = [
                    json!({"type":"model_change","id":model_id,"parentId":null,"timestamp":request.timestamp,"provider":configuration.model["provider"],"modelId":configuration.model["id"]}),
                    json!({"type":"thinking_level_change","id":thinking_id,"parentId":model_id,"timestamp":request.timestamp,"thinkingLevel":configuration.thinking}),
                ].iter().map(serde_json::to_vec).collect::<Result<Vec<_>,_>>().map_err(|_|DomainError::Conflict)?;
                (Seed::New { header, records }, None)
            }
            SessionAction::Fork { entry } => {
                OperationId::new(entry)?;
                let store = runtime.store.clone();
                let saved = runtime
                    .pool
                    .submit(move || store.journal_snapshot(&session))
                    .await?;
                if saved.revision != request.expected_revision {
                    return Err(DomainError::Conflict);
                }
                let row = saved
                    .entries
                    .iter()
                    .find(|row| &row.id == entry)
                    .ok_or(DomainError::NotFound)?;
                let value: Value =
                    serde_json::from_slice(&row.normalized).map_err(|_| DomainError::Conflict)?;
                if value["type"] != "message" || value["message"]["role"] != "user" {
                    return Err(DomainError::InvalidInput(
                        "fork requires a user entry".into(),
                    ));
                }
                let content = &value["message"]["content"];
                let text = match content {
                    Value::String(text) => text.clone(),
                    Value::Array(blocks) => blocks
                        .iter()
                        .filter(|b| b["type"] == "text")
                        .filter_map(|b| b["text"].as_str())
                        .collect::<String>(),
                    _ => return Err(DomainError::InvalidInput("fork user content".into())),
                };
                (
                    Seed::Fork {
                        leaf: row.parent.clone(),
                    },
                    Some(text),
                )
            }
        };
        let target = Target {
            workspace: runtime.workspace.clone(),
            session: request.target.clone(),
        };
        let revision = request.expected_revision;
        let preparation = BootstrapFactory {
            source,
            request,
            seed,
            delegate: factory,
        };
        let selected = self
            .replace_locked(expected_epoch, target, &preparation, true, Some(revision))
            .await?;
        Ok(TransitionOutcome {
            selected,
            selected_text,
        })
    }
}
