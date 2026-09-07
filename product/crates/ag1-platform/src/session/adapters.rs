//! Concrete P09 adapters available to the native tool loop. These are host
//! capabilities; public model tool names alone cannot construct any service.
use crate::target_tools::Port;
use crate::{
    artifact_tools::{ArtifactTools, PreparedArtifact},
    episode_tools::{EpisodeTools, PreparedEpisode},
    file_tools::{FileExecution, FileOutcome, FileService, PreparedFile},
    git_tools::GitTools,
    job_tools::JobService,
    memory_tools::{MemoryTools, PreparedMemory},
    observation_tools::{ObservationTools, PreparedObservation},
    repo_tools::RepoTools,
    search_tools::SearchService,
    shell_tools::{PreparedShell, ShellExecution, ShellService},
    source_tools::{PreparedSource, SourceTools},
    task_tools::{PreparedTask, TaskTools},
    todo_tools::{PreparedTodo, TodoTools},
    web_http::SystemResolver,
    web_tools::{SearchService as WebSearchService, WebService},
};
use ag1_core::{
    Clock, DomainError,
    tools::{
        approval::AuthorizedCall,
        executor::{AdapterOutcome, Effect, ExecutionArtifacts},
        registry::{Registry, Scope, ValidatedCall},
    },
};
use ag1_storage::Store;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Instant,
};

