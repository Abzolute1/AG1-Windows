//! P09 native tool execution for the session loop. Read-only runs execute in
//! bounded groups; mutations form ordered barriers and retain FileService's
//! per-file admission permit through approval and publication.
use super::{
    ToolOutcome, ToolRunner, ToolSelection,
    adapters::{AdapterContext, NativeAdapters, PreparedAdapter},
};
use crate::file_tools::FileService;
use ag1_core::{
    Clock, DomainError, SessionId,
    context::features::Features,
    runtime::session::PendingCall,
    tools::{
        approval::{
            ApprovalRequest, AuthorizedCall, Authorizer, ExecutionFence, Preflight,
            PreflightContext,
        },
        executor::{
            self, AdapterOutcome, Effect, Execution, ExecutionArtifacts, Hooks, Invocation,
            Rejection,
        },
        policy::Policy,
        registry::{Registry, Scope},
        result::ResultBudget,
    },
};
use ag1_storage::{Store, context::ContextStore};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

pub trait ApprovalHandler: Send + Sync {
    fn supports_commands(&self) -> bool {
        false
    }
    fn pending(&self) -> Result<Vec<ApprovalRequest>, DomainError> {
        Ok(Vec::new())
    }
    fn answer(&self, _: &ag1_core::tools::approval::Binding, _: bool) -> Result<(), DomainError> {
        Err(DomainError::Unavailable("approval commands".into()))
    }
    fn approve(&self, request: &ApprovalRequest, cancel: &AtomicBool) -> Result<bool, DomainError>;
}
pub struct RejectApproval;
impl ApprovalHandler for RejectApproval {
    fn approve(&self, _: &ApprovalRequest, _: &AtomicBool) -> Result<bool, DomainError> {
        Ok(false)
    }
}
#[derive(Clone)]
pub struct NativeToolRunner {
    pub files: Arc<FileService>,
    pub adapters: NativeAdapters,
    pub registry: Arc<Registry>,
    pub policy: Policy,
    pub policy_revision: u64,
    pub approvals: Arc<dyn ApprovalHandler>,
    pub store: Arc<Store>,
    pub origin: String,
    pub timestamp: String,
    pub clock: Arc<dyn Clock>,
    /// Explicit host tool deadline, independent of provider reasoning limits.
    pub tool_timeout: Duration,
}
pub type FileRunner = NativeToolRunner;

enum Prepared {
    Ready {
        plan: Box<PreparedAdapter>,
        authorization: Box<AuthorizedCall>,
        deadline: Instant,
    },
    Rejected(DomainError),
}
struct Work {
    call: PendingCall,
    operation: String,
    prepared: Prepared,
}
#[derive(Clone, Copy)]
struct Current<'a> {
    uncertain: bool,
    registry: &'a Registry,
    scope: &'a Scope,
    cancel: &'a AtomicBool,
    revision: &'a AtomicU64,
    selection: &'a ToolSelection,
}
impl Current<'_> {
    fn check(self) -> Result<(), DomainError> {
        if let Some(journal) = &self.selection.journal {
            journal.service_controls()?;
        }
        if self.uncertain {
            Err(DomainError::DurabilityUnknown)
        } else if self.cancel.load(Ordering::Acquire) {
            Err(DomainError::Cancelled)
        } else if self.revision.load(Ordering::Acquire) != self.scope.revision
            || self
                .selection
                .journal
                .as_ref()
                .is_some_and(|journal| !journal.valid())
        {
            Err(DomainError::Conflict)
        } else {
            Ok(())
        }
    }
}
impl NativeToolRunner {
    fn capability_status(&self, registry: &Registry, args: &Value) -> Result<Value, DomainError> {
        use ag1_core::tools::{policy, surface};
        let available = self.adapters.available_names();
        let tools: Vec<_> = registry.native_tools().into_iter().filter(|t|available.iter().any(|n| t["name"]==*n)).collect();
        let permissions = json!({"mode":self.policy.mode,"policyRevision":self.policy_revision,"folders":self.adapters.folders.as_ref().map(|f|&f.selection),"shell":self.adapters.shell.as_ref().map(|s|s.boundary_status()),"metadataGrantsAuthority":false});
        match args["action"].as_str() {
            Some("list") => Ok(json!({"permissions":permissions,"tools":tools.iter().map(|t|json!({"name":t["name"],"advanced":surface::ADVANCED.contains(&t["name"].as_str().unwrap_or("")),"effects":t["effects"]})).collect::<Vec<_>>()})),
            Some("schema") => {
                let name=args["name"].as_str().ok_or_else(||DomainError::InvalidInput("capability schema requires name".into()))?;
                let tool=tools.iter().find(|t|t["name"]==name).ok_or(DomainError::NotFound)?;
                let decision=if let Some(input)=args.get("arguments") {
                    let definition=registry.definitions().find(|d|d.metadata["name"]==name).ok_or(DomainError::NotFound)?;
                    let input=definition.schema.coerce(input)?;
                    let (canonical,input)=surface::route(name,&input,&[])?.unwrap_or((name.into(),input));
                    Some(policy::evaluate_in_folders(&canonical,&input,&self.policy,true,&self.adapters.folders.as_ref().map(|f|f.selection.folders.iter().filter(|g|g.available).map(|g|g.path.display().to_string()).collect::<Vec<_>>()).unwrap_or_default())?)
                } else { None };
                Ok(json!({"tool":tool,"permissions":permissions,"policyDecision":decision,"preparedTargetChecks":"Still enforced at execution; discovery does not reserve approval or grant access."}))
            }
            _ => Err(DomainError::InvalidInput("capability requires list, schema, or run with a discovered specialist name and arguments".into())),
        }
    }

