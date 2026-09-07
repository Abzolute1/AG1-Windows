use super::{Launch, Runtime};
use crate::{resource_fs::ScanControl, target_tools::Port};
use ag1_core::{
    DomainError,
    projects::{Kind, Lifecycle},
    tools::{
        executor::{AdapterOutcome, Effect, ExecutionArtifacts},
        targets::{Binding, Capability, Request, VisionAccess},
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub struct CheckPort {
    pub runtime: Runtime,
    pub kind: Kind,
}
pub struct Plan {
    binding: Binding,
    launch: Launch,
    toolchain_hashes: Vec<(std::path::PathBuf, String)>,
    configuration: Value,
    files: Vec<String>,
}
impl CheckPort {
    fn commands(&self, files: &[String]) -> Result<Vec<super::Command>, DomainError> {
        let mut commands = self.runtime.command(self.kind, "check")?;
        if !files.is_empty()
            && super::checks::detect(&self.runtime.workspace, &self.runtime.toolchains)
                .is_ok_and(|(kind, _)| kind == "typescript")
        {
            // Preserve reference append semantics; a filename cannot become a compiler option.
            commands[0]
                .argv
                .extend(files.iter().map(|f| format!("./{f}")));
        }
        Ok(commands)
    }
}
impl Port for CheckPort {
    type Plan = Plan;
    fn prepare(
        &mut self,
        request: &Request,
        control: &ScanControl<'_>,
    ) -> Result<Plan, DomainError> {
        control.check()?;
        if request.name() != "check" {
            return Err(DomainError::Unavailable(
                "Use an admitted visual target".into(),
            ));
        }
        let files: Vec<String> = request.arguments()["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                let name = v
                    .as_str()
                    .ok_or_else(|| DomainError::InvalidInput("check file".into()))?;
                self.runtime.workspace.snapshot(name, 8 * 1024 * 1024)?;
                Ok(name.to_string())
            })
            .collect::<Result<_, DomainError>>()?;
        let target = self
            .runtime
            .project
            .targets
            .iter()
            .find(|t| t.kind == self.kind)
            .ok_or(DomainError::NotFound)?;
        let launch = Launch {
            commands: self.commands(&files)?,
            readiness: None,
            timeout_ms: 300000,
        };
        let mut toolchain_hashes = vec![];
        for c in &launch.commands {
            toolchain_hashes.push((c.executable.clone(), hash(&c.executable)?));
            if let Some(script) = c
                .argv
                .first()
                .map(std::path::PathBuf::from)
                .filter(|p| p.is_absolute() && p.is_file())
            {
                toolchain_hashes.push((script.clone(), hash(&script)?));
            }
        }
        let binding = Binding {
            workspace: self.runtime.workspace.id().as_str().into(),
            target: target.id.clone(),
            revision: self.runtime.revision()?,
            capabilities: BTreeSet::from([Capability::Check]),
            vision: VisionAccess::default(),
        };
        Ok(Plan {
            binding,
            launch,
            toolchain_hashes,
            configuration: serde_json::to_value(&self.runtime.toolchains).unwrap(),
            files,
        })
    }
    fn binding(&self, plan: &Plan) -> Binding {
        plan.binding.clone()
    }
    fn display(&self, plan: &Plan) -> Value {
        json!({"commands":plan.launch.commands,"kind":self.kind})
    }
    fn revalidate(&mut self, plan: &Plan, control: &ScanControl<'_>) -> Result<(), DomainError> {
        control.check()?;
        if serde_json::to_value(&self.runtime.toolchains).unwrap() != plan.configuration
            || self.runtime.revision() != Ok(plan.binding.revision.clone())
        {
            return Err(DomainError::Conflict);
        }
        if serde_json::to_value(self.commands(&plan.files)?).ok()
            != serde_json::to_value(&plan.launch.commands).ok()
        {
            return Err(DomainError::Conflict);
        }
        for (path, expected) in &plan.toolchain_hashes {
            if hash(path)? != *expected {
                return Err(DomainError::Conflict);
            }
        }
        Ok(())
    }
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        plan: Plan,
        _request: &Request,
        control: &ScanControl<'_>,
        artifacts: &mut A,
    ) -> Result<AdapterOutcome, DomainError> {
        self.revalidate(&plan, control)?;
        self.runtime
            .start(&plan.binding.target, "check", plan.launch, control)?;
        let outcome = self.runtime.finish(control);
        if outcome.is_err() {
            self.runtime.stop()?;
        }
        let record = outcome?;
        let mut logs = vec![];
        for n in 0..record.logs.len() {
            let mut bytes = vec![];
            loop {
                let page = self.runtime.log(&record.id, n, bytes.len() as u64, 32768)?;
                if page.is_empty() {
                    break;
                }
                bytes.extend(page);
            }
            logs.push(artifacts.put_evidence(
                &String::from_utf8_lossy(&bytes),
                "test-output",
                &json!({"type":"tool","name":"check","callId":record.id,"stream":n}),
                &[],
                None,
            )?);
        }
        let current_revision = self.runtime.revision()?;
        let passed = record.lifecycle == Lifecycle::Exited
            && record.exits.iter().all(|e| *e == Some(0))
            && current_revision == record.source_revision;
        Ok(AdapterOutcome {
            result: json!({"content":[{"type":"text","text":serde_json::to_string(&record).unwrap()}],"isError":!passed,"details":{"passed":passed,"commands":record.commands,"exits":record.exits,"sourceRevision":record.source_revision,"currentRevision":current_revision,"logs":logs,"runtimeProof":false,"visualProof":false}}),
            effect: Effect::Committed,
            artifact_page: false,
        })
    }
}
fn hash(path: &std::path::Path) -> Result<String, DomainError> {
    let bytes = std::fs::read(path).map_err(crate::io_error)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Visual capture uses an independent browser. No implicit judge/model call is
/// made; verify_ui returns exact evidence requiring assessment by the caller.
pub struct VisualPort {
    pub runtime: Runtime,
    pub browser: super::preview::Browser,
    pub grant: super::preview::Grant,
    pub store: std::sync::Arc<ag1_storage::Store>,
    pub session: String,
    pub origin: String,
    pub vision: VisionAccess,
}
pub struct VisualPlan {
    binding: Binding,
    grant: super::preview::Grant,
    toolchain_hashes: Vec<(std::path::PathBuf, String)>,
}
impl Port for VisualPort {
    type Plan = VisualPlan;
    fn prepare(
        &mut self,
        request: &Request,
        control: &ScanControl<'_>,
    ) -> Result<VisualPlan, DomainError> {
        control.check()?;
        self.runtime.verify_preview(&self.grant)?;
        if !self.browser.available().available {
            return Err(DomainError::Unavailable(self.browser.available().reason));
        }
        if !request.arguments()["url"]
            .as_str()
            .is_some_and(|url| self.grant.allows(&self.grant.operation, url))
        {
            return Err(DomainError::PermissionDenied);
        }
        for a in request.arguments()["actions"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if a["type"] == "navigate"
                && !a["url"]
                    .as_str()
                    .is_some_and(|url| self.grant.allows(&self.grant.operation, url))
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        let binding = Binding {
            workspace: self.runtime.workspace.id().as_str().into(),
            target: self.grant.operation.clone(),
            revision: self.runtime.revision()?,
            capabilities: BTreeSet::from([Capability::Screenshot, Capability::Interact]),
            vision: VisionAccess {
                caller: self.vision.caller,
                judge: false,
            },
        };
        binding.admits(request)?;
        Ok(VisualPlan {
            binding,
            grant: self.grant.clone(),
            toolchain_hashes: [
                &self.browser.driver,
                &self.browser.executable,
                &self.browser.node,
                &self.browser.playwright,
                &self.browser.helper,
            ]
            .into_iter()
            .map(|p| Ok((p.clone(), hash(p)?)))
            .collect::<Result<_, DomainError>>()?,
        })
    }
    fn binding(&self, plan: &VisualPlan) -> Binding {
        plan.binding.clone()
    }
    fn display(&self, plan: &VisualPlan) -> Value {
        json!({"origin":plan.grant.origin,"operation":plan.grant.operation,"browser":self.browser.executable,"assessment":"caller vision; no separate judge"})
    }
    fn revalidate(
        &mut self,
        plan: &VisualPlan,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        control.check()?;
        self.runtime.verify_preview(&self.grant)?;
        if self.grant.operation != plan.grant.operation
            || !self.grant.open
            || self.runtime.revision() != Ok(plan.binding.revision.clone())
            || plan
                .toolchain_hashes
                .iter()
                .any(|(path, expected)| hash(path).as_ref() != Ok(expected))
            || !self.vision.caller
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        plan: VisualPlan,
        request: &Request,
        control: &ScanControl<'_>,
        artifacts: &mut A,
    ) -> Result<AdapterOutcome, DomainError> {
        self.revalidate(&plan, control)?;
        let observation = self
            .browser
            .observe(&self.grant, request.arguments(), control)?;
        self.runtime.verify_preview(&self.grant)?;
        let artifact = super::preview::retain(
            &self.store,
            &self.session,
            &self.origin,
            &observation.png,
            "web-visual",
            request.arguments()["claim"]
                .as_str()
                .unwrap_or("Inspect web preview"),
            &self.grant.origin,
        )?;
        let evidence = artifacts.put_evidence(
            &observation.result.to_string(),
            "visual-observation",
            &json!({"type":"tool","name":request.name(),"callId":self.grant.operation}),
            &[],
            None,
        )?;
        use base64::Engine;
        Ok(AdapterOutcome {
            result: json!({"content":[{"type":"text","text":json!({"artifact":artifact,"observation":evidence,"assessment_required":true,"runtime_errors":observation.result["errors"]}).to_string()},{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(&observation.png)}],"details":{"artifact":artifact,"assessment_required":true,"visualProof":true,"runtimeProof":false,"browserRssBytes":observation.rss_bytes,"elapsedMs":observation.elapsed_ms}}),
            effect: Effect::Committed,
            artifact_page: false,
        })
    }
}

pub enum ProjectPort {
    Shared(SharedProject),
    SharedCheck(SharedCheck),
    Check(Box<CheckPort>),
    Visual(Box<VisualPort>),
}
pub enum ProjectPlan {
    Check(Plan),
    Visual(VisualPlan),
}
impl Port for ProjectPort {
    type Plan = ProjectPlan;
    fn prepare(&mut self, r: &Request, c: &ScanControl<'_>) -> Result<ProjectPlan, DomainError> {
        match self {
            Self::Shared(p) => p.prepare(r, c),
            Self::SharedCheck(p) => p.prepare(r, c).map(ProjectPlan::Check),
            Self::Check(p) => p.prepare(r, c).map(ProjectPlan::Check),
            Self::Visual(p) => p.prepare(r, c).map(ProjectPlan::Visual),
        }
    }
    fn binding(&self, p: &ProjectPlan) -> Binding {
        match p {
            ProjectPlan::Check(p) => p.binding.clone(),
            ProjectPlan::Visual(p) => p.binding.clone(),
        }
    }
    fn display(&self, p: &ProjectPlan) -> Value {
        match (self, p) {
            (Self::Shared(s), p) => s.display(p),
            (Self::SharedCheck(s), ProjectPlan::Check(p)) => s.display(p),
            (Self::Check(s), ProjectPlan::Check(p)) => s.display(p),
            (Self::Visual(s), ProjectPlan::Visual(p)) => s.display(p),
            _ => json!({"unavailable":"target changed"}),
        }
    }
    fn revalidate(&mut self, p: &ProjectPlan, c: &ScanControl<'_>) -> Result<(), DomainError> {
        match (self, p) {
            (Self::Shared(s), p) => s.revalidate(p, c),
            (Self::SharedCheck(s), ProjectPlan::Check(p)) => s.revalidate(p, c),
            (Self::Check(s), ProjectPlan::Check(p)) => s.revalidate(p, c),
            (Self::Visual(s), ProjectPlan::Visual(p)) => s.revalidate(p, c),
            _ => Err(DomainError::Conflict),
        }
    }
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        p: ProjectPlan,
        r: &Request,
        c: &ScanControl<'_>,
        a: &mut A,
    ) -> Result<AdapterOutcome, DomainError> {
        match (self, p) {
            (Self::Shared(s), p) => s.execute(p, r, c, a),
            (Self::SharedCheck(s), ProjectPlan::Check(p)) => s.execute(p, r, c, a),
            (Self::Check(s), ProjectPlan::Check(p)) => s.execute(p, r, c, a),
            (Self::Visual(s), ProjectPlan::Visual(p)) => s.execute(p, r, c, a),
            _ => Err(DomainError::Conflict),
        }
    }
}

/// One runtime catalog is shared by explicit client controls and model checks.
/// A busy tool keeps its owner; GUI polling uses try_lock and cannot block it.
pub struct SharedCheck {
    pub runtime: std::sync::Arc<std::sync::Mutex<Option<Runtime>>>,
}
impl SharedCheck {
    pub fn available(&self) -> bool {
        self.runtime
            .try_lock()
            .ok()
            .and_then(|slot| {
                slot.as_ref().map(|runtime| {
                    runtime
                        .project
                        .targets
                        .iter()
                        .any(|t| runtime.command(t.kind, "check").is_ok())
                })
            })
            .unwrap_or(false)
    }
    fn with<T>(
        &self,
        call: impl FnOnce(&mut CheckPort) -> Result<T, DomainError>,
    ) -> Result<T, DomainError> {
        let mut slot = self
            .runtime
            .try_lock()
            .map_err(|_| DomainError::QueueFull)?;
        let runtime = slot.as_ref().ok_or(DomainError::NotFound)?;
        let kind = runtime
            .project
            .targets
            .iter()
            .find(|t| runtime.command(t.kind, "check").is_ok())
            .map(|t| t.kind)
            .ok_or_else(|| DomainError::Unavailable("No configured check target".into()))?;
        struct Return<'a> {
            slot: &'a mut Option<Runtime>,
            port: Option<CheckPort>,
        }
        impl Drop for Return<'_> {
            fn drop(&mut self) {
                if let Some(port) = self.port.take() {
                    *self.slot = Some(port.runtime);
                }
            }
        }
        let port = CheckPort {
            runtime: slot.take().ok_or(DomainError::Conflict)?,
            kind,
        };
        let mut owner = Return {
            slot: &mut slot,
            port: Some(port),
        };
        call(owner.port.as_mut().ok_or(DomainError::Conflict)?)
    }
}
impl Port for SharedCheck {
    type Plan = Plan;
    fn prepare(&mut self, r: &Request, c: &ScanControl<'_>) -> Result<Plan, DomainError> {
        self.with(|p| p.prepare(r, c))
    }
    fn binding(&self, p: &Plan) -> Binding {
        p.binding.clone()
    }
    fn display(&self, p: &Plan) -> Value {
        json!({"commands":p.launch.commands,"target":p.binding.target})
    }
    fn revalidate(&mut self, p: &Plan, c: &ScanControl<'_>) -> Result<(), DomainError> {
        self.with(|port| port.revalidate(p, c))
    }
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        p: Plan,
        r: &Request,
        c: &ScanControl<'_>,
        a: &mut A,
    ) -> Result<AdapterOutcome, DomainError> {
        self.with(|port| port.execute(p, r, c, a))
    }
}