#[derive(Clone, Default)]
pub struct NativeAdapters {
    pub resources: Option<Arc<crate::client::resource_actions::Actions>>,
    pub project: Option<Arc<std::sync::Mutex<crate::projects::ProjectPort>>>,
    /// Host-selected planning capability. Ordinary sessions do not expose it.
    pub planning: bool,
    pub delegation: Option<Arc<super::subagent_tool::DelegationService>>,
    pub extensions: Option<Arc<super::extensions::NativeExtensions>>,
    pub search: Option<Arc<SearchService>>,
    pub git: Option<Arc<GitTools>>,
    pub shell: Option<Arc<ShellService>>,
    pub jobs: Option<Arc<JobService<Store>>>,
    pub web: Option<Arc<NativeWeb>>,
    pub memory: Option<Arc<MemoryTools>>,
    pub artifact: Option<Arc<ArtifactTools>>,
    pub repo: Option<Arc<RepoTools>>,
    pub task: Option<Arc<TaskTools>>,
    pub todo: Option<Arc<TodoTools>>,
    pub observation: Option<Arc<ObservationTools>>,
    pub source: Option<Arc<SourceTools>>,
    pub episode: Option<Arc<EpisodeTools>>,
}
/// Host-owned async executor and scope-bound web capabilities. The host keeps
/// this runtime alive until all session tool work has drained.
pub struct NativeWeb {
    pub fetch: Arc<WebService<SystemResolver>>,
    pub search: Option<Arc<WebSearchService<SystemResolver>>>,
    pub runtime: tokio::runtime::Handle,
}
impl NativeAdapters {
    pub fn available_names(&self) -> Vec<String> {
        let mut names = vec![
            "read",
            "write",
            "edit",
            "append",
            "editPreview",
            "editApply",
            "ls",
        ];
        if self.search.is_some() {
            names.extend(["grep", "find"]);
        }
        if self.planning {
            names.push("submit_plan");
        }
        if self.delegation.is_some() {
            names.push("task");
            names.push("explore");
        }
        if self.git.is_some() {
            names.push("diff");
        }
        if self.shell.is_some() {
            names.push("bash");
        }
        if self.jobs.is_some() && self.shell.is_some() {
            names.push("job");
        }
        if self.memory.is_some() {
            names.push("memory");
        }
        if self.artifact.is_some() {
            names.push("artifact");
        }
        if self.repo.is_some() {
            names.push("repo");
        }
        if self.episode.is_some() {
            names.push("episode");
        }
        if self.source.is_some() {
            names.push("sourceContext");
        }
        if self.observation.is_some() {
            names.push("runtimeObservation");
        }
        if self.todo.is_some() {
            names.push("todo");
        }
        if self.task.is_some() {
            names.push("taskState");
        }
        if let Some(web) = &self.web {
            names.push("webFetch");
            if web.search.is_some() {
                names.push("webSearch");
            }
        }
        let mut names: Vec<String> = names.into_iter().map(str::to_owned).collect();
        if let Some(project) = &self.project {
            match &*project.lock().unwrap() {
                crate::projects::ProjectPort::Shared(p) => names.extend(p.names()),
                crate::projects::ProjectPort::SharedCheck(p) if p.available() => {
                    names.push("check".into())
                }
                crate::projects::ProjectPort::Check(p)
                    if p.runtime.command(p.kind, "check").is_ok() =>
                {
                    names.push("check".into())
                }
                crate::projects::ProjectPort::Visual(p)
                    if p.vision.caller
                        && p.browser.available().available
                        && p.runtime.verify_preview(&p.grant).is_ok() =>
                {
                    names.extend(
                        ["screenshot_url", "interact_then_screenshot", "verify_ui"]
                            .map(str::to_string),
                    )
                }
                _ => {}
            }
        }
        if let Some(extensions) = &self.extensions {
            names.extend(extensions.tool_names());
        }
        names
    }
    pub(super) fn prepare(
        &self,
        files: &Arc<FileService>,
        call: &ValidatedCall,
        context: AdapterContext<'_>,
    ) -> Result<PreparedAdapter, DomainError> {
        let missing = || DomainError::Unavailable("native tool adapter is not configured".into());
        if call.origin() == ag1_core::tools::registry::Origin::Extension {
            let service = self.extensions.clone().ok_or_else(missing)?;
            if !service.tool_names().iter().any(|name| name == call.name()) {
                return Err(missing());
            }
            return Ok(PreparedAdapter::Extension(service));
        }
        Ok(match call.name() {
            "check" | "screenshot_url" | "interact_then_screenshot" | "verify_ui" => {
                let service = self.project.clone().ok_or_else(missing)?;
                let request = ag1_core::tools::targets::Request::from_call(call)?;
                let mut port = service.lock().map_err(|_| DomainError::Conflict)?;
                let control = crate::resource_fs::ScanControl {
                    cancel: context.cancel,
                    deadline: context.deadline,
                };
                let plan = port.prepare(&request, &control)?;
                let binding = port.binding(&plan);
                binding.admits(&request)?;
                if binding.workspace != context.scope.workspace {
                    return Err(DomainError::PermissionDenied);
                }
                port.revalidate(&plan, &control)?;
                use sha2::{Digest, Sha256};
                let revision = format!(
                    "project-{:x}",
                    Sha256::digest(serde_json::json!([binding, port.display(&plan)]).to_string())
                );
                drop(port);
                PreparedAdapter::Project {
                    service,
                    plan,
                    request,
                    revision,
                }
            }
            "submit_plan" if self.planning => PreparedAdapter::SubmitPlan,
            "task" => {
                let service = self.delegation.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope, context.journal)?);
                PreparedAdapter::Delegation { service, plan }
            }
            "explore" => {
                let service = self.delegation.clone().ok_or_else(missing)?;
                let plan =
                    Box::new(service.prepare_explore(call, context.scope, context.journal)?);
                PreparedAdapter::Explore { service, plan }
            }
            "read" | "write" | "edit" | "append" | "editPreview" | "editApply" => {
                PreparedAdapter::File {
                    service: files.clone(),
                    plan: Box::new(files.prepare_queued(
                        call,
                        context.scope,
                        context.cancel,
                        context.deadline,
                    )?),
                }
            }
            "ls" => PreparedAdapter::List(files.clone()),
            "grep" | "find" => PreparedAdapter::Search(self.search.clone().ok_or_else(missing)?),
            "diff" => PreparedAdapter::Diff(self.git.clone().ok_or_else(missing)?),
            "repo" => PreparedAdapter::Repo(self.repo.clone().ok_or_else(missing)?),
            "bash" => {
                let service = self.shell.clone().ok_or_else(missing)?;
                let plan = service.prepare(call, context.registry, context.scope)?;
                PreparedAdapter::Shell { service, plan }
            }
            "job" => {
                let service = self.jobs.clone().ok_or_else(missing)?;
                let shell = self.shell.as_ref().ok_or_else(missing)?;
                let plan = if call.args()["action"] == "start" {
                    Some(shell.prepare_job(call, context.registry, context.scope)?)
                } else {
                    None
                };
                if plan
                    .as_ref()
                    .is_some_and(|plan| plan.target_revision() != service.target_revision())
                {
                    return Err(DomainError::Conflict);
                }
                PreparedAdapter::Job {
                    service,
                    plan,
                    may_mutate: matches!(call.args()["action"].as_str(), Some("start" | "kill")),
                }
            }
            "memory" => {
                let service = self.memory.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope)?);
                PreparedAdapter::Memory { service, plan }
            }
            "episode" => {
                let service = self.episode.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope, context.journal)?);
                PreparedAdapter::Episode { service, plan }
            }
            "sourceContext" => {
                let service = self.source.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope, context.journal)?);
                PreparedAdapter::Source { service, plan }
            }
            "runtimeObservation" => {
                let service = self.observation.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope, context.journal)?);
                PreparedAdapter::Observation { service, plan }
            }
            "todo" => {
                let service = self.todo.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope, context.journal)?);
                PreparedAdapter::Todo { service, plan }
            }
            "taskState" => {
                let service = self.task.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope, context.journal)?);
                PreparedAdapter::Task { service, plan }
            }
            "artifact" => {
                let service = self.artifact.clone().ok_or_else(missing)?;
                let plan = Box::new(service.prepare(call, context.scope)?);
                PreparedAdapter::Artifact { service, plan }
            }
            "webFetch" | "webSearch" => {
                let service = self.web.clone().ok_or_else(missing)?;
                let search = call.name() == "webSearch";
                if search && service.search.is_none() {
                    return Err(missing());
                }
                PreparedAdapter::Web { service, search }
            }
            _ => return Err(missing()),
        })
    }
}
#[derive(Clone, Copy)]
pub(super) struct AdapterContext<'a> {
    pub journal: Option<&'a Arc<super::tool_journal::ToolJournal>>,
    pub registry: &'a Registry,
    pub scope: &'a Scope,
    pub policy_revision: u64,
    pub clock: &'a dyn Clock,
    pub profile: &'a ag1_core::tools::research::Profile,
    pub loaded_memory: &'a std::collections::BTreeSet<String>,
    pub cancel: &'a AtomicBool,
    pub deadline: Instant,
}
impl<'a> AdapterContext<'a> {
    fn file(self) -> FileExecution<'a, impl Fn() -> u64 + 'a> {
        FileExecution {
            registry: self.registry,
            scope: self.scope,
            policy_revision: self.policy_revision,
            now: move || self.clock.now_ms(),
            cancel: self.cancel,
            deadline: self.deadline,
        }
    }
    fn shell(self) -> ShellExecution<'a, impl Fn() -> u64 + 'a> {
        ShellExecution {
            registry: self.registry,
            scope: self.scope,
            policy_revision: self.policy_revision,
            now: move || self.clock.now_ms(),
            cancel: self.cancel,
            deadline: self.deadline,
        }
    }
}
pub(super) enum PreparedAdapter {
    Explore {
        service: Arc<super::subagent_tool::DelegationService>,
        plan: Box<super::subagent_tool::PreparedExplore>,
    },
    Extension(Arc<super::extensions::NativeExtensions>),
    Project {
        service: Arc<std::sync::Mutex<crate::projects::ProjectPort>>,
        plan: crate::projects::ProjectPlan,
        request: ag1_core::tools::targets::Request,
        revision: String,
    },
    SubmitPlan,
    Delegation {
        service: Arc<super::subagent_tool::DelegationService>,
        plan: Box<super::subagent_tool::PreparedDelegation>,
    },
    Episode {
        service: Arc<EpisodeTools>,
        plan: Box<PreparedEpisode>,
    },
    Observation {
        service: Arc<ObservationTools>,
        plan: Box<PreparedObservation>,
    },
    Todo {
        service: Arc<TodoTools>,
        plan: Box<PreparedTodo>,
    },
    Source {
        service: Arc<SourceTools>,
        plan: Box<PreparedSource>,
    },
    Task {
        service: Arc<TaskTools>,
        plan: Box<PreparedTask>,
    },
    Artifact {
        service: Arc<ArtifactTools>,
        plan: Box<PreparedArtifact>,
    },
    Memory {
        service: Arc<MemoryTools>,
        plan: Box<PreparedMemory>,
    },
    Web {
        service: Arc<NativeWeb>,
        search: bool,
    },
    File {
        service: Arc<FileService>,
        plan: Box<PreparedFile>,
    },
    List(Arc<FileService>),
    Search(Arc<SearchService>),
    Diff(Arc<GitTools>),
    Repo(Arc<RepoTools>),
    Shell {
        service: Arc<ShellService>,
        plan: PreparedShell,
    },
    Job {
        service: Arc<JobService<Store>>,
        plan: Option<PreparedShell>,
        may_mutate: bool,
    },
}
impl PreparedAdapter {
    pub fn target_revision(&self) -> String {
        match self {
            Self::Project { revision, .. } => revision.clone(),
            Self::Extension(service) => service.target_revision(),
            Self::SubmitPlan => "submit-plan-v1".into(),
            Self::Delegation { plan, .. } => plan.target_revision().into(),
            Self::Explore { plan, .. } => plan.target_revision().into(),
            Self::Web { service, search } => {
                if *search {
                    service
                        .search
                        .as_ref()
                        .expect("prepared search capability")
                        .target_revision()
                        .into()
                } else {
                    service.fetch.target_revision().into()
                }
            }
            Self::Memory { plan, .. } => plan.target_revision().into(),
            Self::Task { plan, .. } => plan.target_revision().into(),
            Self::Todo { plan, .. } => plan.target_revision().into(),
            Self::Observation { plan, .. } => plan.target_revision().into(),
            Self::Source { plan, .. } => plan.target_revision().into(),
            Self::Episode { plan, .. } => plan.target_revision().into(),
            Self::Artifact { plan, .. } => plan.target_revision().into(),
            Self::File { plan, .. } => plan.target_revision().into(),
            Self::List(service) => service.list_target_revision(),
            Self::Search(service) => service.target_revision(),
            Self::Diff(service) => service.target_revision(),
            Self::Repo(service) => service.target_revision(),
            Self::Shell { plan, .. } => plan.target_revision().into(),
            Self::Job { service, .. } => service.target_revision().into(),
        }
    }
    pub fn transformed_call(&self) -> Option<&ValidatedCall> {
        match self {
            Self::Shell { plan, .. }
            | Self::Job {
                plan: Some(plan), ..
            } => Some(plan.call()),
            _ => None,
        }
    }
    pub fn may_mutate(&self) -> bool {
        match self {
            Self::Project { .. } => true,
            Self::Extension(_) => true,
            Self::Delegation { .. } => true,
            Self::Explore { .. } => true,
            Self::Memory { plan, .. } => plan.may_mutate(),
            Self::Task { plan, .. } => plan.may_mutate(),
            Self::Todo { plan, .. } => plan.may_mutate(),
            Self::Observation { plan, .. } => plan.may_mutate(),
            Self::Source { plan, .. } => plan.may_mutate(),
            Self::Episode { plan, .. } => plan.may_mutate(),
            Self::Artifact { plan, .. } => plan.may_mutate(),
            Self::File { plan, .. } => plan.will_mutate(),
            Self::Shell { .. } => true,
            Self::Job { may_mutate, .. } => *may_mutate,
            _ => false,
        }
    }
    pub fn execute(
        self,
        authorization: AuthorizedCall,
        context: AdapterContext<'_>,
        artifacts: &mut impl ExecutionArtifacts,
    ) -> Result<AdapterOutcome, DomainError> {
        let file = |outcome: FileOutcome| AdapterOutcome {
            effect: if outcome.published.is_some() {
                Effect::Committed
            } else {
                Effect::None
            },
            result: outcome.result,
            artifact_page: false,
        };
        match self {
            Self::Project {
                service,
                plan,
                request,
                revision,
            } => {
                authorization.consume(
                    context.registry,
                    ag1_core::tools::approval::ExecutionFence {
                        scope: context.scope,
                        policy_revision: context.policy_revision,
                        target_revision: &revision,
                        now: context.clock.now_ms(),
                    },
                )?;
                let control = crate::resource_fs::ScanControl {
                    cancel: context.cancel,
                    deadline: context.deadline,
                };
                let mut port = service.lock().map_err(|_| DomainError::Conflict)?;
                port.revalidate(&plan, &control)?;
                port.binding(&plan).admits(&request)?;
                port.execute(plan, &request, &control, artifacts)
            }
            Self::Extension(service) => {
                let call = authorization.consume(
                    context.registry,
                    ag1_core::tools::approval::ExecutionFence {
                        scope: context.scope,
                        policy_revision: context.policy_revision,
                        target_revision: &service.target_revision(),
                        now: context.clock.now_ms(),
                    },
                )?;
                let journal = context.journal.ok_or_else(|| {
                    DomainError::Unavailable("extension tool requires native journal".into())
                })?;
                let result = service
                    .invoke_tool(
                        journal,
                        context.registry,
                        context.scope,
                        &call,
                        "tool.execute",
                        &crate::resource_fs::ScanControl {
                            cancel: context.cancel,
                            deadline: context.deadline,
                        },
                    )?
                    .ok_or_else(|| DomainError::InvalidInput("extension tool result".into()))?;
                Ok(AdapterOutcome {
                    result,
                    effect: Effect::Unknown,
                    artifact_page: false,
                })
            }
            Self::Web { service, search } => {
                service.execute(search, authorization, context, artifacts)
            }
            Self::SubmitPlan => {
                if context.cancel.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(DomainError::Cancelled);
                }
                if Instant::now() >= context.deadline {
                    return Err(DomainError::TimedOut);
                }
                let call = authorization.consume(
                    context.registry,
                    ag1_core::tools::approval::ExecutionFence {
                        scope: context.scope,
                        policy_revision: context.policy_revision,
                        target_revision: "submit-plan-v1",
                        now: context.clock.now_ms(),
                    },
                )?;
                Ok(AdapterOutcome {
                    effect: Effect::None,
                    result: ag1_core::tools::submit_plan::execute(call.args())?,
                    artifact_page: false,
                })
            }
            Self::Delegation { service, plan } => {
                service.execute(*plan, authorization, context.file(), artifacts)
            }
            Self::Explore { service, plan } => {
                service.execute_explore(*plan, authorization, context.file(), artifacts)
            }
            Self::Memory { service, plan } => {
                service.execute(*plan, authorization, context.file(), context.loaded_memory)
            }
            Self::Task { service, plan } => service.execute(*plan, authorization, context.file()),
            Self::Todo { service, plan } => service.execute(*plan, authorization, context.file()),
            Self::Observation { service, plan } => {
                service.execute(*plan, authorization, context.file())
            }
            Self::Source { service, plan } => service.execute(*plan, authorization, context.file()),
            Self::Episode { service, plan } => {
                service.execute(*plan, authorization, context.file())
            }
            Self::Artifact { service, plan } => {
                service.execute(*plan, authorization, context.file())
            }
            Self::File { service, plan } => service
                .execute(*plan, authorization, context.file())
                .map(file),
            Self::List(service) => service
                .execute_list(authorization, context.file())
                .map(file),
            Self::Search(service) => service.execute(authorization, context.file()).map(file),
            Self::Diff(service) => service.execute(authorization, context.file()).map(file),
            Self::Repo(service) => service.execute(authorization, context.file()).map(file),
            Self::Shell { service, plan } => {
                service.execute(plan, authorization, context.shell(), artifacts, |_| {})
            }
            Self::Job { service, plan, .. } => {
                service.execute(plan, authorization, context.shell(), artifacts)
            }
        }
    }
}