    fn review_changes(args: &Value) -> String {
        // Construct only a bounded display from the normalized, bound arguments.
        // The complete arguments remain in the assistant's native tool record.
        let mut parts = Vec::new();
        if let Some(edits) = args["edits"].as_array() {
            parts.push(format!("{} ordered edits; showing a bounded excerpt:\n", edits.len()));
            for edit in edits.iter().take(16) {
                let old: String = edit["oldText"].as_str().unwrap_or("").chars().take(1500).collect();
                let new: String = edit["newText"].as_str().unwrap_or("").chars().take(1500).collect();
                parts.push(format!("Old text:\n{old}\nReplacement:\n{new}"));
                if parts.iter().map(String::len).sum::<usize>() >= 3000 { break; }
            }
        } else if let Some(content) = args["content"].as_str() {
            parts.push(format!("Proposed content ({} bytes; bounded excerpt):\n{}", content.len(), content.chars().take(3000).collect::<String>()));
        }
        parts.join("\n\n").chars().take(3000).collect()
    }
    fn execution_budget(&self, call: &PendingCall) -> Duration {
        // Delegation has its own bounded child lifetime and must include time
        // waiting for a local inference slot. Ordinary tools retain their limit.
        match call.name.as_str() {
            "task" => Duration::from_secs_f64(call.block["arguments"]["timeout"].as_f64()
                .filter(|v| v.is_finite() && *v > 0.).unwrap_or(300.).clamp(1., 86400.) + 30.),
            "explore" => Duration::from_secs(180 * call.block["arguments"]["questions"].as_array().map_or(1, |q| q.len().clamp(1, 4)) as u64 + 30),
            _ => self.tool_timeout,
        }
    }
    fn prepare(
        &self,
        current: Current<'_>,
        call: &PendingCall,
        allowed: bool,
        serialized: bool,
        operation: String,
        authorizer: &mut Authorizer,
    ) -> Work {
        let scope = current.scope;
        let cancel = current.cancel;
        let prepared = (|| {
            current.check()?;
            if !allowed {
                return Err(DomainError::PermissionDenied);
            }
            if serialized {
                return Err(DomainError::InvalidInput("Controlled runtime: multiple tool calls in one assistant turn are blocked. Wait for the previous tool result and continue with exactly one next action.".into()));
            }
            if cancel.load(Ordering::Acquire) {
                return Err(DomainError::Cancelled);
            }
            if let Some(journal) = &current.selection.journal
                && let Some(reason) = journal.repetition.stop_reason()? {
                return Err(DomainError::InvalidInput(reason));
            }
            let mut valid = current.registry.prepare(
                &operation,
                &call.id,
                &call.name,
                &call.block["arguments"],
                call.block["_argumentsTruncated"] != true,
            )?;
            let branch = current.selection.journal.as_ref().map(|j| j.snapshot(scope)).transpose()?;
            if valid.origin() == ag1_core::tools::registry::Origin::BuiltIn
                && let Some((name, args)) = ag1_core::tools::surface::route(valid.name(), valid.args(), branch.as_deref().map_or(&[], |b| b.as_slice()))? {
                valid = current.registry.prepare(&operation, &call.id, &name, &args, true)?;
            }
            if let Some(controlled) = current
                .selection
                .journal
                .as_ref()
                .and_then(|j| j.controlled.as_ref())
            {
                let args = valid.args();
                let path = super::controlled::path(args)
                    .map(|path| self.files.relative_path(path))
                    .transpose()?;
                let decision = controlled
                    .lock()
                    .map_err(|_| DomainError::Conflict)?
                    .evaluate(valid.name(), args, path.as_deref())?;
                if let Some(reason) = decision.rejection() {
                    return Err(DomainError::InvalidInput(reason));
                }
            }
            let deadline = Instant::now()
                .checked_add(self.tool_timeout)
                .ok_or(DomainError::Conflict)?;
            if let Some(extensions) = &self.adapters.extensions
                && executor::recover(&*self.store, &scope.workspace, &scope.session, &operation)?.is_none() {
                let journal = current.selection.journal.as_deref().ok_or_else(|| DomainError::Unavailable("extension hooks require the native journal".into()))?;
                if valid.origin()==ag1_core::tools::registry::Origin::Extension {
                    let args=extensions.prepare_tool(journal,current.registry,scope,&valid,&crate::resource_fs::ScanControl{cancel,deadline})?;
                    valid=current.registry.prepare(&operation,&call.id,&call.name,&args,true)?;
                }
                let result = extensions.tool_event(journal, current.registry, scope,
                    json!({"type":"tool_call","toolName":valid.name(),"toolCallId":valid.call_id(),"input":valid.args()}),
                    &crate::resource_fs::ScanControl { cancel, deadline })?;
                if result.as_ref().is_some_and(|result| result["block"] == true) { return Err(DomainError::PermissionDenied); }
                current.check()?;
            }
            let plan = if valid.name() == "capability" && valid.origin() == ag1_core::tools::registry::Origin::BuiltIn {
                PreparedAdapter::Capability(self.capability_status(current.registry, valid.args())?)
            } else { self.adapters.prepare(
                &self.files,
                &valid,
                AdapterContext {
                    policy_mode: self.policy.mode,                    journal: current.selection.journal.as_ref(),
                    registry: current.registry,
                    scope,
                    policy_revision: self.policy_revision,
                    clock: &*self.clock,
                    profile: &current.selection.research_profile,
                    loaded_memory: &current.selection.loaded_memory,
                    cancel,
                    deadline,
                },
            )? };
            let valid = plan.transformed_call().cloned().unwrap_or(valid);
            current.check()?;
            if Instant::now() >= deadline {
                return Err(DomainError::TimedOut);
            }
            let target = plan.target_revision();
            let changes = Self::review_changes(valid.args());
            let mut policy = self.policy.clone();
            if let Some(folders) = &self.adapters.folders
                && ["write", "edit", "append", "editApply"].contains(&valid.name())
                && !(valid.name() == "edit" && valid.args()["mode"] == "preview")
            {
                let path = valid.args()["path"].as_str().or_else(||valid.args()["file_path"].as_str()).unwrap_or("");
                policy.cwd = folders.resolve(path, true)?.root().to_str().ok_or(DomainError::PermissionDenied)?.into();
            }
            let folder_paths = self.adapters.folders.as_ref().map(|f|f.selection.folders.iter().filter(|g|g.available).map(|g|g.path.display().to_string()).collect::<Vec<_>>()).unwrap_or_default();
            let authorization = match authorizer.preflight_in_folders(
                valid,
                PreflightContext {
                    scope,
                    policy: &policy,
                    policy_revision: self.policy_revision,
                    target_revision: &target,
                    now: self.clock.now_ms(),
                    ttl: 300_000,
                    untrusted_extensions: true,
                },
                &folder_paths,
            )? {
                Preflight::Ready(authorization) => authorization,
                Preflight::Rejected(reason) => return Err(DomainError::InvalidInput(reason)),
                Preflight::Approval(mut request) => {
                    if let PreparedAdapter::GitHub {plan,..} = &plan {
                        request.display["summary"] = json!("Review exact GitHub request");
                        request.display["details"] = json!(plan.approval_details());
                    }
                    if let PreparedAdapter::LocalGit {plan,..} = &plan {
                        request.display["summary"]=json!("Review local Git change");
                        request.display["kind"]=json!("native_local_git");
                        request.display["details"]=json!(serde_json::to_string_pretty(&plan.preview()).map_err(|_|DomainError::Conflict)?);
                    }
                    if !changes.is_empty() {
                        request.display["changes"] = json!(changes);
                    }
                    let approved = self.approvals.approve(&request, cancel)?;
                    current.check()?;
                    if cancel.load(Ordering::Acquire) {
                        return Err(DomainError::Cancelled);
                    }
                    if approved && let PreparedAdapter::File { service, plan } = &plan {
                        service.revalidate_review(plan)?;
                    }
                    authorizer
                        .answer_for_execution(
                            &request.binding,
                            approved,
                            current.registry,
                            ExecutionFence {
                                scope,
                                policy_revision: self.policy_revision,
                                target_revision: &target,
                                now: self.clock.now_ms(),
                            },
                            self.execution_budget(call).as_millis().try_into().map_err(|_| DomainError::Conflict)?,
                        )?
                        .ok_or(DomainError::PermissionDenied)?
                }
            };
            if let Some(journal) = &current.selection.journal {
                let (reason, events) = journal.repetition.before(
                    &call.name,
                    &call.id,
                    authorization.call().args(),
                    self.clock.now_ms(),
                )?;
                journal.repetition_events(events);
                if let Some(reason) = reason {
                    return Err(DomainError::InvalidInput(reason));
                }
            }
            Ok(Prepared::Ready {
                plan: Box::new(plan),
                authorization: Box::new(authorization),
                deadline: Instant::now().checked_add(self.execution_budget(call)).ok_or(DomainError::Conflict)?,
            })
        })()
        .unwrap_or_else(Prepared::Rejected);
        Work {
            call: call.clone(),
            operation,
            prepared,
        }
    }
    fn execute_one(
        &self,
        current: Current<'_>,
        mut work: Work,
    ) -> Result<ToolOutcome, DomainError> {
        let scope = current.scope;
        let cancel = current.cancel;
        if matches!(work.prepared, Prepared::Ready { .. })
            && let Err(error) = current.check()
        {
            work.prepared = Prepared::Rejected(error);
        }
        let mut artifacts = ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            &self.origin,
            &work.operation,
            &self.timestamp,
            Features::default(),
        )?;
        let shell = matches!(&work.prepared, Prepared::Ready {plan,..} if matches!(plan.as_ref(),PreparedAdapter::Shell {..}));
        let mut native_settled = false;
        let outcome = match work.prepared {
            Prepared::Rejected(reason) => executor::reject(
                &*self.store,
                &mut artifacts,
                Invocation {
                    operation: &work.operation,
                    call_id: &work.call.id,
                    name: &work.call.name,
                    arguments: &work.call.block["arguments"],
                    complete: work.call.block["_argumentsTruncated"] != true,
                },
                Rejection {
                    scope,
                    reason: &reason,
                    clock: &*self.clock,
                    budget: ResultBudget::default(),
                },
                |_| {},
            ),
            Prepared::Ready {
                plan,
                authorization,
                deadline,
            } => {
                let target = plan.target_revision().to_owned();
                executor::execute_with_artifacts(
                    &*self.store,
                    &mut artifacts,
                    *authorization,
                    Execution {
                        registry: current.registry,
                        fence: ExecutionFence {
                            scope,
                            policy_revision: self.policy_revision,
                            target_revision: &target,
                            now: self.clock.now_ms(),
                        },
                        budget: ResultBudget::default(),
                        clock: &*self.clock,
                    },
                    &mut RuntimeHooks {
                        registry: current.registry,
                        journal: current.selection.journal.as_deref(),
                        clock: &*self.clock,
                        extensions: self.adapters.extensions.as_deref(),
                        scope,
                        cancel,
                        deadline,
                    },
                    |authorization, artifacts| {
                        if let Err(error) = current.check() {
                            return Ok(AdapterOutcome {
                                effect: Effect::None,
                                result: json!({"content":[{"type":"text","text":format!("Tool cancelled before execution: {error:?}")}],"isError":true,"details":{"executionEffect":"none","rejectedBeforeExecution":true}}),
                                artifact_page: false,
                            });
                        }
                        let may_mutate = plan.may_mutate();
                        match (*plan).execute(
                            authorization,
                            AdapterContext {
                    policy_mode: self.policy.mode,                                journal: current.selection.journal.as_ref(),
                                registry: current.registry,
                                scope,
                                policy_revision: self.policy_revision,
                                clock: &*self.clock,
                                profile: &current.selection.research_profile,
                                loaded_memory: &current.selection.loaded_memory,
                                cancel,
                                deadline,
                            },
                            artifacts,
                        ) {
                            Ok(outcome) => {
                                native_settled = true;
                                Ok(outcome)
                            }
                            Err(error) => Ok(AdapterOutcome {
                                effect: if may_mutate {
                                    Effect::Unknown
                                } else {
                                    Effect::None
                                },
                                result: json!({"content":[{"type":"text","text":format!("Tool execution failed: {error:?}{}",if may_mutate {". Inspect the target before retrying."} else {""})}],"isError":true,"details":{"executionEffect":if may_mutate {"unknown"}else{"none"}}}),
                                artifact_page: false,
                            }),
                        }
                    },
                    |_| {},
                )
            }
        }?;
        let mut execution_settled = native_settled || outcome.record.effect != Effect::Unknown;
        if outcome.replayed && shell && !execution_settled {
            let native = artifacts.read_json(
                outcome
                    .record
                    .native_result
                    .as_ref()
                    .ok_or(DomainError::DurabilityUnknown)?,
            )?;
            execution_settled = matches!(
                native["details"]["termination"].as_str(),
                Some("exit" | "signal" | "cancelled" | "timeout" | "output_limit")
            ) && native["details"]["rawOutputArtifact"]["id"].is_string();
        }
        Ok(ToolOutcome {
            outcome,
            execution_settled,
        })
    }
}
impl ToolRunner for NativeToolRunner {
    #[cfg(target_os = "linux")]
    fn client_mcp(&self) -> Option<Arc<crate::mcp::service::Session>> { self.adapters.mcp_management.clone() }
    #[cfg(target_os = "linux")]
    fn mcp_request_runner(&self,scope:&Scope)->Result<Option<Arc<dyn ToolRunner>>,DomainError> {
        let Some(discovery)=&self.adapters.mcp_discovery else { return Ok(None); };
        let mut runner=self.clone();
        let mut registry=self.registry.as_ref().clone();
        runner.adapters.mcp=Some(Arc::new(discovery.capture(scope,&mut registry)?));
        runner.registry=Arc::new(registry);
        Ok(Some(Arc::new(runner)))
    }
    #[cfg(target_os = "linux")]
    fn mcp_request_definitions(&self)->Option<Vec<ag1_core::tools::mcp::SelectedDefinition>> {
        self.adapters.mcp_discovery.as_ref().map(|_|self.registry.mcp_selection())
    }

