//! Single mutation owner for a durable branch. Host actions can leave the owner
//! while blocking storage or asynchronous context work runs. Only opaque receipts
//! advance the committed view; preparations capture immutable request inputs.
use super::boundary::{Append, Boundary, Compaction, Generation, Preparation, Stamp};
use crate::{
    DomainError, SessionId,
    context::{
        compaction::context_messages,
        compiler::{Compilation, CompileOptions, ContextSnapshot},
        estimate_tokens, validate_history,
    },
    storage::StoredJournal,
    task::{
        JournalWritePort, PreparedStateCommit, bounded_json, bounded_json_with_limit, branch_values,
        invalid, units,
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};
mod plans;

// Long local-model reasoning is retained in native custody and must be able to
// cross the small default state-record budget without making state unbounded.
const LONG_ASSISTANT_STATE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct PendingCall {
    pub id: String,
    pub name: String,
    /// Retain the complete provider block, including malformed-argument markers.
    /// P09 preflight, rather than this journal owner, validates executable args.
    pub block: Value,
}
impl PendingCall {
    /// The journal independently derives canonical inputs, never trusting a
    /// renderer's or adapter's declaration of which state it may update.
    pub fn effective(&self, branch: &[Value]) -> Result<Self, DomainError> {
        let mut call = self.clone();
        if let Some((name, args)) = crate::tools::surface::route(&self.name, &self.block["arguments"], branch)? {
            call.name = name.clone(); call.block["name"] = serde_json::json!(name); call.block["arguments"] = args;
        }
        Ok(call)
    }
}
fn calls(message: &Value) -> Result<Vec<PendingCall>, DomainError> {
    let mut result = vec![];
    let mut ids = BTreeSet::new();
    for block in message["content"].as_array().into_iter().flatten() {
        if block["type"] != "toolCall" {
            continue;
        }
        let id = block["id"]
            .as_str()
            .ok_or_else(|| invalid("tool call id"))?;
        let name = block["name"]
            .as_str()
            .ok_or_else(|| invalid("tool call name"))?;
        if id.is_empty()
            || id.len() > 128
            || id.chars().any(char::is_control)
            || !ids.insert(id)
            || name.is_empty()
            || name.len() > 128
            || name.chars().any(char::is_control)
        {
            return Err(invalid("tool call identity"));
        }
        result.push(PendingCall {
            id: id.into(),
            name: name.into(),
            block: block.clone(),
        });
        if result.len() > 128 {
            return Err(DomainError::LimitExceeded("runtime tool batch".into()));
        }
    }
    Ok(result)
}
enum ToolTransition {
    Keep,
    Start(Vec<PendingCall>),
    Finish(String),
}

/// Resolved host configuration, with no credential or process ownership. Tools
/// are public SDK schemas. The host resolves the journal's selected model before
/// resuming; a different resolved model is rejected instead of silently selected.
#[derive(Clone, Debug)]
pub struct Configuration {
    pub model: Value,
    pub thinking: String,
    pub tools: Vec<Value>,
    pub system_prompt: String,
    pub prompt: Option<Arc<super::prompt::PromptConfiguration>>,
    pub compiler: CompileOptions,
}
impl Configuration {
    pub fn context_limit(&self) -> u64 {
        let maximum = self.model["contextWindow"].as_u64().unwrap_or(0);
        if self.compiler.context_window == 0 {
            maximum
        } else {
            self.compiler.context_window.min(maximum)
        }
    }

    /// A validated request-local tool view with its resource prompt rebuilt.
    pub fn with_tools(&self, tools: Vec<Value>) -> Result<Self, DomainError> {
        let mut configuration = self.clone();
        configuration.tools = tools;
        configuration.refresh_prompt()?;
        configuration.validate()?;
        Ok(configuration)
    }
    fn refresh_prompt(&mut self) -> Result<(), DomainError> {
        if let Some(prompt) = &self.prompt {
            self.system_prompt = prompt.build(&self.model, &self.tools)?;
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        bounded_json(&self.model)?;
        bounded_json(&json!(self.tools))?;
        let window = self.model["contextWindow"].as_u64().unwrap_or(0);
        if self.model["provider"].as_str().is_none_or(str::is_empty)
            || self.model["id"].as_str().is_none_or(str::is_empty)
            || !(1..=16_000_000).contains(&window)
            || !["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                .contains(&self.thinking.as_str())
            || self.system_prompt.len() > 4 * 1024 * 1024
            || self.tools.len() > 128
        {
            return Err(invalid("runtime configuration"));
        }
        let mut names = BTreeSet::new();
        for tool in &self.tools {
            let name = tool["name"].as_str().ok_or_else(|| invalid("tool name"))?;
            if name.is_empty()
                || name.len() > 128
                || name.chars().any(char::is_control)
                || !names.insert(name)
                || !tool["description"].is_string()
                || !tool["parameters"].is_object()
            {
                return Err(invalid("runtime tool schema"));
            }
        }
        Ok(())
    }
    pub fn fixed_tokens(&self) -> u64 {
        (units(&self.system_prompt) as u64).div_ceil(3)
            + self.tools.iter().map(|tool| {
                // Same fields and per-tool rounding as the reference attribution.
                let schema = json!({"name":tool["name"],"description":tool["description"],"parameters":tool["parameters"]});
                (units(&schema.to_string()) as u64).div_ceil(3)
            }).sum::<u64>()
    }
    fn names(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect()
    }
}

pub struct SessionRuntime {
    boundary: Boundary,
    journal_revision: u64,
    committed_sequence: u64,
    branch: Arc<Vec<Value>>,
    messages: ContextSnapshot,
    configuration: Arc<Configuration>,
    pending_commit: bool,
    pending_tools: VecDeque<PendingCall>,
}

/// One ordered P05 transaction. Execute on the host's bounded storage executor.
/// Dropping an unacknowledged action leaves the session blocked; reload/reconcile
/// is required, since the owner cannot infer whether the worker committed it.
pub struct AppendAction {
    token: Append,
    write: PreparedStateCommit,
    next_branch: Arc<Vec<Value>>,
    next_messages: ContextSnapshot,
    next_configuration: Arc<Configuration>,
    tools: ToolTransition,
    compaction: Option<Compaction>,
}
pub struct CommitReceipt {
    action: AppendAction,
    result: Result<(u64, u64), DomainError>,
}
pub struct BranchAction {
    action: AppendAction,
    leaf: Option<String>,
    checkpoint: BranchCheckpoint,
}
#[derive(Clone)]
pub struct BranchCheckpoint {
    stamp: Stamp,
    branch: Arc<Vec<Value>>,
    configuration: Arc<Configuration>,
}
impl BranchAction {
    pub fn checkpoint(&self) -> BranchCheckpoint {
        self.checkpoint.clone()
    }
    pub fn execute(self, store: &impl crate::task::JournalBranchPort) -> CommitReceipt {
        let write = &self.action.write;
        let result = if write.records.is_empty() {
            store.select_leaf(
                &write.session,
                &write.command,
                write.expected,
                self.leaf.as_deref(),
            )
        } else {
            // The first new record points at the chosen historical leaf.
            // append_records selects the final record in that same transaction.
            write.commit(store)
        };
        CommitReceipt {
            action: self.action,
            result,
        }
    }
}
impl AppendAction {
    pub fn write(&self) -> &PreparedStateCommit {
        &self.write
    }
    pub fn execute(self, store: &impl JournalWritePort) -> CommitReceipt {
        let result = self.write.commit(store);
        CommitReceipt {
            action: self,
            result,
        }
    }
}

pub struct PreparationAction {
    token: Preparation,
    stamp: Stamp,
    journal_revision: u64,
    branch: Arc<Vec<Value>>,
    messages: ContextSnapshot,
    configuration: Arc<Configuration>,
    additional_input_tokens: u64,
}
/// An immutable summary input. The reservation fences the owner; the captured
/// prefix and configuration fence a delayed summary without losing a new suffix.
pub struct CompactionCapture {
    token: Compaction,
    branch: Arc<Vec<Value>>,
    configuration: Arc<Configuration>,
    journal_revision: u64,
}
impl CompactionCapture {
    pub fn branch(&self) -> &[Value] {
        &self.branch
    }
    pub fn configuration(&self) -> &Configuration {
        &self.configuration
    }
    pub fn journal_revision(&self) -> u64 {
        self.journal_revision
    }
}
impl PreparationAction {
    /// Host-accounted growth after compilation (for example a provider payload
    /// callback). It participates in the same final hard admission margin.
    pub fn account_additional_input(&mut self, tokens: u64) -> Result<(), DomainError> {
        self.additional_input_tokens = self
            .additional_input_tokens
            .checked_add(tokens)
            .ok_or_else(|| invalid("additional input token estimate"))?;
        Ok(())
    }
    /// Apply a host-owned client-mode tool view to this request only. Durable
    /// configuration is unchanged; normal stamp admission still fences it.
    pub fn set_tools(&mut self, tools: Vec<Value>) -> Result<(), DomainError> {
        self.configuration = Arc::new(self.configuration.with_tools(tools)?);
        Ok(())
    }
    /// Apply a typed native MCP selection before request-specific instructions.
    pub fn set_mcp_tools(&mut self, selected: &[crate::tools::mcp::SelectedDefinition], tools: Vec<Value>) -> Result<(), DomainError> {
        let mut configuration = self.configuration.as_ref().clone();
        if let Some(prompt) = &configuration.prompt {
            configuration.prompt = Some(Arc::new(prompt.with_mcp_selection(selected)?));
        }
        self.configuration = Arc::new(configuration.with_tools(tools)?);
        Ok(())
    }
    pub fn set_system_prompt(&mut self, system: String) -> Result<(), DomainError> {
        let mut configuration = self.configuration.as_ref().clone();
        configuration.system_prompt = system;
        configuration.validate()?;
        self.configuration = Arc::new(configuration);
        Ok(())
    }
    pub fn stamp(&self) -> &Stamp {
        &self.stamp
    }
    pub fn journal_revision(&self) -> u64 {
        self.journal_revision
    }
    pub fn branch(&self) -> &[Value] {
        &self.branch
    }
    pub fn messages(&self) -> &ContextSnapshot {
        &self.messages
    }
    pub fn configuration(&self) -> &Configuration {
        &self.configuration
    }
    pub fn compile_options(&self) -> CompileOptions {
        let mut options = self.configuration.compiler.clone();
        let maximum = self.configuration.model["contextWindow"].as_u64().unwrap();
        options.context_window = if options.context_window == 0 {
            maximum
        } else {
            options.context_window.min(maximum)
        };
        options.fixed_tokens = Some(self.configuration.fixed_tokens());
        options
    }
}
pub struct GenerationAction {
    token: Generation,
    configuration: Arc<Configuration>,
    projection: Compilation,
    stamp: Stamp,
    estimated_input_tokens: u64,
}
impl GenerationAction {
    pub fn configuration(&self) -> &Configuration {
        &self.configuration
    }
    pub fn projection(&self) -> &Compilation {
        &self.projection
    }
    pub fn stamp(&self) -> &Stamp {
        &self.stamp
    }
    pub fn estimated_input_tokens(&self) -> u64 {
        self.estimated_input_tokens
    }
}

fn validate_native_thinking(profile: &Value) -> Result<(), DomainError> {
    let valid = |mode: &Value| {
        let Some(id) = mode.as_str().or_else(|| mode["id"].as_str()) else {
            return false;
        };
        let native = mode["nativeValue"].as_str().unwrap_or(id);
        [
            "off", "on", "minimal", "low", "medium", "high", "xhigh", "max",
        ]
        .contains(&id)
            && native.len() <= 8
            && (native.eq_ignore_ascii_case(id)
                || id == "off" && native.eq_ignore_ascii_case("none"))
    };
    if profile.to_string().len() > 4096
        || profile["apply"] != "reasoning_effort"
        || profile["modes"].as_array().is_none_or(|modes| {
            modes.is_empty() || modes.len() > 8 || modes.iter().any(|m| !valid(m))
        })
    {
        return Err(invalid("native thinking profile"));
    }
    Ok(())
}
impl SessionRuntime {
    pub fn resume(
        session: SessionId,
        mut epoch: u64,
        journal: &StoredJournal,
        mut configuration: Configuration,
    ) -> Result<Self, DomainError> {
        let header: Value =
            serde_json::from_slice(&journal.header).map_err(|_| invalid("session header"))?;
        if header["id"] != session.as_str() {
            return Err(invalid("session identity"));
        }
        let branch = branch_values(journal)?;
        for entry in &branch {
            if entry["type"] == "compaction"
                && let Some(saved) = entry.get("contextEpoch")
            {
                epoch = epoch.max(
                    saved
                        .as_u64()
                        .ok_or_else(|| invalid("compaction context epoch"))?,
                );
            }
            if entry["type"] == "model_change" {
                configuration.compiler.context_window = 0;
            }
            if entry["customType"] == "ag1_context_limit" {
                let tokens = entry["data"]["tokens"]
                    .as_u64()
                    .ok_or_else(|| invalid("context limit"))?;
                if tokens < 1024 {
                    return Err(invalid("context limit"));
                }
                configuration.compiler.context_window = tokens;
            }
            if entry["type"] == "thinking_level_change" {
                configuration.thinking = entry["thinkingLevel"]
                    .as_str()
                    .ok_or_else(|| invalid("thinking level"))?
                    .into();
            }
        }
        if let Some(saved) = branch
            .iter()
            .rev()
            .take_while(|entry| entry["type"] != "model_change")
            .find_map(|entry| entry.get("nativeThinking"))
            && saved["provider"] == configuration.model["provider"]
            && saved["modelId"] == configuration.model["id"]
        {
            validate_native_thinking(&saved["profile"])?;
            configuration.model["thinking"] = saved["profile"].clone();
        }
        if let Some(entry) = branch
            .iter()
            .rev()
            .find(|entry| entry["type"] == "model_change")
            && (entry["provider"] != configuration.model["provider"]
                || entry["modelId"] != configuration.model["id"])
        {
            return Err(invalid("resolve the branch's selected model before resume"));
        }
        if let Some(entry) = branch
            .iter()
            .rev()
            .find(|entry| entry["type"] == "custom" && entry["customType"] == "ag1_active_tools")
            && entry["data"] != json!(configuration.names())
        {
            return Err(invalid("resolve the branch's active tools before resume"));
        }
        if configuration.compiler.context_window
            > configuration.model["contextWindow"].as_u64().unwrap_or(0)
        {
            return Err(invalid("restored context exceeds model capacity"));
        }
        configuration.refresh_prompt()?;
        configuration.validate()?;
        let messages = ContextSnapshot::capture(context_messages(&branch))?;
        let mut pending_tools = VecDeque::<PendingCall>::new();
        for message in messages.values() {
            if message["role"] == "assistant" {
                for call in calls(message)? {
                    if pending_tools.iter().any(|prior| prior.id == call.id) {
                        return Err(invalid("duplicate unresolved tool call"));
                    }
                    pending_tools.push_back(call);
                }
            } else if message["role"] == "toolResult" {
                pending_tools.retain(|call| {
                    message["toolCallId"] != call.id || message["toolName"] != call.name
                });
            }
            if pending_tools.len() > 128 {
                return Err(DomainError::LimitExceeded("unresolved tool calls".into()));
            }
        }
        let mut boundary = Boundary::new(session, epoch)?;
        boundary.register_tools(
            &pending_tools
                .iter()
                .map(|call| call.id.clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(Self {
            boundary,
            journal_revision: journal.revision,
            committed_sequence: journal.committed_sequence,
            branch: Arc::new(branch),
            messages,
            configuration: Arc::new(configuration),
            pending_commit: false,
            pending_tools,
        })
    }
    pub fn stamp(&self) -> &Stamp {
        self.boundary.stamp()
    }
    pub fn journal_revision(&self) -> u64 {
        self.journal_revision
    }
    pub fn committed_sequence(&self) -> u64 {
        self.committed_sequence
    }
    pub fn branch_snapshot(&self) -> Arc<Vec<Value>> {
        self.branch.clone()
    }
    pub fn branch(&self) -> &[Value] {
        &self.branch
    }
    pub fn messages(&self) -> &ContextSnapshot {
        &self.messages
    }
    pub fn configuration(&self) -> &Configuration {
        &self.configuration
    }
    pub fn settled(&self) -> bool {
        self.boundary.settled()
    }
    pub fn persistence_healthy(&self) -> bool {
        self.boundary.persistence_healthy()
    }
    pub fn pending_commit(&self) -> bool {
        self.pending_commit
    }
    /// Durable unresolved calls in source order. After resume these are for
    /// reconciliation against P09 operation receipts, never automatic execution.
    pub fn pending_tools(&self) -> &VecDeque<PendingCall> {
        &self.pending_tools
    }
    pub fn cancel(&mut self) {
        self.boundary.cancel();
    }
    pub fn fail_persistence(&mut self) {
        self.boundary.fail_persistence();
    }
    pub fn next_turn(&mut self) -> Result<(), DomainError> {
        self.boundary.next_turn()
    }

    fn append(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        mut record: Value,
        configuration: Arc<Configuration>,
    ) -> Result<AppendAction, DomainError> {
        if self.pending_commit {
            return Err(DomainError::Unavailable("pending journal commit".into()));
        }
        SessionId::new(command)?;
        SessionId::new(id)?;
        if chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
            || self.branch.iter().any(|e| e["id"] == id)
        {
            return Err(invalid("entry timestamp or duplicate identity"));
        }
        if self.branch.len() >= 100_000 {
            return Err(DomainError::LimitExceeded("session branch".into()));
        }
        record["id"] = json!(id);
        record["parentId"] = self
            .branch
            .last()
            .map(|e| e["id"].clone())
            .unwrap_or(Value::Null);
        record["timestamp"] = json!(timestamp);
        let record_limit = if record["type"] == "message"
            && record["message"]["role"] == "assistant"
        {
            LONG_ASSISTANT_STATE_BYTES
        } else {
            4 * 1024 * 1024
        };
        bounded_json_with_limit(&record, record_limit)?;
        let tools = if record["type"] == "message" && record["message"]["role"] == "assistant" {
            if !self.pending_tools.is_empty() {
                return Err(DomainError::Unavailable("unresolved tool calls".into()));
            }
            ToolTransition::Start(calls(&record["message"])?)
        } else if record["type"] == "message" && record["message"]["role"] == "toolResult" {
            let pending = self.pending_tools.front().ok_or(DomainError::Conflict)?;
            if record["message"]["toolCallId"] != pending.id
                || record["message"]["toolName"] != pending.name
            {
                return Err(DomainError::Conflict);
            }
            ToolTransition::Finish(pending.id.clone())
        } else {
            ToolTransition::Keep
        };
        let records = vec![serde_json::to_vec(&record).map_err(|_| invalid("entry encoding"))?];
        let mut branch = self.branch.as_ref().clone();
        branch.push(record);
        let messages = ContextSnapshot::capture(context_messages(&branch))?;
        let token = self.boundary.begin_append()?;
        self.pending_commit = true;
        Ok(AppendAction {
            token,
            write: PreparedStateCommit {
                session: self.stamp().session().clone(),
                command: command.into(),
                expected: self.journal_revision,
                records,
            },
            next_branch: Arc::new(branch),
            next_messages: messages,
            next_configuration: configuration,
            tools,
            compaction: None,
        })
    }
    pub fn append_message(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        message: Value,
    ) -> Result<AppendAction, DomainError> {
        validate_history(std::slice::from_ref(&message))?;
        if !["user", "assistant", "toolResult", "bashExecution", "custom"]
            .contains(&message["role"].as_str().unwrap_or(""))
        {
            return Err(invalid("durable runtime message role"));
        }
        if message["role"] == "custom"
            && (message["customType"]
                .as_str()
                .is_none_or(|name| name.is_empty() || name.len() > 128)
                || !message["display"].is_boolean())
        {
            return Err(invalid("durable custom message"));
        }
        self.append(
            command,
            id,
            timestamp,
            json!({"type":"message","message":message}),
            self.configuration.clone(),
        )
    }
    pub fn select_model(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        model: Value,
    ) -> Result<AppendAction, DomainError> {
        let mut configuration = self.configuration.as_ref().clone();
        configuration.model = model;
        configuration.compiler.context_window = 0;
        configuration.refresh_prompt()?;
        configuration.validate()?;
        let record = json!({"type":"model_change","provider":configuration.model["provider"],"modelId":configuration.model["id"]});
        self.append(command, id, timestamp, record, Arc::new(configuration))
    }
    /// An explicit working-context limit within the admitted model capacity.
    /// This changes compilation only; runtime profiles and server argv are not rewritten.
    pub fn select_context(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        tokens: u64,
    ) -> Result<AppendAction, DomainError> {
        let maximum = self.configuration.model["contextWindow"]
            .as_u64()
            .ok_or_else(|| invalid("model context"))?;
        if tokens < 1024 || tokens > maximum {
            return Err(invalid("context limit exceeds admitted model capacity"));
        }
        let mut configuration = self.configuration.as_ref().clone();
        configuration.compiler.context_window = tokens;
        self.append(
            command,
            id,
            timestamp,
            json!({"type":"custom","customType":"ag1_context_limit","data":{"tokens":tokens}}),
            Arc::new(configuration),
        )
    }
    pub fn select_tools(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        tools: Vec<Value>,
    ) -> Result<AppendAction, DomainError> {
        let mut configuration = self.configuration.as_ref().clone();
        configuration.tools = tools;
        configuration.refresh_prompt()?;
        configuration.validate()?;
        let record =
            json!({"type":"custom","customType":"ag1_active_tools","data":configuration.names()});
        self.append(command, id, timestamp, record, Arc::new(configuration))
    }
    pub fn install_prompt(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        prompt: super::prompt::PromptConfiguration,
    ) -> Result<AppendAction, DomainError> {
        let mut configuration = self.configuration.as_ref().clone();
        let record = json!({"type":"custom","customType":"ag1_prompt_resources","data":{"fingerprint":prompt.fingerprint()}});
        configuration.prompt = Some(Arc::new(prompt));
        configuration.refresh_prompt()?;
        configuration.validate()?;
        self.append(command, id, timestamp, record, Arc::new(configuration))
    }
    pub fn select_thinking(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        thinking: String,
    ) -> Result<AppendAction, DomainError> {
        self.select_native_thinking(command, id, timestamp, thinking, None)
    }
    /// Persist a discovered native effort mapping with the selected model. This
    /// is request metadata; it never modifies the model's launch profile.
    pub fn select_native_thinking(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        thinking: String,
        native_profile: Option<Value>,
    ) -> Result<AppendAction, DomainError> {
        let mut configuration = self.configuration.as_ref().clone();
        configuration.thinking = thinking;
        let mut record =
            json!({"type":"thinking_level_change","thinkingLevel":configuration.thinking});
        if let Some(profile) = native_profile {
            validate_native_thinking(&profile)?;
            configuration.model["thinking"] = profile.clone();
            record["nativeThinking"] = json!({"provider":configuration.model["provider"],"modelId":configuration.model["id"],"profile":profile});
        }
        configuration.validate()?;
        self.append(command, id, timestamp, record, Arc::new(configuration))
    }
    pub fn acknowledge(&mut self, receipt: CommitReceipt) -> Result<(), DomainError> {
        let action = receipt.action;
        // Token validation occurs before touching any owner state, including on
        // an error receipt from an obsolete runtime with the same session ID.
        match receipt.result {
            Ok((revision, sequence))
                // P05 append revisions count rows; a branch-selection action
                // has no rows and advances the revision once.
                if self.journal_revision.checked_add(action.write.records.len().max(1) as u64) == Some(revision)
                    && sequence > self.committed_sequence =>
            {
                match action.tools {
                    ToolTransition::Keep => self.boundary.acknowledge(&action.token)?,
                    ToolTransition::Start(calls) => {
                        self.boundary.acknowledge_tool_batch(
                            &action.token,
                            &calls.iter().map(|call| call.id.clone()).collect::<Vec<_>>(),
                        )?;
                        self.pending_tools = calls.into();
                    }
                    ToolTransition::Finish(id) => {
                        self.boundary.acknowledge_tool(&action.token, &id)?;
                        self.pending_tools.pop_front();
                    }
                }
                self.journal_revision = revision;
                self.committed_sequence = sequence;
                self.branch = action.next_branch;
                self.messages = action.next_messages;
                self.configuration = action.next_configuration;
                self.pending_commit = false;
                if let Some(compaction) = action.compaction {
                    self.boundary.finish_compaction(&compaction, true)?;
                }
                Ok(())
            }
            result => {
                self.boundary.append_failed(&action.token)?;
                self.pending_commit = false;
                if let Some(compaction) = action.compaction {
                    self.boundary.finish_compaction(&compaction, false)?;
                }
                Err(result.err().unwrap_or(DomainError::DurabilityUnknown))
            }
        }
    }
    /// Remove one settled failed attempt from the active branch while keeping
    /// its raw records in storage. Rejected tool outcomes may follow the failure.
    pub fn branch_before_failure(
        &mut self,
        command: &str,
        failed: &str,
        expected: &Stamp,
    ) -> Result<BranchAction, DomainError> {
        SessionId::new(command)?;
        if self.stamp() != expected || !self.settled() {
            return Err(DomainError::Conflict);
        }
        let index = self
            .branch
            .iter()
            .position(|entry| entry["id"] == failed)
            .ok_or(DomainError::Conflict)?;
        let message = &self.branch[index]["message"];
        if message["role"] != "assistant"
            || message["stopReason"] != "error"
            || self.branch[index + 1..].iter().any(|entry| {
                entry["message"]["role"] != "toolResult"
                    || entry["message"]["details"]["rejectedBeforeExecution"] != true
            })
        {
            return Err(DomainError::Conflict);
        }
        let branch = self.branch[..index].to_vec();
        self.stage_branch(command, branch)
    }
    /// Successful providers can report input usage beyond their context window.
    /// Only a terminal leaf response can be excluded: executed tools and later
    /// journal mutations must never disappear during overflow recovery.
    pub fn branch_before_overflow(
        &mut self,
        command: &str,
        failed: &str,
        expected: &Stamp,
    ) -> Result<BranchAction, DomainError> {
        let entry = self
            .branch
            .iter()
            .find(|entry| entry["id"] == failed)
            .ok_or(DomainError::Conflict)?;
        if entry["message"]["stopReason"] == "error" {
            return self.branch_before_failure(command, failed, expected);
        }
        if self.stamp() != expected
            || !self.settled()
            || self.branch.last().is_none_or(|entry| entry["id"] != failed)
            || entry["message"]["role"] != "assistant"
            || !super::retry::overflow(&entry["message"], self.configuration.context_limit())
            || entry["message"]["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "toolCall"))
        {
            return Err(DomainError::Conflict);
        }
        self.stage_branch(command, self.branch[..self.branch.len() - 1].to_vec())
    }
    /// Roll back an unsent continuation only while its exact post-selection
    /// stamp remains current. New edits, compaction or a replacement owner win.
    pub fn restore_branch(
        &mut self,
        command: &str,
        checkpoint: &BranchCheckpoint,
        expected: &Stamp,
    ) -> Result<BranchAction, DomainError> {
        if self.stamp() != expected
            || !expected.same_owner_epoch(&checkpoint.stamp)
            || !self.settled()
            || !Arc::ptr_eq(&self.configuration, &checkpoint.configuration)
        {
            return Err(DomainError::Conflict);
        }
        self.stage_branch(command, checkpoint.branch.as_ref().clone())
    }
    fn stage_branch(
        &mut self,
        command: &str,
        branch: Vec<Value>,
    ) -> Result<BranchAction, DomainError> {
        SessionId::new(command)?;
        let messages = ContextSnapshot::capture(context_messages(&branch))?;
        let leaf = branch
            .last()
            .and_then(|entry| entry["id"].as_str())
            .map(str::to_owned);
        let token = self.boundary.begin_append()?;
        self.pending_commit = true;
        Ok(BranchAction {
            leaf,
            checkpoint: BranchCheckpoint {
                stamp: self.stamp().clone(),
                branch: self.branch.clone(),
                configuration: self.configuration.clone(),
            },
            action: AppendAction {
                token,
                write: PreparedStateCommit {
                    session: self.stamp().session().clone(),
                    command: command.into(),
                    expected: self.journal_revision,
                    records: vec![],
                },
                next_branch: Arc::new(branch),
                next_messages: messages,
                next_configuration: self.configuration.clone(),
                tools: ToolTransition::Keep,
                compaction: None,
            },
        })
    }

    pub fn append_task_state(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        task: crate::task::TaskState,
    ) -> Result<AppendAction, DomainError> {
        self.append(
            command,
            id,
            timestamp,
            json!({"type":"custom","customType":"task_state","data":task.value()}),
            self.configuration.clone(),
        )
    }
    /// Journal-backed tool effects use the same append token and acknowledgement
    /// path as every other mutation. Only the current pending call may publish.
    pub fn append_tool_state(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        call_id: &str,
        custom_type: &str,
        data: Value,
    ) -> Result<AppendAction, DomainError> {
        let pending = self.pending_tools.front().ok_or(DomainError::Conflict)?.effective(&self.branch)?;
        if pending.id != call_id {
            return Err(DomainError::Conflict);
        }
        match (pending.name.as_str(), custom_type) {
            ("explore", "ag1.explore_tasks") => {
                super::subagent::explore::validate_rollup(&data, &pending.block["arguments"])?;
            }
            ("task", "ag1.subagent_task") => {
                super::subagent::tool::validate_rollup(&data, &pending.block["arguments"])?;
            }
            ("taskState", "task_state") => {
                crate::task::TaskState::parse(&data)?;
            }
            ("sourceContext", "context_capsule") => {
                crate::context::capsule::CapsuleRecord::parse(&data)?;
            }
            ("episode", "context_episode") => {
                use crate::context::episode;
                let record = episode::EpisodeRecord::parse(&data)?;
                let args = &pending.block["arguments"];
                if args["action"] == "begin" {
                    let expected = episode::begin_record(
                        &self.branch,
                        timestamp,
                        data["id"].as_str().unwrap_or(""),
                        args["goal"].as_str().unwrap_or(""),
                        args["title"].as_str(),
                    )?;
                    if expected.value() != record.value() {
                        return Err(DomainError::Conflict);
                    }
                } else if args["action"] == "close" {
                    let (open, start) =
                        episode::find_open(&self.branch).ok_or(DomainError::Conflict)?;
                    if data["event"] != "closed"
                        || data["id"] != open.value()["id"]
                        || data["goal"] != open.value()["goal"]
                        || data["title"] != open.value()["title"]
                        || data["openedAt"] != open.value()["createdAt"]
                        || data["closedAt"] != timestamp
                        || data["startEntryId"] != self.branch[start]["id"]
                        || data["endEntryId"] != self.branch.last().unwrap()["id"]
                        || data["entryCount"].as_u64() != Some((self.branch.len() - start) as u64)
                    {
                        return Err(DomainError::Conflict);
                    }
                    let mut expected = data.clone();
                    for field in [
                        "outcome",
                        "synopsis",
                        "claims",
                        "fileRevisions",
                        "verificationSources",
                    ] {
                        expected[field] = args.get(field).cloned().unwrap_or_else(|| json!([]));
                    }
                    if episode::EpisodeRecord::parse(&expected)?.value() != record.value() {
                        return Err(DomainError::Conflict);
                    }
                } else {
                    return Err(DomainError::PermissionDenied);
                }
            }
            _ => return Err(DomainError::PermissionDenied),
        }
        self.append(
            command,
            id,
            timestamp,
            json!({"type":"custom","customType":custom_type,"data":data}),
            self.configuration.clone(),
        )
    }
    /// Derived checklist/observation changes share one durable transaction.
    /// The owner derives every record from the pending call and current branch.
    pub fn append_journal_tool(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        call_id: &str,
    ) -> Result<(Option<AppendAction>, Value), DomainError> {
        use sha2::{Digest, Sha256};
        let pending = self.pending_tools.front().ok_or(DomainError::Conflict)?.effective(&self.branch)?;
        if pending.id != call_id || !["todo", "runtimeObservation"].contains(&pending.name.as_str())
        {
            return Err(DomainError::PermissionDenied);
        }
        let now = chrono::DateTime::parse_from_rfc3339(timestamp)
            .map_err(|_| invalid("journal tool timestamp"))?
            .timestamp_millis();
        let digest = format!("{:x}", Sha256::digest(command.as_bytes()));
        let plan = if pending.name == "todo" {
            crate::tools::todo::prepare(
                &self.branch,
                &pending.block["arguments"],
                now,
                &format!("plan_{}", &digest[..12]),
            )?
        } else {
            crate::tools::observation::prepare(
                &self.branch,
                &pending.block["arguments"],
                timestamp,
                crate::tools::observation::ObservationIdentity {
                    observation: &format!("obs_{}", &digest[..12]),
                    entry: &format!("{id}-0"),
                    task_entry: &format!("ts_{}", &digest[..8]),
                    call: call_id,
                },
            )?
        };
        if plan.records.is_empty() {
            return Ok((None, plan.result));
        }
        if self.pending_commit {
            return Err(DomainError::Unavailable("pending journal commit".into()));
        }
        SessionId::new(command)?;
        SessionId::new(id)?;
        if self.branch.len() + plan.records.len() > 100_000 {
            return Err(DomainError::LimitExceeded("session branch".into()));
        }
        let mut branch = self.branch.as_ref().clone();
        let mut records = vec![];
        for (index, mut record) in plan.records.into_iter().enumerate() {
            let entry_id = format!("{id}-{index}");
            SessionId::new(&entry_id)?;
            if branch.iter().any(|entry| entry["id"] == entry_id) {
                return Err(invalid("duplicate journal tool identity"));
            }
            record["id"] = json!(entry_id);
            record["parentId"] = branch
                .last()
                .map(|entry| entry["id"].clone())
                .unwrap_or(Value::Null);
            record["timestamp"] = json!(timestamp);
            bounded_json(&record)?;
            records.push(
                serde_json::to_vec(&record).map_err(|_| invalid("journal tool entry encoding"))?,
            );
            branch.push(record);
        }
        let messages = ContextSnapshot::capture(context_messages(&branch))?;
        let token = self.boundary.begin_append()?;
        self.pending_commit = true;
        Ok((
            Some(AppendAction {
                token,
                write: PreparedStateCommit {
                    session: self.stamp().session().clone(),
                    command: command.into(),
                    expected: self.journal_revision,
                    records,
                },
                next_branch: Arc::new(branch),
                next_messages: messages,
                next_configuration: self.configuration.clone(),
                tools: ToolTransition::Keep,
                compaction: None,
            }),
            plan.result,
        ))
    }
    pub fn capture_inspection(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        result_entry: &str,
    ) -> Result<Option<AppendAction>, DomainError> {
        let record = self
            .branch
            .iter()
            .find(|record| record["id"] == result_entry)
            .ok_or(DomainError::Conflict)?;
        let message = &record["message"];
        if record["type"] != "message"
            || message["role"] != "toolResult"
            || message["isError"].as_bool().unwrap_or(false)
        {
            return Ok(None);
        }
        let Ok(evidence) =
            crate::evidence::InspectionEvidence::parse(&message["details"]["inspectionEvidence"])
        else {
            return Ok(None);
        };
        if evidence.value()["toolCallId"] != message["toolCallId"] {
            return Ok(None);
        }
        self.append(
            command,
            id,
            timestamp,
            json!({"type":"custom","customType":"inspection_evidence","data":evidence.value()}),
            self.configuration.clone(),
        )
        .map(Some)
    }
    pub fn capture_instruction(
        &mut self,
        command: &str,
        id: &str,
        timestamp: &str,
        instruction: &str,
    ) -> Result<Option<AppendAction>, DomainError> {
        let current = super::context::latest_task(&self.branch)
            .unwrap_or_else(|| crate::task::TaskState::empty(timestamp));
        let users = self
            .branch
            .iter()
            .filter(|entry| entry["type"] == "message" && entry["message"]["role"] == "user")
            .collect::<Vec<_>>();
        let prior = users
            .iter()
            .take(users.len().saturating_sub(1))
            .map(|entry| crate::context::hygiene::content(&entry["message"], "\n"))
            .collect::<Vec<_>>();
        let next = current.capture_instruction(instruction, &prior, timestamp)?;
        if next.value() == current.value() {
            return Ok(None);
        }
        self.append_task_state(command, id, timestamp, next)
            .map(Some)
    }
    pub fn prepare_request(&mut self) -> Result<PreparationAction, DomainError> {
        let token = self.boundary.prepare()?;
        Ok(PreparationAction {
            token,
            stamp: self.stamp().clone(),
            journal_revision: self.journal_revision,
            branch: self.branch.clone(),
            messages: self.messages.clone(),
            configuration: self.configuration.clone(),
            additional_input_tokens: 0,
        })
    }
    pub fn reserve_compaction(&mut self) -> Result<Compaction, DomainError> {
        self.boundary.reserve_compaction()
    }
    /// Tree selection and optional summary/label publication share one commit.
    /// A delayed navigation never discards a suffix appended after its capture.
    pub fn navigate_tree(
        &mut self,
        capture: &CompactionCapture,
        journal: &StoredJournal,
        target: &str,
        command: &str,
        mut records: Vec<Value>,
    ) -> Result<BranchAction, DomainError> {
        self.check_compaction_capture(capture)?;
        if journal.revision != capture.journal_revision
            || self.journal_revision != capture.journal_revision
            || branch_values(journal)? != *self.branch
            || records.len() > 2
        {
            return Err(DomainError::Conflict);
        }
        let mut branch = super::tree::prepare(journal, target)?.branch;
        let mut bytes = vec![];
        for record in &mut records {
            if !["branch_summary", "label"]
                .iter()
                .any(|kind| record["type"] == *kind)
                || record["id"]
                    .as_str()
                    .is_none_or(|id| journal.entries.iter().any(|row| row.id == id))
            {
                return Err(invalid("tree navigation record"));
            }
            SessionId::new(record["id"].as_str().unwrap())?;
            if record["type"] == "branch_summary"
                && (record["summary"]
                    .as_str()
                    .is_none_or(|s| s.trim().is_empty())
                    || record["fromId"]
                        != journal
                            .leaf
                            .clone()
                            .map(Value::String)
                            .unwrap_or(Value::Null))
            {
                return Err(invalid("tree summary"));
            }
            record["parentId"] = branch
                .last()
                .map(|e| e["id"].clone())
                .unwrap_or(Value::Null);
            bounded_json(record)?;
            bytes.push(serde_json::to_vec(record).map_err(|_| invalid("tree navigation record"))?);
            branch.push(record.clone());
        }
        let mut action = self.stage_branch(command, branch)?;
        action.action.write.records = bytes;
        action.action.compaction = Some(capture.token.clone());
        Ok(action)
    }
    pub fn compaction_ready(&self) -> bool {
        self.boundary.compaction_ready()
    }
    pub fn capture_compaction(&self, token: &Compaction) -> Result<CompactionCapture, DomainError> {
        self.boundary.check_compaction(token)?;
        if !self.boundary.compaction_ready() {
            return Err(DomainError::Unavailable(
                "compaction is waiting for accepted work".into(),
            ));
        }
        Ok(CompactionCapture {
            token: token.clone(),
            branch: self.branch.clone(),
            configuration: self.configuration.clone(),
            journal_revision: self.journal_revision,
        })
    }
    pub fn abandon_compaction(&mut self, token: &Compaction) -> Result<(), DomainError> {
        self.boundary.finish_compaction(token, false)
    }
    /// The host retains full trajectory/output artifacts before this call. A
    /// committed suffix remains in the parent chain exactly once; summary text
    /// never replaces original records. Model/tool changes require a new capture.
    pub fn append_compaction(
        &mut self,
        capture: &CompactionCapture,
        command: &str,
        id: &str,
        timestamp: &str,
        result: Value,
    ) -> Result<AppendAction, DomainError> {
        self.check_compaction_capture(capture)?;
        let first = result["firstKeptEntryId"]
            .as_str()
            .ok_or_else(|| invalid("compaction cut point"))?;
        if result["summary"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
            || result["tokensBefore"].as_u64().is_none()
            || !capture.branch.iter().any(|entry| entry["id"] == first)
        {
            return Err(invalid("compaction result"));
        }
        let cut = capture
            .branch
            .iter()
            .position(|entry| entry["id"] == first)
            .unwrap();
        if crate::context::compaction::close_pair_boundary(&capture.branch, cut) != cut {
            return Err(invalid("compaction splits a tool/result pair"));
        }
        let next_epoch = self
            .stamp()
            .epoch()
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let mut record = json!({"type":"compaction","summary":result["summary"],"firstKeptEntryId":first,"tokensBefore":result["tokensBefore"],"contextEpoch":next_epoch});
        if let Some(details) = result.get("details") {
            record["details"] = details.clone();
        }
        let mut action = self.append(command, id, timestamp, record, self.configuration.clone())?;
        action.compaction = Some(capture.token.clone());
        Ok(action)
    }
    pub fn check_compaction_capture(&self, capture: &CompactionCapture) -> Result<(), DomainError> {
        self.boundary.check_compaction(&capture.token)?;
        if !self.boundary.compaction_ready()
            || !Arc::ptr_eq(&capture.configuration, &self.configuration)
            || !self.branch.starts_with(&capture.branch)
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    /// Call after ALL async preparation, including credential resolution and
    /// acquisition of the shared P06 generation permit. The host must dispatch
    /// the returned action without another await or intervening session command.
    pub fn admit_request(
        &mut self,
        preparation: PreparationAction,
        projection: Result<Compilation, DomainError>,
    ) -> Result<GenerationAction, DomainError> {
        let checked = projection.and_then(|projection| {
            validate_history(&projection.messages)?;
            let estimate = preparation
                .configuration
                .fixed_tokens()
                .checked_add(projection.messages.iter().map(estimate_tokens).sum::<u64>())
                .and_then(|tokens| tokens.checked_add(preparation.additional_input_tokens))
                .ok_or_else(|| invalid("request token estimate"))?;
            let with_margin = estimate
                .checked_mul(11)
                .ok_or_else(|| invalid("context token estimate"))?
                .div_ceil(10);
            if with_margin > preparation.configuration.context_limit() {
                return Err(DomainError::LimitExceeded(
                    "request context admission margin".into(),
                ));
            }
            Ok((projection, estimate))
        });
        let (projection, estimated_input_tokens) = match checked {
            Ok(value) => value,
            Err(error) => {
                let _ = self.boundary.discard_preparation(&preparation.token);
                return Err(error);
            }
        };
        let token = self.boundary.admit(preparation.token)?;
        Ok(GenerationAction {
            token,
            configuration: preparation.configuration,
            projection,
            stamp: self.stamp().clone(),
            estimated_input_tokens,
        })
    }
    /// Transport settlement only. Accepted message/tool/append actions continue
    /// to hold the boundary closed until their independent durable receipts.
    pub fn generation_terminal(
        &mut self,
        generation: &GenerationAction,
    ) -> Result<(), DomainError> {
        self.boundary.generation_terminal(&generation.token)
    }
}