impl NativeWeb {
    fn execute(
        &self,
        search: bool,
        authorization: AuthorizedCall,
        context: AdapterContext<'_>,
        artifacts: &mut impl ExecutionArtifacts,
    ) -> Result<AdapterOutcome, DomainError> {
        use crate::{provider::http::cancellation, web_tools::WebExecution};
        use std::{sync::atomic::Ordering, time::Duration};
        // NativeToolRunner runs on the bounded blocking pool, never on a Tokio
        // reactor thread. No per-call runtime or detached cancellation task.
        self.runtime.block_on(async {
            let (source, mut token) = cancellation();
            if context.cancel.load(Ordering::Acquire) {
                source.cancel();
            }
            let execution = WebExecution {
                registry: context.registry,
                scope: context.scope,
                policy_revision: context.policy_revision,
                now: || context.clock.now_ms(),
                deadline: tokio::time::Instant::from_std(context.deadline),
                cancel: &mut token,
                profile: context.profile.clone(),
            };
            let operation = async {
                if search {
                    self.search
                        .as_ref()
                        .ok_or(ag1_core::DomainError::PermissionDenied)?
                        .execute(
                            authorization,
                            execution,
                            utc_year(context.clock.now_ms()),
                            artifacts,
                        )
                        .await
                } else {
                    self.fetch
                        .execute(authorization, execution, artifacts)
                        .await
                }
            };
            tokio::pin!(operation);
            loop {
                tokio::select! {
                    result = &mut operation => return result,
                    _ = tokio::time::sleep(Duration::from_millis(10)) => {
                        if context.cancel.load(Ordering::Acquire) {
                            source.cancel();
                            // Drain the service's cancellation path before the
                            // durable tool result can be published.
                            return operation.await;
                        }
                    }
                }
            }
        })
    }
}
// Gregorian civil year from nonnegative Unix milliseconds, using 400-year eras.
fn utc_year(milliseconds: u64) -> u32 {
    let days = milliseconds / 86_400_000 + 719468;
    let era = days / 146097;
    let day = days % 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let day_of_year = day - (365 * year + year / 4 - year / 100);
    let month = (5 * day_of_year + 2) / 153;
    (year + era * 400 + u64::from(month >= 10)) as u32
}