    #[cfg(target_os = "linux")]
    fn prepare_mcp_request(&self,scope:&Scope,tools:&[Value])->Result<Vec<Value>,DomainError> {
        let valid=self.adapters.mcp.as_ref().is_some_and(|mcp|mcp.validate(scope,&self.registry).is_ok());
        let catalog=self.registry.public_tools();
        let dynamic=self.adapters.mcp_discovery.is_some();
        let mut view:Vec<_>=tools.iter().filter_map(|tool| {
            if dynamic && (tool["sourceInfo"]["source"]=="mcp" || self.registry.definitions().any(|d|d.origin==ag1_core::tools::registry::Origin::Mcp && d.metadata["name"]==tool["name"])) { return None; }
            let definition=self.registry.definitions().find(|d|d.metadata["name"]==tool["name"]);
            if definition.is_some_and(|d|d.origin==ag1_core::tools::registry::Origin::Mcp) {
                valid.then(||catalog.iter().find(|d|d["name"]==tool["name"]).cloned()).flatten()
            } else { Some(tool.clone()) }
        }).collect();
        if dynamic && valid {
            let facade=ag1_core::tools::mcp::discovery::NAME;
            if !view.iter().any(|t|t["name"]==facade) && let Some(tool)=catalog.iter().find(|t|t["name"]==facade) { view.push(tool.clone()); }
            view.extend(catalog.into_iter().filter(|t|self.registry.definitions().any(|d|d.origin==ag1_core::tools::registry::Origin::Mcp && d.metadata["name"]==t["name"])));
        }
        Ok(view)
    }
    #[cfg(target_os = "linux")]
    fn validate_mcp_request(&self,scope:&Scope,tools:&[Value])->Result<(),DomainError> {
        if self.adapters.mcp_discovery.is_some() {
            self.adapters.mcp.as_ref().ok_or(DomainError::Conflict)?.validate(scope,&self.registry)?;
            let catalog=self.registry.public_tools();
            // Native planning/controlled modes may further reduce the captured
            // selection. Revalidate every advertised MCP schema without adding
            // removed tools back after those mode-specific restrictions.
            for tool in tools {
                let mcp=tool["sourceInfo"]["source"]=="mcp" || self.registry.definitions().any(|d|d.origin==ag1_core::tools::registry::Origin::Mcp && d.metadata["name"]==tool["name"]);
                if mcp && !catalog.iter().any(|expected|expected==tool) { return Err(DomainError::Conflict); }
            }
            return Ok(());
        }
        if self.prepare_mcp_request(scope,tools)?!=tools { return Err(DomainError::Conflict); }
        Ok(())
    }

