//! Durable tool lifecycle. An acknowledged intent precedes execution; an acknowledged result precedes end.
//! A recovered intent is uncertain and is never automatically executed again.
use super::{
    approval::{AuthorizedCall, ExecutionFence},
    registry::{Registry, Scope, ValidatedCall},
    result::{self, ResultArtifacts, ResultBudget},
};
use crate::{DomainError, context::ContextArtifact, storage::StoragePort};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub trait ExecutionArtifacts: ResultArtifacts {
    fn read_json(&self, artifact: &ContextArtifact) -> Result<Value, DomainError>;
}
pub enum BeforeHook {
    Continue,
    Arguments(Value),
    Block(String),
}
pub trait Hooks {
    fn before(&mut self, _call: &ValidatedCall) -> Result<BeforeHook, DomainError> {
        Ok(BeforeHook::Continue)
    }
    fn after(
        &mut self,
        _call: &ValidatedCall,
        _native_result: &Value,
    ) -> Result<Option<Value>, DomainError> {
        Ok(None)
    }
}
pub struct NoHooks;
impl Hooks for NoHooks {}
/// Hook argument changes go through preparation, schema validation and policy again.
pub fn prepare(
    registry: &Registry,
    call: &ValidatedCall,
    hooks: &mut impl Hooks,
) -> Result<ValidatedCall, DomainError> {
    registry.validate_current(call)?;
    match hooks.before(call)? {
        BeforeHook::Continue => Ok(call.clone()),
        BeforeHook::Arguments(args) => {
            registry.prepare(call.operation(), call.call_id(), call.name(), &args, true)
        }
        BeforeHook::Block(_) => Err(DomainError::PermissionDenied),
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    None,
    Committed,
    Unknown,
}
/// Adapters report effect state separately from success. A cancelled shell may already have changed files.
pub struct AdapterOutcome {
    pub result: Value,
    pub effect: Effect,
    pub artifact_page: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Intent,
    Completed,
    Rejected,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Record {
    pub version: u32,
    pub identity: String,
    pub operation: String,
    pub call_id: String,
    pub tool: String,
    pub workspace: String,
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
    pub policy_revision: u64,
    pub target_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<super::mcp::Identity>,
    pub arguments: ContextArtifact,
    pub status: Status,
    pub effect: Effect,
    pub started_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_result: Option<ContextArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<ContextArtifact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_hook_error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvent {
    pub id: String,
    pub operation: String,
    pub call_id: String,
    pub stage: String,
}
pub struct Outcome {
    pub record: Record,
    pub result: Value,
    pub replayed: bool,
}
pub struct Execution<'a> {
    pub registry: &'a Registry,
    pub fence: ExecutionFence<'a>,
    pub budget: ResultBudget,
    pub clock: &'a dyn crate::Clock,
}
fn digest(v: &Value) -> String {
    format!("{:x}", Sha256::digest(v.to_string().as_bytes()))
}
fn key(workspace: &str, session: &str, operation: &str) -> String {
    format!("tool-{}", digest(&json!([workspace, session, operation])))
}
fn identity(call: &ValidatedCall, scope: &Scope) -> String {
    let legacy = digest(&json!([
        call.operation(),
        call.call_id(),
        call.name(),
        call.args_hash(),
        scope.workspace,
        scope.session,
        scope.epoch,
        scope.revision
    ]));
    match call.mcp_identity() {
        Some(identity) => digest(&json!(["mcp", legacy, identity])),
        None => legacy,
    }
}
fn source(call: &ValidatedCall) -> Value {
    let mut source = json!({"type":"tool","name":call.name(),"callId":call.call_id()});
    if let Some(identity) = call.mcp_identity() {
        source["mcp"] = json!(identity);
    }
    source
}
fn event(record: &Record, stage: &str) -> ToolEvent {
    ToolEvent {
        id: format!(
            "{}:{stage}",
            key(&record.workspace, &record.session, &record.operation)
        ),
        operation: record.operation.clone(),
        call_id: record.call_id.clone(),
        stage: stage.into(),
    }
}
fn load(store: &impl StoragePort, key: &str) -> Result<Option<(String, Record)>, DomainError> {
    store
        .load_metadata("tool-operation", key)?
        .map(|d| {
            let record: Record = serde_json::from_slice(&d.bytes)
                .map_err(|_| DomainError::InvalidInput("tool operation record".into()))?;
            if record.version != 1 {
                return Err(DomainError::Unavailable("tool record version".into()));
            }
            Ok((d.sha256, record))
        })
        .transpose()
}
fn save(
    store: &impl StoragePort,
    key: &str,
    expected: Option<&str>,
    record: &Record,
) -> Result<String, DomainError> {
    let bytes =
        serde_json::to_vec(record).map_err(|_| DomainError::InvalidInput("tool record".into()))?;
    store.commit_metadata("tool-operation", key, expected, &bytes)
}
fn receipt(artifacts: &impl ExecutionArtifacts, record: Record) -> Result<Outcome, DomainError> {
    if record.status == Status::Intent {
        return Err(DomainError::DurabilityUnknown);
    }
    let result = artifacts.read_json(
        record
            .result
            .as_ref()
            .ok_or(DomainError::DurabilityUnknown)?,
    )?;
    Ok(Outcome {
        record,
        result,
        replayed: true,
    })
}
/// Parent-authorized recovery only. Returning an intent does not grant permission to retry its effect.
pub fn recover(
    store: &impl StoragePort,
    workspace: &str,
    session: &str,
    operation: &str,
) -> Result<Option<Record>, DomainError> {
    let key = key(workspace, session, operation);
    load(store, &key)?
        .map(|(_, r)| {
            if r.workspace != workspace || r.session != session || r.operation != operation {
                return Err(DomainError::Conflict);
            }
            Ok(r)
        })
        .transpose()
}
/// Execute closure is invoked exactly once after intent acknowledgement. The adapter consumes the ticket.
/// Storage or transport failures after intent leave an uncertain record for explicit reconciliation.
pub fn execute(
    store: &impl StoragePort,
    artifacts: &mut impl ExecutionArtifacts,
    authorization: AuthorizedCall,
    execution: Execution<'_>,
    hooks: &mut impl Hooks,
    adapter: impl FnOnce(AuthorizedCall) -> Result<AdapterOutcome, DomainError>,
    on_event: impl FnMut(ToolEvent),
) -> Result<Outcome, DomainError> {
    execute_with_artifacts(
        store,
        artifacts,
        authorization,
        execution,
        hooks,
        |ticket, _| adapter(ticket),
        on_event,
    )
}
/// Artifact-producing adapters share the same operation port and occurrence sequence.
pub fn execute_with_artifacts<A: ExecutionArtifacts>(
    store: &impl StoragePort,
    artifacts: &mut A,
    authorization: AuthorizedCall,
    execution: Execution<'_>,
    hooks: &mut impl Hooks,
    adapter: impl FnOnce(AuthorizedCall, &mut A) -> Result<AdapterOutcome, DomainError>,
    mut on_event: impl FnMut(ToolEvent),
) -> Result<Outcome, DomainError> {
    let Execution {
        registry,
        fence,
        budget,
        clock,
    } = execution;
    let fence = ExecutionFence {
        now: clock.now_ms(),
        ..fence
    };
    authorization.check(registry, fence)?;
    let call = authorization.call().clone();
    let id = identity(&call, fence.scope);
    let key = key(
        &fence.scope.workspace,
        &fence.scope.session,
        call.operation(),
    );
    if let Some((_, record)) = load(store, &key)? {
        if record.identity != id {
            return Err(DomainError::Conflict);
        }
        return receipt(artifacts, record);
    }
    let args = artifacts.put_evidence(
        &call.args().to_string(),
        "tool-arguments",
        &source(&call),
        &[],
        None,
    )?;
    let mut record = Record {
        version: 1,
        identity: id,
        operation: call.operation().into(),
        call_id: call.call_id().into(),
        tool: call.name().into(),
        workspace: fence.scope.workspace.clone(),
        session: fence.scope.session.clone(),
        epoch: fence.scope.epoch,
        revision: fence.scope.revision,
        policy_revision: fence.policy_revision,
        target_revision: fence.target_revision.into(),
        mcp: call.mcp_identity().cloned(),
        arguments: args,
        status: Status::Intent,
        effect: Effect::Unknown,
        started_ms: fence.now,
        finished_ms: None,
        native_result: None,
        result: None,
        post_hook_error: None,
    };
    let revision = save(store, &key, None, &record)?;
    on_event(event(&record, "start"));
    let outcome = adapter(authorization, artifacts)?;
    if outcome.artifact_page && call.name() != "artifact" {
        return Err(DomainError::PermissionDenied);
    }
    super::schema::bounded(&outcome.result, 128 * 1024 * 1024, 64)?;
    let verified = call.origin() == super::registry::Origin::BuiltIn
        && call.name() == "check"
        && outcome.result["isError"] != true
        && outcome.result["details"]["passed"] == true;
    let native = artifacts.put_evidence(
        &outcome.result.to_string(),
        "tool-native-result",
        &source(&call),
        &[],
        Some(&record.arguments.id),
    )?;
    record.effect = outcome.effect;
    record.native_result = Some(native.clone());
    let producer = if call.origin() == super::registry::Origin::BuiltIn
        && matches!(call.name(), "bash" | "job")
    {
        outcome.result["details"]["fullOutputArtifact"]["id"]
            .as_str()
            .map(|id| {
                result::ProducerArtifact::authorize(artifacts, id, call.name(), call.call_id())
            })
            .transpose()?
    } else {
        None
    };
    let mut hook_replaced_content = false;
    let projected_input = match hooks.after(&call, &outcome.result) {
        Ok(None) => outcome.result,
        Ok(Some(custom)) => {
            if super::schema::bounded(&custom, 128 * 1024 * 1024, 64).is_err() {
                record.post_hook_error =
                    Some("Post-hook output exceeded limits; native result retained".into());
                outcome.result
            } else {
                // Preserve bounded malformed hook output for diagnosis with its own provenance.
                artifacts.put_evidence(
                    &custom.to_string(),
                    "tool-post-hook-result",
                    &json!({"type":"system","name":"extension-post-hook","callId":call.call_id()}),
                    &[],
                    Some(&native.id),
                )?;
                if result::validate(&custom, budget).is_err() {
                    record.post_hook_error =
                        Some("Post-hook output was invalid; native result retained".into());
                    outcome.result
                } else {
                    hook_replaced_content = true;
                    // Hook output cannot forge native inspection/task/runtime evidence.
                    json!({"content":custom["content"],"isError":outcome.result["isError"]==true||custom["isError"]==true,"details":{"extensionDetails":custom.get("details"),"nativeResultArtifact":native,"postHookOutput":true}})
                }
            }
        }
        Err(_) => {
            record.post_hook_error = Some("Post-hook failed; native result retained".into());
            outcome.result
        }
    };
    let projection = result::project_with_producer(
        artifacts,
        call.name(),
        call.call_id(),
        &projected_input,
        budget,
        outcome.artifact_page,
        if hook_replaced_content {
            None
        } else {
            producer.as_ref()
        },
    )?;
    let result_artifact = artifacts.put_evidence(
        &projection.result.to_string(),
        "tool-request-result",
        &json!({"type":"system","name":"core-tool-projection","callId":call.call_id()}),
        &[],
        Some(&native.id),
    )?;
    record.result = Some(result_artifact);
    record.status = Status::Completed;
    record.finished_ms = Some(clock.now_ms().max(record.started_ms));
    if verified {
        store.commit_verified_tool(
            &record.session,
            &key,
            &revision,
            &serde_json::to_vec(&record)
                .map_err(|_| DomainError::InvalidInput("tool record".into()))?,
        )?;
    } else {
        save(store, &key, Some(&revision), &record)?;
    }
    on_event(event(&record, "end"));
    Ok(Outcome {
        record,
        result: projection.result,
        replayed: false,
    })
}

/// The host owns operation and call IDs. Arguments may still be malformed or truncated.
pub struct Invocation<'a> {
    pub operation: &'a str,
    pub call_id: &'a str,
    pub name: &'a str,
    pub arguments: &'a Value,
    pub complete: bool,
}
pub struct Rejection<'a> {
    pub scope: &'a Scope,
    pub reason: &'a DomainError,
    pub clock: &'a dyn crate::Clock,
    pub budget: ResultBudget,
}
/// Validation/policy rejections have durable results too, without an execution intent or adapter call.
pub fn reject(
    store: &impl StoragePort,
    artifacts: &mut impl ExecutionArtifacts,
    invocation: Invocation<'_>,
    rejection: Rejection<'_>,
    on_event: impl FnMut(ToolEvent),
) -> Result<Outcome, DomainError> {
    reject_with_hint(store, artifacts, invocation, rejection, None, on_event)
}
/// Native preparation may add bounded recovery context without changing the
/// rejection reason, emitting an execution intent, or granting retry authority.
pub fn reject_with_hint(
    store: &impl StoragePort,
    artifacts: &mut impl ExecutionArtifacts,
    invocation: Invocation<'_>,
    rejection: Rejection<'_>,
    hint: Option<&str>,
    mut on_event: impl FnMut(ToolEvent),
) -> Result<Outcome, DomainError> {
    if hint.is_some_and(|hint| hint.is_empty() || hint.len() > 8192 || hint.contains('\0')) {
        return Err(DomainError::InvalidInput("tool rejection hint".into()));
    }
    let Invocation {
        operation,
        call_id,
        name,
        arguments,
        complete,
    } = invocation;
    let Rejection {
        scope,
        reason,
        clock,
        budget,
    } = rejection;
    crate::OperationId::new(operation)?;
    super::registry::validate_call_id(call_id)?;
    if name.len() > 128 || !name.is_ascii() {
        return Err(DomainError::InvalidInput("tool name".into()));
    }
    super::schema::bounded(arguments, 8 * 1024 * 1024, 64)?;
    let identity = digest(&json!([
        "rejected",
        operation,
        call_id,
        name,
        arguments,
        complete,
        scope.workspace,
        scope.session,
        scope.epoch,
        scope.revision
    ]));
    let key = key(&scope.workspace, &scope.session, operation);
    if let Some((_, record)) = load(store, &key)? {
        if record.identity != identity {
            return Err(DomainError::Conflict);
        }
        return receipt(artifacts, record);
    }
    let arguments = artifacts.put_evidence(
        &arguments.to_string(),
        "tool-rejected-arguments",
        &json!({"type":"system","name":"tool-validation","callId":call_id}),
        &[],
        None,
    )?;
    let message = match reason {
        DomainError::InvalidInput(message)
        | DomainError::Unavailable(message)
        | DomainError::LimitExceeded(message) => message.as_str(),
        DomainError::PermissionDenied => "Tool execution was blocked by policy",
        DomainError::Cancelled => "Tool execution was cancelled before it started",
        DomainError::Conflict => "Tool execution conflicted with current state; prepare a new call",
        DomainError::NotFound => "Tool target was not found",
        DomainError::TimedOut => "Tool execution expired before it started",
        _ => "Tool preparation failed; no execution was started",
    };
    let message = hint
        .map(|hint| format!("{message}\n{hint}"))
        .unwrap_or_else(|| message.into());
    let result = json!({"content":[{"type":"text","text":message}],"isError":true,"details":{"executionEffect":"none","rejectedBeforeExecution":true}});
    let projection = result::project(artifacts, name, call_id, &result, budget, false)?;
    let response = artifacts.put_evidence(
        &projection.result.to_string(),
        "tool-request-result",
        &json!({"type":"system","name":"tool-rejection","callId":call_id}),
        &[],
        Some(&arguments.id),
    )?;
    let now = clock.now_ms();
    let record = Record {
        version: 1,
        identity,
        operation: operation.into(),
        call_id: call_id.into(),
        tool: name.into(),
        workspace: scope.workspace.clone(),
        session: scope.session.clone(),
        epoch: scope.epoch,
        revision: scope.revision,
        policy_revision: 0,
        target_revision: String::new(),
        mcp: None,
        arguments,
        status: Status::Rejected,
        effect: Effect::None,
        started_ms: now,
        finished_ms: Some(now),
        native_result: None,
        result: Some(response),
        post_hook_error: None,
    };
    save(store, &key, None, &record)?;
    on_event(event(&record, "end"));
    Ok(Outcome {
        record,
        result: projection.result,
        replayed: false,
    })
}

/// Parent-authorized idempotency lookup, before preparing a mutation against files that may already have changed.
/// The artifact port enforces requester/session access. This is not a renderer metadata API.
pub fn replay(
    store: &impl StoragePort,
    artifacts: &impl ExecutionArtifacts,
    call: &ValidatedCall,
    scope: &Scope,
) -> Result<Option<Outcome>, DomainError> {
    let key = key(&scope.workspace, &scope.session, call.operation());
    load(store, &key)?
        .map(|(_, record)| {
            if record.identity != identity(call, scope) {
                return Err(DomainError::Conflict);
            }
            receipt(artifacts, record)
        })
        .transpose()
}
