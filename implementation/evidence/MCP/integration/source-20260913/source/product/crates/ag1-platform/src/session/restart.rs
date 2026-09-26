//! Explicit host restart inspection. No stored PID, receipt or model message
//! authorizes process control or repeats a possibly executed external effect.
use super::*;
use ag1_core::{context::ContextArtifact, tools::executor::ExecutionArtifacts};
use ag1_storage::{InterruptedCommand, context::ContextStore};
use serde::Serialize;
#[derive(Serialize)]
pub struct InspectionPage {
    pub journal_revision: u64,
    pub pending_calls: Vec<Value>,
    pub tool_records: Vec<ag1_core::tools::executor::Record>,
    pub interrupted_commands: Vec<InterruptedCommand>,
}
pub struct ReconcileCall {
    pub command: String,
    pub call_id: String,
    pub expected_revision: u64,
    pub timestamp: String,
    /// Host-authored, integrity-checked observation that prior execution has
    /// ended. A known result can be cited, but recovery never invents success.
    pub inspection: ContextArtifact,
}
impl NativeSession {
    pub fn restart_required(&self) -> bool {
        self.restart_required.load(Ordering::Acquire)
    }
    pub async fn inspect_restart(
        self: &Arc<Self>,
        after_tool: Option<String>,
        after_command: Option<String>,
        limit: usize,
    ) -> Result<InspectionPage, DomainError> {
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        self.pool
            .submit(move || {
                let _activity = activity;
                let (stamp, revision, pending) = {
                    let state = runtime.state()?;
                    (
                        state.stamp().clone(),
                        state.journal_revision(),
                        state
                            .pending_tools()
                            .iter()
                            .map(|c| c.block.clone())
                            .collect(),
                    )
                };
                let tool_records = runtime.store.session_tool_records(
                    &runtime.workspace,
                    stamp.session().as_str(),
                    after_tool.as_deref(),
                    limit,
                )?;
                let interrupted_commands = runtime.store.interrupted_commands(
                    &runtime.workspace,
                    stamp.session().as_str(),
                    after_command.as_deref(),
                    limit,
                )?;
                let state = runtime.state()?;
                if state.stamp() != &stamp || state.journal_revision() != revision {
                    return Err(DomainError::Conflict);
                }
                Ok(InspectionPage {
                    journal_revision: revision,
                    pending_calls: pending,
                    tool_records,
                    interrupted_commands,
                })
            })
            .await
    }
    fn restart_idle(&self) -> Result<(), DomainError> {
        if self
            .active
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .is_some()
            || self
                .compacting
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .is_some()
            || self.pending_transport()?.is_some()
            || !self.background_work_settled()?
            || self
                .tool_owner
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .as_ref()
                .and_then(std::sync::Weak::upgrade)
                .is_some()
            || !self.persistence_healthy()?
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    /// Host-only: call after establishing that previous execution has ended and
    /// inspecting its possible effects. This publishes one explicit error result
    /// in source order. It never executes an adapter or edits the old receipt.
    pub async fn reconcile_interrupted_call(
        self: &Arc<Self>,
        input: ReconcileCall,
        listener: Arc<dyn Listener>,
    ) -> Result<(), DomainError> {
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        self.pool.submit(move ||{
            let _activity=activity;runtime.restart_idle()?;
            OperationId::new(&input.command)?;
            let timestamp=chrono::DateTime::parse_from_rfc3339(&input.timestamp).map_err(|_|DomainError::InvalidInput("recovery timestamp".into()))?.timestamp_millis();
            let (session,call)={let state=runtime.state()?;
                if state.journal_revision()!=input.expected_revision || state.pending_commit(){return Err(DomainError::Conflict);}
                let call=state.pending_tools().front().filter(|c|c.id==input.call_id).cloned().ok_or(DomainError::Conflict)?;
                (state.stamp().session().clone(),call)
            };
            let artifacts=ContextStore::new(&runtime.store,session.clone(),&runtime.origin,&input.command,&input.timestamp,Default::default())?;
            let inspection=artifacts.read_json(&input.inspection)?;
            validate_inspection(&inspection,&runtime.workspace,session.as_str(),"callId",&input.call_id)?;
            let message=json!({"role":"toolResult","toolCallId":call.id,"toolName":call.name,"content":[{"type":"text","text":"The host was interrupted before this tool's transcript outcome was acknowledged. Prior execution has been inspected and ended; effects remain unknown. Inspect the retained evidence before deciding further work. This call was not replayed."}],"details":{"nativeRecovery":{"effect":"unknown","executionSettled":true,"inspection":input.inspection}},"isError":true,"timestamp":timestamp});
            let action={let mut state=runtime.state()?;if state.journal_revision()!=input.expected_revision{return Err(DomainError::Conflict);}state.append_message(&input.command,&input.command,&input.timestamp,message)?};
            let mut wait=CommitWait{state:runtime.state.clone(),changed:runtime.changed.clone(),expected_revision:input.expected_revision,received:false};
            let receipt=action.execute(runtime.store.as_ref());
            let result=runtime.acknowledge_stamped(Ok(receipt),listener.as_ref());wait.received=true;
            result.map(|_|())
        }).await
    }
    /// Close one inspected orphan command as failed. The immutable inspection
    /// remains referenced by a separate metadata receipt, including on lost ACK.
    /// A host must resolve pending tool calls before releasing this barrier.
    pub async fn reconcile_interrupted_command(
        self: &Arc<Self>,
        key: String,
        inspection: ContextArtifact,
        timestamp: String,
    ) -> Result<(), DomainError> {
        let activity = self.enter_owned()?;
        let runtime = self.clone();
        self.pool.submit(move ||{
            let _activity=activity;runtime.restart_idle()?;
            if !runtime.restart_required(){return Err(DomainError::Conflict);}
            let (session,revision,epoch)={let state=runtime.state()?;if !state.pending_tools().is_empty() || !state.settled(){return Err(DomainError::Conflict);}(state.stamp().session().clone(),state.journal_revision(),state.stamp().epoch())};
            OperationId::new(&key)?;
            let old=runtime.store.command_result(&key)?.ok_or(DomainError::NotFound)?;
            let intent:Value=serde_json::from_slice(&old.intent).map_err(|_|DomainError::InvalidInput("interrupted intent".into()))?;
            if intent["kind"]!="native-session-command" || intent["session"]!=session.as_str() || intent["payload"]["scope"]["workspace"]!=runtime.workspace || intent["payload"]["scope"]["session"]!=session.as_str(){return Err(DomainError::PermissionDenied);}
            let artifacts=ContextStore::new(&runtime.store,session.clone(),&runtime.origin,&key,&timestamp,Default::default())?;
            validate_inspection(&artifacts.read_json(&inspection)?,&runtime.workspace,session.as_str(),"commandKey",&key)?;
            let audit=json!({"version":1,"command":key,"intentSha256":ag1_storage::digest(&old.intent),"inspection":inspection,"disposition":"interrupted_unknown"});
            use ag1_core::storage::StoragePort;
            let audit_key=format!("restart-{}",ag1_storage::digest(key.as_bytes()));
            if let Some(saved)=runtime.store.load_metadata("runtime-interruption",&audit_key)?{
                if serde_json::from_slice::<Value>(&saved.bytes).ok()!=Some(audit.clone()){return Err(DomainError::Conflict);}
            }else{runtime.store.commit_metadata("runtime-interruption",&audit_key,None,&serde_json::to_vec(&audit).map_err(|_|DomainError::Conflict)?)?;}
            let terminal=json!({"version":1,"operation":{"operation":key,"status":"failed"},"journal_revision":revision,"context_epoch":epoch,"execution_settled":true,"generations":null});
            if old.state=="uncertain" {
                runtime.store.settle_intent(&key,&old.intent,&serde_json::to_vec(&terminal).map_err(|_|DomainError::Conflict)?)?;
            }else if old.state!="completed" || old.result.as_deref().and_then(|b|serde_json::from_slice::<Value>(b).ok()).is_none_or(|value|value["operation"]["status"]!="failed" || value["execution_settled"]!=true){return Err(DomainError::Conflict);}
            let more=!runtime.store.interrupted_commands(&runtime.workspace,session.as_str(),None,1)?.is_empty();
            runtime.restart_required.store(more,Ordering::Release);
            runtime.changed.notify_waiters();Ok(())
        }).await
    }
}
fn validate_inspection(
    value: &Value,
    workspace: &str,
    session: &str,
    key: &str,
    id: &str,
) -> Result<(), DomainError> {
    if value["version"] != 1
        || value["workspace"] != workspace
        || value["session"] != session
        || value[key] != id
        || value["executionSettled"] != true
        || value["observations"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
    {
        return Err(DomainError::PermissionDenied);
    }
    Ok(())
}