/// Client composition of the existing check and visual adapters over one owner.
/// Each operation revalidates the live runtime and current model capability.
pub struct SharedProject {
    pub check: SharedCheck,
    pub browser: super::preview::Browser,
    pub store: std::sync::Arc<ag1_storage::Store>,
    pub session: String,
    pub caller: std::sync::Mutex<std::sync::Weak<crate::session::NativeSession>>,
}
impl SharedProject {
    pub fn names(&self) -> Vec<String> {
        let mut names = vec![];
        if self.check.available() {
            names.push("check".into());
        }
        // Configured tools remain discoverable before a preview is started.
        // prepare() checks readiness and actual selected-model vision.
        if self.browser.available().available {
            names.extend(
                ["screenshot_url", "interact_then_screenshot", "verify_ui"].map(str::to_owned),
            );
        }
        names
    }
    fn with_visual<T>(
        &self,
        call: impl FnOnce(&mut VisualPort) -> Result<T, DomainError>,
    ) -> Result<T, DomainError> {
        let caller = self
            .caller
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .upgrade()
            .ok_or(DomainError::Cancelled)?;
        let model = caller.configuration()?.model;
        let vision = model["input"]
            .as_array()
            .is_some_and(|v| v.iter().any(|s| s == "image"));
        let mut slot = self
            .check
            .runtime
            .try_lock()
            .map_err(|_| DomainError::QueueFull)?;
        let grant = slot
            .as_ref()
            .ok_or(DomainError::NotFound)?
            .preview_grant()?;
        struct Return<'a> {
            slot: &'a mut Option<Runtime>,
            port: Option<VisualPort>,
        }
        impl Drop for Return<'_> {
            fn drop(&mut self) {
                if let Some(p) = self.port.take() {
                    *self.slot = Some(p.runtime);
                }
            }
        }
        let port = VisualPort {
            runtime: slot.take().ok_or(DomainError::Conflict)?,
            browser: self.browser.clone(),
            grant,
            store: self.store.clone(),
            session: self.session.clone(),
            origin: self.session.clone(),
            vision: VisionAccess {
                caller: vision,
                judge: false,
            },
        };
        let mut owner = Return {
            slot: &mut slot,
            port: Some(port),
        };
        call(owner.port.as_mut().ok_or(DomainError::Conflict)?)
    }
}
impl Port for SharedProject {
    type Plan = ProjectPlan;
    fn prepare(&mut self, r: &Request, c: &ScanControl<'_>) -> Result<ProjectPlan, DomainError> {
        if r.name() == "check" {
            self.check.prepare(r, c).map(ProjectPlan::Check)
        } else {
            self.with_visual(|p| p.prepare(r, c))
                .map(ProjectPlan::Visual)
        }
    }
    fn binding(&self, p: &ProjectPlan) -> Binding {
        match p {
            ProjectPlan::Check(p) => p.binding.clone(),
            ProjectPlan::Visual(p) => p.binding.clone(),
        }
    }
    fn display(&self, p: &ProjectPlan) -> Value {
        match p {
            ProjectPlan::Check(p) => self.check.display(p),
            ProjectPlan::Visual(p) => {
                json!({"target":p.binding.target,"origin":p.grant.origin,"capabilities":p.binding.capabilities})
            }
        }
    }
    fn revalidate(&mut self, p: &ProjectPlan, c: &ScanControl<'_>) -> Result<(), DomainError> {
        match p {
            ProjectPlan::Check(p) => self.check.revalidate(p, c),
            ProjectPlan::Visual(p) => self.with_visual(|port| port.revalidate(p, c)),
        }
    }
    fn execute<A: ExecutionArtifacts>(
        &mut self,
        p: ProjectPlan,
        r: &Request,
        c: &ScanControl<'_>,
        a: &mut A,
    ) -> Result<AdapterOutcome, DomainError> {
        match p {
            ProjectPlan::Check(p) => self.check.execute(p, r, c, a),
            ProjectPlan::Visual(p) => self.with_visual(|port| port.execute(p, r, c, a)),
        }
    }
}
