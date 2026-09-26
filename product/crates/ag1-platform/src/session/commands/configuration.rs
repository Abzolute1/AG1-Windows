use super::*;
use ag1_core::runtime::session::{AppendAction, Configuration};
use serde_json::Value;

pub(super) fn catalogs(
    current: &Configuration,
    settings: &mut RunSettings,
) -> Result<(), DomainError> {
    if settings.models.is_empty() {
        settings.models.push(current.model.clone());
    }
    if settings.tools.is_empty() {
        settings.tools = current.tools.clone();
    }
    if settings.models.len() > 1024 {
        return Err(DomainError::LimitExceeded("model catalog".into()));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut candidate = current.clone();
    candidate.tools = settings.tools.clone();
    for model in &settings.models {
        candidate.model = model.clone();
        candidate.validate()?;
        if !identities.insert((
            model["provider"].clone().to_string(),
            model["id"].clone().to_string(),
        )) {
            return Err(DomainError::InvalidInput(
                "duplicate model catalog identity".into(),
            ));
        }
    }
    Ok(())
}

enum Change {
    Plan(
        Box<(
            Option<ag1_core::runtime::planning::Plan>,
            ag1_core::runtime::planning::Selection,
        )>,
    ),
    Model(Value),
    Thinking(String, Option<Value>),
    Context(u64),
    Tools(Vec<Value>),
    Task(Option<ag1_core::task::TaskState>),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Model {
    provider: String,
    model_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Thinking {
    level: String,
    #[serde(default)]
    native_profile: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tools {
    names: Vec<String>,
}
fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, w::ProtocolError> {
    serde_json::from_value(args.clone()).map_err(|_| w::ProtocolError::Malformed {
        detail: "invalid native control arguments".into(),
    })
}
impl SessionCommands {
    fn change(
        &self,
        command: &w::Command,
        timestamp: &str,
        id: &str,
    ) -> Result<Change, w::ProtocolError> {
        let mut next = self.runtime().configuration().map_err(domain)?;
        let change = match command {
            w::Command::Plan { action, args } => {
                Change::Plan(Box::new(self.plan_change(action, args, timestamp)?))
            }
            w::Command::Model { action, args } if action == "select" => {
                let selected: Model = parse(args)?;
                let model = self
                    .settings
                    .models
                    .iter()
                    .find(|m| m["provider"] == selected.provider && m["id"] == selected.model_id)
                    .ok_or(w::ProtocolError::NotFound)?
                    .clone();
                if model
                    .get("runtimeConflict")
                    .is_some_and(|v| v != false && !v.is_null())
                {
                    return Err(unavailable("model_runtime_conflict"));
                }
                next.model = model.clone();
                Change::Model(model)
            }
            w::Command::Model { action, args } if action == "context" => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Context {
                    tokens: u64,
                }
                let context: Context = parse(args)?;
                if context.tokens < 1024
                    || context.tokens > next.model["contextWindow"].as_u64().unwrap_or(0)
                {
                    return Err(w::ProtocolError::Malformed {
                        detail: "Context limit exceeds admitted model capacity".into(),
                    });
                }
                Change::Context(context.tokens)
            }
            w::Command::Model { action, args } if action == "thinking" => {
                let thinking: Thinking = parse(args)?;
                next.thinking = thinking.level.clone();
                Change::Thinking(thinking.level, thinking.native_profile)
            }
            w::Command::Runtime { action, args } if action == "tools" => {
                let selected: Tools = parse(args)?;
                let tools: Vec<_> = selected
                    .names
                    .iter()
                    .map(|name| {
                        self.settings
                            .tools
                            .iter()
                            .find(|t| t["name"] == *name)
                            .cloned()
                            .ok_or(w::ProtocolError::NotFound)
                    })
                    .collect::<Result<_, _>>()?;
                next.tools = tools.clone();
                Change::Tools(tools)
            }
            w::Command::Task { action, args } => {
                let mut args =
                    args.as_object()
                        .cloned()
                        .ok_or_else(|| w::ProtocolError::Malformed {
                            detail: "task arguments".into(),
                        })?;
                if args.contains_key("action") {
                    return Err(w::ProtocolError::Malformed {
                        detail: "duplicate task action".into(),
                    });
                }
                args.insert("action".into(), json!(action));
                let state = self.runtime().state().map_err(domain)?;
                let plan = ag1_core::tools::task_state::prepare(
                    state.branch(),
                    &Value::Object(args),
                    timestamp,
                    id,
                )
                .map_err(domain)?;
                Change::Task(plan.next)
            }
            _ => return Err(unavailable("native_control_action")),
        };
        next.validate().map_err(domain)?;
        Ok(change)
    }

    /// Executes on the administrative worker, never an async polling thread.
    /// An active native tool worker retains its own source-order append barrier.
    fn commit_control(&self, action: AppendAction) -> Result<(), DomainError> {
        let runtime = self.runtime();
        let mut wait = super::super::CommitWait {
            state: runtime.state.clone(),
            changed: runtime.changed.clone(),
            expected_revision: action.write().expected,
            received: false,
        };
        let owner = runtime
            .tool_owner
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        let action = if let Some(owner) = owner {
            match owner.enqueue_control(action) {
                Ok(receipt) => {
                    wait.received = true;
                    return receipt
                        .blocking_recv()
                        .unwrap_or(Err(DomainError::DurabilityUnknown))
                        .map(|_| ());
                }
                Err(action) => *action,
            }
        } else {
            action
        };
        let result = runtime
            .acknowledge_stamped(
                Ok(action.execute(runtime.store.as_ref())),
                &|_: &super::super::SessionEvent| {},
            )
            .map(|_| ());
        wait.received = true;
        result
    }

    pub(super) fn configuration_worker(
        &self,
        envelope: &w::CommandEnvelope,
        key: &str,
    ) -> Result<w::CommandResult, w::ProtocolError> {
        if self
            .book()
            .map_err(domain)?
            .active
            .as_ref()
            .is_some_and(|a| a.client_control)
        {
            return Err(unavailable("native_client_action_busy"));
        }
        let timestamp = i64::try_from(self.clock.now_ms())
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .ok_or_else(|| w::ProtocolError::Malformed {
                detail: "command clock".into(),
            })?
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let id = format!("control-{}", ag1_storage::digest(key.as_bytes()));
        let change = self.change(&envelope.command, &timestamp, &id)?;
        if !self.runtime().persistence_healthy().map_err(domain)? {
            return Err(unavailable("native_command_recovery"));
        }
        self.book()
            .map_err(domain)?
            .sequence
            .checked_add(1)
            .ok_or(w::ProtocolError::Capacity {
                resource: "event_sequence".into(),
            })?;
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
            let action = {
                let mut state = self.runtime().state().map_err(domain)?;
                if state.journal_revision() != envelope.expected_revision.get() {
                    return Err(w::ProtocolError::StaleContext);
                }
                match change {
                    Change::Plan(change) => {
                        Some(state.update_plan(&id, &id, &timestamp, change.0, change.1))
                    }
                    Change::Model(model) => Some(state.select_model(&id, &id, &timestamp, model)),
                    Change::Context(tokens) => {
                        Some(state.select_context(&id, &id, &timestamp, tokens))
                    }
                    Change::Thinking(level, profile) => {
                        Some(state.select_native_thinking(&id, &id, &timestamp, level, profile))
                    }
                    Change::Tools(tools) => Some(state.select_tools(&id, &id, &timestamp, tools)),
                    Change::Task(Some(task)) => {
                        Some(state.append_task_state(&id, &id, &timestamp, task))
                    }
                    Change::Task(None) => None,
                }
                .transpose()
                .map_err(domain)?
            };
            if let Some(action) = action {
                self.commit_control(action).map_err(domain)?;
            }
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