    fn folder_access(&self) -> Option<(Arc<crate::folder_access::FolderAccess>, u64)> {
        Some((self.adapters.folder_authority.clone()?, self.adapters.folders.as_ref()?.selection.revision))
    }
    fn client_activity(&self) -> Result<Value, DomainError> {
        self.adapters
            .jobs
            .as_ref()
            .map(|jobs| jobs.client_records())
            .unwrap_or_else(|| Ok(serde_json::json!({"jobs":[]})))
    }
    fn client_resources(&self) -> Option<Arc<crate::client::resource_actions::Actions>> {
        self.adapters.resources.clone()
    }
    fn client_extensions(&self) -> Option<Arc<super::extensions::NativeExtensions>> {
        self.adapters.extensions.clone()
    }
    fn client_catalog(&self) -> Vec<Value> {
        let names = self.adapters.available_names();
        self.registry
            .native_tools()
            .into_iter()
            .filter(|tool| names.iter().any(|name| tool["name"] == *name))
            .collect()
    }
    fn supports_planning(&self) -> bool {
        true
    }
    fn planning_tools(
        &self,
        allow_inspection: bool,
    ) -> Result<(Arc<dyn ToolRunner>, Value), DomainError> {
        let mut runner = self.clone();
        runner.adapters.planning = true;
        runner.policy.mode = ag1_core::tools::policy::Mode::Planning;
        runner.policy.planning_read_only_tools =
            Some(if allow_inspection { "allow" } else { "block" }.into());
        if !runner
            .policy
            .trusted_tools
            .iter()
            .any(|s| s == "submit_plan")
        {
            runner.policy.trusted_tools.push("submit_plan".into());
        }
        runner.policy_revision = runner
            .policy_revision
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let submit_tool = runner
            .registry
            .native_tools()
            .into_iter()
            .find(|t| t["name"] == "submit_plan")
            .ok_or(DomainError::NotFound)?;
        Ok((Arc::new(runner), submit_tool))
    }

