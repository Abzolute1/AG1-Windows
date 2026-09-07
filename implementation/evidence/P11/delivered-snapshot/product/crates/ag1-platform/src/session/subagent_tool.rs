//! P09 capability wrapper for native foreground delegation. Model arguments
//! cannot supply providers, tool services, credentials or a child executor.
use super::{
    NativeSession, ProviderResolver,
    files::NativeToolRunner,
    subagent::{Request, Services},
    tool_journal::ToolJournal,
};
use crate::file_tools::FileExecution;
use ag1_core::{
    DomainError,
    context::ContextArtifactPort,
    runtime::subagent::{self as domain, Profile},
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        executor::{AdapterOutcome, Effect},
        registry::{Origin, Scope, ValidatedCall},
    },
};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Weak, atomic::Ordering},
    time::{Duration, Instant},
};

pub struct DelegationService {
    parent: Weak<NativeSession>,
    template: Arc<NativeToolRunner>,
    provider: Arc<dyn ProviderResolver>,
    models: Vec<Value>,
    runtime: tokio::runtime::Handle,
}
pub struct PreparedDelegation {
    call: ValidatedCall,
    scope: Scope,
    target: String,
    journal: Arc<ToolJournal>,
    request: Request,
}
impl PreparedDelegation {
    pub fn target_revision(&self) -> &str {
        &self.target
    }
}
impl DelegationService {
    pub fn new(
        parent: &Arc<NativeSession>,
        template: Arc<NativeToolRunner>,
        provider: Arc<dyn ProviderResolver>,
        models: Vec<Value>,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self, DomainError> {
        if parent.delegated
            || template.adapters.delegation.is_some()
            || models.len() > 1024
            || !Arc::ptr_eq(&template.store, &parent.store)
            || template.origin != parent.origin
        {
            return Err(DomainError::PermissionDenied);
        }
        domain::routing::validate_catalog(&models)?;
        Ok(Self {
            parent: Arc::downgrade(parent),
            template,
            provider,
            models,
            runtime,
        })
    }
    pub fn prepare(
        &self,
        call: &ValidatedCall,
        scope: &Scope,
        journal: Option<&Arc<ToolJournal>>,
    ) -> Result<PreparedDelegation, DomainError> {
        let parent = self.parent.upgrade().ok_or(DomainError::Cancelled)?;
        let session = parent.state()?.stamp().session().as_str().to_owned();
        if call.name() != "task"
            || call.origin() != Origin::BuiltIn
            || scope.workspace != parent.workspace
            || scope.session != session
        {
            return Err(DomainError::PermissionDenied);
        }
        let journal = journal
            .ok_or_else(|| {
                DomainError::Unavailable("delegation requires the native journal".into())
            })?
            .clone();
        journal.snapshot(scope)?;
        let args = call.args();
        let profile: Profile = serde_json::from_value(args["type"].clone())
            .map_err(|_| DomainError::InvalidInput("delegated profile".into()))?;
        let request = Request {
            operation: call.operation().into(),
            description: args["description"]
                .as_str()
                .ok_or(DomainError::Conflict)?
                .into(),
            profile,
            prompt: args["prompt"].as_str().ok_or(DomainError::Conflict)?.into(),
            model: args["model"].as_str().map(str::to_owned),
            max_turns: args["maxTurns"]
                .as_f64()
                .filter(|n| n.is_finite() && n.floor() >= 1.)
                .map(|n| n.floor().min(domain::MAX_TURNS as f64) as u64),
            timeout: args["timeout"].as_f64().map(|n| n.max(1.)),
            token_budget: None,
            resume_from: args["resumeFrom"].as_str().map(str::to_owned),
        };
        if let Some(id) = &request.resume_from {
            domain::checkpoint_id(id)?;
        }
        // Resolve availability during preparation, before recording P09 intent.
        domain::routing::select(
            profile,
            request.model.as_deref(),
            &parent.configuration()?.model,
            &self.models,
        )?;
        Ok(PreparedDelegation {
            call: call.clone(),
            scope: scope.clone(),
            target: format!(
                "delegation:{}:{}:{}",
                parent.incarnation, scope.epoch, scope.revision
            ),
            journal,
            request,
        })
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        plan: PreparedDelegation,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
        artifacts: &mut impl ContextArtifactPort,
    ) -> Result<AdapterOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if scope != &plan.scope
            || authorization.call().args_hash() != plan.call.args_hash()
            || authorization.call().operation() != plan.call.operation()
        {
            return Err(DomainError::Conflict);
        }
        if cancel.load(Ordering::Acquire) {
            return Err(DomainError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(DomainError::TimedOut);
        }
        plan.journal.snapshot(scope)?;
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &plan.target,
                now: now(),
            },
        )?;
        let parent = self.parent.upgrade().ok_or(DomainError::Cancelled)?;
        let result=self.runtime.block_on(async {
            let child=parent.start_subagent(plan.request,Services {tools:self.template.clone(),provider:self.provider.clone(),models:self.models.clone()})?;
            let cancellation=child.cancellation();
            let finish=child.finish();tokio::pin!(finish);
            let mut control_error=None;
            let mut next_progress=Instant::now();
            loop {
                tokio::select! {
                    result=&mut finish=>break result.map(|result|(result,control_error)),
                    _=tokio::time::sleep(Duration::from_millis(10))=>{
                        if let Err(error)=plan.journal.service_controls() {control_error.get_or_insert(error);}
                        if Instant::now() >= next_progress {
                            match cancellation.progress() {
                                Ok(progress) => plan.journal.delegation_progress(&call, &progress),
                                Err(error) => {control_error.get_or_insert(error);}
                            }
                            next_progress = Instant::now() + Duration::from_millis(250);
                        }
                        if cancel.load(Ordering::Acquire) || Instant::now()>=deadline || control_error.is_some() {cancellation.cancel();}
                    }
                }
            }
        })?;
        let (result, control_error) = result;
        let raw = json!(&result);
        let original = artifacts.put_evidence(
            &raw.to_string(),
            "subagent-result",
            &json!({"type":"tool","name":"task","callId":call.call_id()}),
            &[],
            None,
        )?;
        if !result.settled || control_error.is_some() {
            return Err(DomainError::Unavailable(format!(
                "Delegated outcome requires inspection; retained artifact {}",
                original.id
            )));
        }
        // Accounting is accepted persistence, not a best-effort observer. Child
        // custody survives a failed parent roll-up and the tool cannot replay it.
        let mut usage = result.usage.clone();
        usage["cost"] =
            json!({"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":result.usage["cost"]});
        let rollup = json!({"description":result.description,"type":result.profile,"model":result.model,"state":result.state,"turns":result.turns,"durationMs":result.duration_ms,"usage":usage,"evidence":result.evidence,"error":result.error,"childSession":result.child_session,"checkpointId":result.checkpoint_id,"resultArtifact":original});
        let mut current = scope.clone();
        current.revision = plan.journal.revision()?;
        plan.journal
            .commit_state(&current, &call, "ag1.subagent_task", rollup)
            .map_err(DomainError::from)?;
        Ok(AdapterOutcome {
            effect: if result.profile.readonly() {
                Effect::None
            } else {
                Effect::Unknown
            },
            result: json!({"content":[{"type":"text","text":domain::tool::render(&raw)?}],"details":raw,"isError":result.state==domain::checkpoint::State::Failed}),
            artifact_page: false,
        })
    }
}
