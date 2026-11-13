//! Model lifecycle facade over the same owned Runtime used by the host UI.
use super::*;
use ag1_core::tools::{
    executor::{AdapterOutcome, Effect},
    targets::{Binding, Capability, VisionAccess},
};

pub struct PreviewPlan {
    pub binding: Binding,
    pub display: Value,
    action: String,
    operation: Option<String>,
    launch: Option<Launch>,
    toolchain_hashes: Vec<(PathBuf, String)>,
    configuration: Value,
}
impl Runtime {
    pub fn prepare_model_preview(
        &mut self,
        args: &Value,
        control: &ScanControl<'_>,
    ) -> Result<PreviewPlan, DomainError> {
        control.check()?;
        let action = args["action"]
            .as_str()
            .ok_or_else(|| invalid("preview action"))?;
        if !["status", "start", "stop"].contains(&action) {
            return Err(invalid("preview action"));
        }
        // Discovery changes only the in-memory target catalog, never starts work.
        self.discover()?;
        let operation = self.running.as_ref().map(|r| r.record.id.clone());
        let mut launch = None;
        let target = if action == "start" {
            if operation.is_some() {
                return Err(DomainError::Unavailable("An owned target is already running; inspect preview status before starting another.".into()));
            }
            let target = self
                .project
                .targets
                .iter()
                .find(|t| {
                    t.kind == Kind::Web && args["target"].as_str().is_none_or(|id| id == t.id)
                })
                .ok_or_else(|| {
                    DomainError::Unavailable(
                        "Preview requires a root package.json with an installed Vite dev script."
                            .into(),
                    )
                })?;
            let package = self.workspace.snapshot("package.json", 8192)?;
            let document: Value =
                serde_json::from_slice(&package.bytes).map_err(|_| invalid("package.json"))?;
            if document["scripts"]["dev"]
                .as_str()
                .and_then(|s| s.split_whitespace().next())
                != Some("vite")
            {
                return Err(DomainError::Unavailable("This preview lifecycle supports a root scripts.dev starting with vite. Configure other targets through the host Runtime UI.".into()));
            }
            let mut commands = self.command(Kind::Web, "run")?;
            let socket = std::net::TcpListener::bind("127.0.0.1:0").map_err(io_error)?;
            let address = socket.local_addr().map_err(io_error)?;
            commands[0].argv.extend([
                "--".into(),
                "--host".into(),
                "127.0.0.1".into(),
                "--port".into(),
                address.port().to_string(),
                "--strictPort".into(),
            ]);
            launch = Some(Launch {
                commands,
                readiness: Some(ReadyHttp {
                    address,
                    path: "/package.json".into(),
                    expected_body: String::from_utf8(package.bytes)
                        .map_err(|_| invalid("package.json UTF-8"))?,
                }),
                timeout_ms: 300000,
            });
            target.id.clone()
        } else {
            if action == "stop" {
                if operation.is_none() || args["operation"].as_str() != operation.as_deref() {
                    return Err(DomainError::Conflict);
                }
                if self.running.as_ref().is_none_or(|run| {
                    run.launch.readiness.is_none()
                        || !self
                            .project
                            .targets
                            .iter()
                            .any(|t| t.id == run.record.target && t.kind == Kind::Web)
                }) {
                    return Err(DomainError::PermissionDenied);
                }
            }
            self.running
                .as_ref()
                .map(|r| r.record.target.clone())
                .unwrap_or_else(|| self.project.id.clone())
        };
        let mut toolchain_hashes = vec![];
        if let Some(launch) = &launch {
            for command in &launch.commands {
                for path in std::iter::once(&command.executable).chain(
                    command
                        .argv
                        .first()
                        .map(PathBuf::from)
                        .as_ref()
                        .filter(|p| p.is_absolute() && p.is_file()),
                ) {
                    toolchain_hashes
                        .push((path.clone(), digest(&fs::read(path).map_err(io_error)?)));
                }
            }
        }
        let binding = Binding {
            workspace: self.workspace.id().as_str().into(),
            target,
            revision: self.revision()?,
            capabilities: [Capability::Preview].into(),
            vision: VisionAccess::default(),
        };
        let display = json!({"action":action,"operation":operation,"target":binding.target,"commands":launch.as_ref().map(|l| &l.commands),"lifetimeMs":launch.as_ref().map(|l| l.timeout_ms)});
        Ok(PreviewPlan {
            binding,
            display,
            action: action.into(),
            operation,
            launch,
            toolchain_hashes,
            configuration: serde_json::to_value(&self.toolchains)
                .map_err(|_| DomainError::Conflict)?,
        })
    }
    pub fn revalidate_model_preview(
        &self,
        plan: &PreviewPlan,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        control.check()?;
        if serde_json::to_value(&self.toolchains).map_err(|_| DomainError::Conflict)?
            != plan.configuration
        {
            return Err(DomainError::Conflict);
        }
        if self.workspace.id().as_str() != plan.binding.workspace
            || self.revision()? != plan.binding.revision
            || self.running.as_ref().map(|r| &r.record.id) != plan.operation.as_ref()
        {
            return Err(DomainError::Conflict);
        }
        for (path, expected) in &plan.toolchain_hashes {
            if digest(&fs::read(path).map_err(io_error)?) != *expected {
                return Err(DomainError::Conflict);
            }
        }
        Ok(())
    }
    pub fn execute_model_preview(
        &mut self,
        plan: PreviewPlan,
        control: &ScanControl<'_>,
    ) -> Result<AdapterOutcome, DomainError> {
        self.revalidate_model_preview(&plan, control)?;
        let effect = if plan.action == "status" {
            Effect::None
        } else {
            Effect::Committed
        };
        let operation = match plan.action.as_str() {
            "start" => Some(self.start(
                &plan.binding.target,
                "run",
                plan.launch.ok_or(DomainError::Conflict)?,
                control,
            )?),
            "stop" => self.stop()?.map(|r| r.id),
            _ => plan.operation,
        };
        // Poll once. Startup remains owned and asynchronous; never block the model
        // waiting for a server to stay alive or turn arbitrary URLs into grants.
        let state = if self.running.is_some() {
            Some(self.poll(control)?)
        } else {
            None
        };
        if state
            .as_ref()
            .is_some_and(|r| matches!(r.lifecycle, Lifecycle::Exited | Lifecycle::Failed))
        {
            self.finish(control)?;
        }
        let grant = self.preview_grant().ok();
        let value = json!({"action":plan.action,"operation":operation,"state":state.as_ref().map(|r| r.lifecycle),"ready":grant.is_some(),"url":grant.as_ref().map(|g| format!("{}/",g.origin)),"targets":self.project.targets.iter().filter(|t| t.kind==Kind::Web).map(|t| &t.id).collect::<Vec<_>>(),"nextAction":if grant.is_some(){"Use this exact url with screenshot_url, interact_then_screenshot or verify_ui."}else if self.running.is_some(){"Startup is in progress. Continue useful work, then call preview status."}else{"Use preview start for an installed Vite target. Failed targets can be inspected in the host Runtime logs."}});
        Ok(AdapterOutcome {
            result: json!({"content":[{"type":"text","text":value.to_string()}],"details":{"preview":value}}),
            effect,
            artifact_page: false,
        })
    }
}