    fn approval_handler(&self) -> Option<Arc<dyn ApprovalHandler>> {
        self.approvals
            .supports_commands()
            .then(|| self.approvals.clone())
    }
    fn supports_controlled_policy(&self) -> bool {
        true
    }
    fn job_notices(
        &self,
        scope: &Scope,
        operation: &str,
    ) -> Result<Vec<ag1_core::tools::jobs::Notice>, DomainError> {
        let Some(jobs) = &self.adapters.jobs else {
            return Ok(Vec::new());
        };
        let mut artifacts = ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            &self.origin,
            operation,
            &self.timestamp,
            Features::default(),
        )?;
        jobs.refresh_notices(scope, &mut artifacts)
    }
    fn acknowledge_job_notice(&self, scope: &Scope, id: &str) -> Result<(), DomainError> {
        self.adapters
            .jobs
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("background job notices".into()))?
            .acknowledge_scoped_notice(scope, id)
    }
    fn handles_session_event(&self, event: &str) -> bool {
        self.adapters
            .extensions
            .as_ref()
            .is_some_and(|extensions| extensions.handles(event))
    }
    fn discover_session_resources(
        &self,
        scope: &Scope,
        snapshot: &Value,
    ) -> Result<Option<ag1_core::runtime::prompt::PromptConfiguration>, DomainError> {
        let Some(extensions) = &self.adapters.extensions else {
            return Ok(None);
        };
        if self.tool_timeout.is_zero() || self.tool_timeout > Duration::from_secs(86400) {
            return Err(DomainError::InvalidInput(
                "extension callback deadline".into(),
            ));
        }
        let cancel = AtomicBool::new(false);
        let control = crate::resource_fs::ScanControl {
            cancel: &cancel,
            deadline: Instant::now()
                .checked_add(self.tool_timeout)
                .ok_or(DomainError::Conflict)?,
        };
        let mut snapshot = snapshot.clone();
        snapshot["allTools"] = json!(self.registry.native_tools());
        extensions.discover_resources(
            scope,
            &snapshot,
            (*self.registry).clone(),
            &self.timestamp,
            &control,
        )
    }
    fn session_event(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
    ) -> Result<Option<Value>, DomainError> {
        self.request_event(scope, snapshot, event, &AtomicBool::new(false))
    }
    fn request_event(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
        cancel: &AtomicBool,
    ) -> Result<Option<Value>, DomainError> {
        let Some(extensions) = &self.adapters.extensions else {
            return Ok(None);
        };
        if self.tool_timeout.is_zero() || self.tool_timeout > Duration::from_secs(86400) {
            return Err(DomainError::InvalidInput(
                "extension callback deadline".into(),
            ));
        }
        let deadline = Instant::now()
            .checked_add(self.tool_timeout)
            .ok_or(DomainError::Conflict)?;
        let mut snapshot = snapshot.clone();
        snapshot["allTools"] = json!(self.registry.native_tools());
        extensions.event_with_snapshot(
            scope,
            &snapshot,
            event,
            &crate::resource_fs::ScanControl { cancel, deadline },
            || Ok(()),
        )
    }
    fn quiesce(&self, scope: &Scope, operation: &str) -> Result<(), DomainError> {
        #[cfg(target_os = "linux")]
        if let Some(mcp)=&self.adapters.mcp_management { mcp.close()?; }
        if let Some(jobs) = &self.adapters.jobs {
            let mut artifacts = ContextStore::new(
                &self.store,
                SessionId::new(&scope.session)?,
                &self.origin,
                operation,
                &self.timestamp,
                Features::default(),
            )?;
            jobs.quiesce(scope, &mut artifacts)?;
        }
        if let Some(extensions) = &self.adapters.extensions {
            extensions.stop()?;
        }
        Ok(())
    }
    fn execute(
        &self,
        scope: &Scope,
        calls: &[PendingCall],
        selection: &ToolSelection,
        operation: &str,
        cancel: &AtomicBool,
        current_revision: &AtomicU64,
    ) -> Result<Vec<ToolOutcome>, DomainError> {
        if let Some((authority, revision)) = self.folder_access() { authority.check(revision)?; }
        if self.policy.mode != ag1_core::tools::policy::Mode::Planning
            && let Some(policy) = &self.adapters.approval_policy
        {
            let selected = policy.snapshot()?;
            let mut runner = self.clone();
            runner.policy.mode = selected.mode;
            runner.policy_revision = selected.revision;
            runner.adapters.approval_policy = None;
            return runner.execute(scope, calls, selection, operation, cancel, current_revision);
        }
        if calls.len() > 128
            || self.tool_timeout.is_zero()
            || self.tool_timeout > Duration::from_secs(86400)
        {
            return Err(DomainError::InvalidInput("native tool batch limits".into()));
        }
        let mut registry = (*self.registry).clone();
        let available: std::collections::BTreeSet<_> =
            self.adapters.available_names().into_iter().collect();
        let mut names = selection
            .names
            .iter()
            .filter(|name| {
                available.contains(*name)
                    && registry
                        .definitions()
                        .any(|definition| definition.metadata["name"] == **name)
            })
            .cloned()
            .collect::<Vec<_>>();
        for facade in selection.names.iter().filter(|n| ag1_core::tools::surface::is_facade(n)) {
            let dependencies = if facade == "capability" { ag1_core::tools::surface::ADVANCED } else { ag1_core::tools::surface::dependencies(facade) };
            for dependency in dependencies {
                if available.contains(*dependency) && !names.iter().any(|n|n==dependency) { names.push((*dependency).into()); }
            }
        }
        registry.activate(&names, &available)?;
        let mut authorizer = Authorizer::new(operation)?;
        let mut outputs = vec![];
        let mut index = 0;
        let mut uncertain = false;
        while index < calls.len() {
            let mut current_scope = scope.clone();
            if let Some(journal) = &selection.journal {
                current_scope.revision = journal.revision()?;
            }
            let current = Current {
                uncertain,
                registry: &registry,
                scope: &current_scope,
                cancel,
                revision: current_revision,
                selection,
            };
            let eligible = |name: &str| matches!(name, "read" | "ls" | "grep" | "find" | "diff");
            let controlled = selection
                .journal
                .as_ref()
                .is_some_and(|j| j.controlled.is_some());
            let end = if !controlled
                && self.adapters.extensions.is_none()
                && eligible(&calls[index].name)
            {
                (index..calls.len())
                    .take(8)
                    .take_while(|i| eligible(&calls[*i].name))
                    .last()
                    .unwrap()
                    + 1
            } else {
                index + 1
            };
            // Policy/preflight runs in source order. No later mutation captures
            // a file before preceding work has drained and released its permit.
            let work = (index..end)
                .map(|i| {
                    self.prepare(
                        current,
                        &calls[i],
                        selection.names.contains(&calls[i].name),
                        i > 0
                            && selection
                                .journal
                                .as_ref()
                                .is_some_and(|j| j.serialize_controlled),
                        format!("{operation}-{i}"),
                        &mut authorizer,
                    )
                })
                .collect::<Vec<_>>();
            let completed = if work.len() == 1 {
                vec![self.execute_one(current, work.into_iter().next().unwrap())]
            } else {
                std::thread::scope(|threads| {
                    let handles = work
                        .into_iter()
                        .map(|work| threads.spawn(move || self.execute_one(current, work)))
                        .collect::<Vec<_>>();
                    // Always join EVERY accepted worker, including after an
                    // earlier error. Vec ordering follows calls, not completion.
                    handles
                        .into_iter()
                        .map(|handle| {
                            handle.join().unwrap_or_else(|_| {
                                Err(DomainError::Io("native tool worker panicked".into()))
                            })
                        })
                        .collect::<Vec<_>>()
                })
            };
            for result in completed {
                let result = result?;
                if let Some(journal) = &selection.journal {
                    journal.publish_result(&result.outcome)?;
                    if result.execution_settled && !result.outcome.replayed
                        && matches!(result.outcome.record.tool.as_str(), "edit" | "editApply" | "editPreview" | "write" | "append" | "todo" | "bash" | "check" | "task" | "explore") {
                        let record = &result.outcome.record;
                        let artifacts = ContextStore::new(&self.store, SessionId::new(&scope.session)?,
                            &self.origin, &record.operation, &self.timestamp, Features::default())?;
                        // Rejections have no native-result artifact. Their
                        // acknowledged rejection is already the native result.
                        let native = record.native_result.as_ref()
                            .map(|artifact| artifacts.read_json(artifact)).transpose()?
                            .unwrap_or_else(|| result.outcome.result.clone());
                        // The durable arguments are the actual admitted inputs,
                        // including native shell transforms. Extension display
                        // hooks cannot redefine recovery identity or outcomes.
                        let arguments = artifacts.read_json(&record.arguments)?;
                        let mut snapshot_scope = current_scope.clone();
                        snapshot_scope.revision = journal.revision()?;
                        let todo = if record.tool == "todo" {
                            ag1_core::tools::todo::latest(&journal.snapshot(&snapshot_scope)?)
                        } else { None };
                        let events = journal.repetition.recovery_outcome(&record.tool, &record.call_id,
                            &arguments, &native, todo.as_ref())?;
                        journal.repetition_events(events);
                    }
                    if let Some(completion) = &journal.completion {
                        let artifacts = ContextStore::new(
                            &self.store,
                            SessionId::new(&scope.session)?,
                            &self.origin,
                            &result.outcome.record.operation,
                            &self.timestamp,
                            Features::default(),
                        )?;
                        super::completion_evidence::observe(
                            completion,
                            &artifacts,
                            &result.outcome,
                        )?;
                    }
                    if let Some(controlled) = &journal.controlled {
                        let artifacts = ContextStore::new(
                            &self.store,
                            SessionId::new(&scope.session)?,
                            &self.origin,
                            &result.outcome.record.operation,
                            &self.timestamp,
                            Features::default(),
                        )?;
                        super::controlled::observe(
                            controlled,
                            &self.files,
                            &artifacts,
                            &result.outcome,
                        )?;
                    }
                }
                uncertain |= !result.execution_settled;
                outputs.push(result);
            }
            index = end;
        }
        Ok(outputs)
    }
}

