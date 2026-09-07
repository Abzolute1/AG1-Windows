//! Trusted resource helpers invoked only by explicit native client commands.
use super::*;
use crate::{
    file_tools::FileExecution,
    package_execution::{PackageService, PreparedPackage},
    resource_fs::ScanControl,
    session::files::ApprovalHandler,
    skill_creator::SkillCreator,
};
use ag1_core::{
    context::features::Features,
    tools::{
        approval::{Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{self, Execution, NoHooks},
        registry::{Registry, Scope},
        result::ResultBudget,
    },
};
use std::{collections::BTreeMap, time::Instant};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Packages {
    pub process_host: PathBuf,
    pub node: PathBuf,
    pub npm_cli: PathBuf,
    pub git: PathBuf,
    #[serde(default)]
    pub offline: bool,
}
pub struct Actions {
    registry: Registry,
    skill: SkillCreator,
    settings: crate::package_settings::PackageSettings,
    packages: Option<PackageService>,
    store: Arc<Store>,
    approvals: Arc<PendingApprovals>,
    clock: Arc<dyn Clock>,
    policy: ag1_core::tools::policy::Policy,
}
impl Actions {
    pub(super) fn new(
        workspace: Arc<Workspace>,
        store: Arc<Store>,
        state: &Path,
        options: Option<&Packages>,
        approvals: Arc<PendingApprovals>,
        clock: Arc<dyn Clock>,
        mode: &str,
    ) -> Result<Self> {
        let skill = SkillCreator::new(workspace.clone(), ".ag1/skills")?;
        let mut registry = Registry::builtins()?;
        SkillCreator::register(&mut registry)?;
        let packages = options
            .map(|o| package_service(workspace.clone(), state, o))
            .transpose()?;
        let settings = crate::package_settings::PackageSettings::new(
            workspace.clone(),
            ".ag1/settings.json",
            workspace.root(),
            &state.join("home"),
        )?;
        let mut names = vec!["createSkill".into(), "write".into()];
        if packages.is_some() {
            PackageService::register(&mut registry)?;
            names.push("managePackage".into());
        }
        registry.activate(&names, &names.iter().cloned().collect())?;
        let policy = serde_json::from_value(json!({"mode":mode,"cwd":workspace.root()}))
            .map_err(json_error)?;
        Ok(Self {
            registry,
            skill,
            settings,
            packages,
            store,
            approvals,
            clock,
            policy,
        })
    }
    pub fn catalog(&self) -> Value {
        json!({"createSkill":true,"configurePackage":true,"managePackage":self.packages.is_some(),"scope":"Isolated project and private resource caches","approval":"Native policy and one-use approval binding"})
    }
    pub fn execute(
        &self,
        scope: &Scope,
        operation: &str,
        name: &str,
        args: &Value,
        cancel: &AtomicBool,
    ) -> Result<Value> {
        if !matches!(name, "createSkill" | "managePackage" | "configurePackage") {
            return Err(DomainError::PermissionDenied);
        }
        enum Plan {
            Skill(crate::skill_creator::PreparedSkill),
            Package(PreparedPackage),
            Settings(Box<crate::package_settings::PreparedSettings>),
        }
        let (call, plan) = if name == "configurePackage" {
            let source = args["source"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("package source".into()))?;
            let change = match args["enabled"].as_bool() {
                Some(true) => crate::package_settings::Change::Add,
                Some(false) => crate::package_settings::Change::Remove,
                None => return Err(DomainError::InvalidInput("package enabled boolean".into())),
            };
            let Some(plan) =
                self.settings
                    .prepare(&self.registry, scope, operation, change, source)?
            else {
                return Ok(json!({"record":{"status":"completed"},"result":{"unchanged":true}}));
            };
            (plan.call().clone(), Plan::Settings(Box::new(plan)))
        } else {
            let call = self
                .registry
                .prepare(operation, operation, name, args, true)?;
            let plan = if name == "createSkill" {
                Plan::Skill(self.skill.prepare(&call, scope)?)
            } else {
                Plan::Package(
                    self.packages
                        .as_ref()
                        .ok_or_else(|| {
                            DomainError::Unavailable("Package helpers are not configured".into())
                        })?
                        .prepare(&call, scope)?,
                )
            };
            (call, plan)
        };
        let target = match &plan {
            Plan::Skill(p) => p.target_revision(),
            Plan::Package(p) => p.target_revision(),
            Plan::Settings(p) => p.target_revision(),
        }
        .to_owned();
        let mut authorizer = Authorizer::new(operation)?;
        let fence = || ExecutionFence {
            scope,
            policy_revision: 1,
            target_revision: &target,
            now: self.clock.now_ms(),
        };
        let ticket = match authorizer.preflight(
            call,
            PreflightContext {
                scope,
                policy: &self.policy,
                policy_revision: 1,
                target_revision: &target,
                now: self.clock.now_ms(),
                ttl: 300000,
                untrusted_extensions: false,
            },
        )? {
            Preflight::Ready(ticket) => ticket,
            Preflight::Rejected(reason) => return Err(DomainError::InvalidInput(reason)),
            Preflight::Approval(request) => {
                let allow = self.approvals.approve(&request, cancel)?;
                authorizer
                    .answer(&request.binding, allow, &self.registry, fence())?
                    .ok_or(DomainError::PermissionDenied)?
            }
        };
        let control = ScanControl {
            cancel,
            deadline: Instant::now() + Duration::from_secs(300),
        };
        control.check()?;
        let mut artifacts = ag1_storage::context::ContextStore::new(
            &self.store,
            SessionId::new(&scope.session)?,
            &scope.session,
            operation,
            &timestamp(),
            Features::default(),
        )?;
        let execution = Execution {
            registry: &self.registry,
            fence: fence(),
            budget: ResultBudget::default(),
            clock: self.clock.as_ref(),
        };
        let result = match plan {
            Plan::Skill(plan) => executor::execute_with_artifacts(
                self.store.as_ref(),
                &mut artifacts,
                ticket,
                execution,
                &mut NoHooks,
                |authorization, _| {
                    self.skill
                        .execute(
                            plan,
                            authorization,
                            FileExecution {
                                registry: &self.registry,
                                scope,
                                policy_revision: 1,
                                now: || self.clock.now_ms(),
                                cancel,
                                deadline: control.deadline,
                            },
                        )
                        .map(|r| r.outcome)
                },
                |_| {},
            )?,
            Plan::Settings(plan) => executor::execute_with_artifacts(
                self.store.as_ref(),
                &mut artifacts,
                ticket,
                execution,
                &mut NoHooks,
                |authorization, _| {
                    self.settings
                        .execute(
                            *plan,
                            authorization,
                            FileExecution {
                                registry: &self.registry,
                                scope,
                                policy_revision: 1,
                                now: || self.clock.now_ms(),
                                cancel,
                                deadline: control.deadline,
                            },
                        )
                        .map(|r| executor::AdapterOutcome {
                            result: r.result,
                            effect: if r.published.is_some() {
                                executor::Effect::Committed
                            } else {
                                executor::Effect::None
                            },
                            artifact_page: false,
                        })
                },
                |_| {},
            )?,
            Plan::Package(plan) => self
                .packages
                .as_ref()
                .ok_or(DomainError::Conflict)?
                .execute(
                    self.store.as_ref(),
                    &mut artifacts,
                    plan,
                    ticket,
                    crate::package_execution::Execution {
                        lifecycle: execution,
                        control,
                    },
                    &mut NoHooks,
                )?,
        };
        // P09 retains both the exact output artifact and durable execution status.
        Ok(json!({"record":result.record,"result":result.result}))
    }
}
pub(super) fn package_service(
    workspace: Arc<Workspace>,
    state: &Path,
    options: &Packages,
) -> Result<PackageService> {
    use ag1_core::resources::{package_lifecycle, packages};
    let home = state.join("home");
    let agent = home.join(".ag1/agent");
    let temporary = state.join("package-temporary");
    let spools = state.join("package-spools");
    let paths = [
        workspace.root().join(".ag1/npm"),
        workspace.root().join(".ag1/git"),
        agent.join("npm"),
        agent.join("git"),
        temporary.clone(),
    ];
    let manager = WorkspaceService::new()?;
    let mut roots = vec![];
    for path in &paths {
        fs::create_dir_all(path).map_err(crate::io_error)?;
        roots.push(manager.register(path)?);
    }
    fs::create_dir_all(&spools).map_err(crate::io_error)?;
    let binaries = BTreeMap::from([
        (
            "npm".into(),
            crate::package_execution::Binary {
                executable: options.node.clone(),
                arguments: vec![
                    options
                        .npm_cli
                        .to_str()
                        .ok_or(DomainError::PermissionDenied)?
                        .into(),
                ],
            },
        ),
        (
            "git".into(),
            crate::package_execution::Binary {
                executable: options.git.clone(),
                arguments: vec![],
            },
        ),
    ]);
    PackageService::new(
        workspace.clone(),
        Arc::new(crate::process::ProcessSupervisor::new(
            &options.process_host,
        )?),
        crate::package_execution::Config {
            manager: package_lifecycle::Config {
                cwd: workspace.root().display().to_string(),
                agent_dir: agent.display().to_string(),
                home: home.display().to_string(),
                temporary_dir: temporary.display().to_string(),
                npm: packages::Command {
                    command: "npm".into(),
                    args: vec![],
                },
                offline: options.offline,
            },
            npm_prefix: roots[2].clone(),
            readable_roots: vec![workspace],
            writable_roots: roots,
            binaries,
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            spools,
        },
    )
}