struct RuntimeHooks<'a> {
    registry: &'a Registry,
    extensions: Option<&'a super::extensions::NativeExtensions>,
    scope: &'a Scope,
    cancel: &'a AtomicBool,
    deadline: Instant,
    journal: Option<&'a super::tool_journal::ToolJournal>,
    clock: &'a dyn Clock,
}
impl Hooks for RuntimeHooks<'_> {
    fn after(
        &mut self,
        call: &ag1_core::tools::registry::ValidatedCall,
        result: &serde_json::Value,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        let mut extension_error = None;
        let mut projected = if let (Some(extensions), Some(journal)) =
            (self.extensions, self.journal)
        {
            let mut event = json!({"type":"tool_result","toolName":call.name(),"toolCallId":call.call_id(),"input":call.args(),"content":result["content"],"isError":result["isError"]==true});
            if let Some(details) = result.get("details") {
                event["details"] = details.clone();
            }
            let mut scope = self.scope.clone();
            scope.revision = journal.revision()?;
            extensions
                .tool_event(
                    journal,
                    self.registry,
                    &scope,
                    event,
                    &crate::resource_fs::ScanControl {
                        cancel: self.cancel,
                        deadline: self.deadline,
                    },
                )
                .unwrap_or_else(|error| {
                    extension_error = Some(error);
                    None
                })
                .map(|patch| {
                    let mut projection = result.clone();
                    for key in ["content", "details", "isError"] {
                        if let Some(value) = patch.get(key) {
                            projection[key] = value.clone();
                        }
                    }
                    projection
                })
        } else {
            None
        };
        if let Some(journal) = self.journal {
            journal.repetition.after(
                call.name(),
                call.call_id(),
                call.args(),
                result,
                self.clock.now_ms(),
            )?;
            let content = &result["content"];
            let text = match content.as_str() {
                Some(text) => std::borrow::Cow::Borrowed(text),
                None => std::borrow::Cow::Owned(
                    content
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|block| block["type"] == "text")
                        .filter_map(|block| block["text"].as_str())
                        .collect::<String>(),
                ),
            };
            let guidance = journal
                .failures
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .observe(call.name(), result["isError"] == true, &text)?;
            if let Some(guidance) = guidance {
                // P09 retained the native result before this hook. Only the
                // model-facing result gains guidance; evidence stays original.
                let projection = projected.get_or_insert_with(|| result.clone());
                let mut blocks = projection["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                blocks.push(json!({"type":"text","text":guidance}));
                projection["content"] = json!(blocks);
            }
        }
        if let Some(error) = extension_error {
            return Err(error);
        }
        Ok(projected)
    }
}
