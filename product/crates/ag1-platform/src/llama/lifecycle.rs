//! A runtime observation never grants authority over the observed process.
use super::{LaunchPlan, Material};
use ag1_core::DomainError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeState {
    Inspect,
    Loading,
    Ready,
    Failed(String),
    Stopping,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelIdentity {
    pub path: Option<String>,
    pub alias: Option<String>,
    pub model: Option<String>,
    pub context_window: Option<u64>,
    pub vision: bool,
}
impl ModelIdentity {
    pub fn matches(&self, plan: &LaunchPlan) -> bool {
        // A matching alias must never override a contradictory weight path.
        // Some llama.cpp versions omit model_path; only then use the identity
        // exposed by that version's props response.
        let model = if let Some(path) = &self.path {
            path == &plan.model_path
        } else if let Some(alias) = &self.alias {
            alias == &plan.model_id
        } else {
            self.model
                .as_ref()
                .is_some_and(|m| m == &plan.model_id || m == &plan.model_path)
        };
        model
            && self.context_window.is_some_and(|loaded| {
                loaded >= plan.requested_context && loaded - plan.requested_context < 1024
            })
            && (plan.projector.is_none() || self.vision)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    Vacant,
    Occupied,
    Healthy(ModelIdentity),
}

/// The platform adapter owns non-serializable handles (pidfd/child or authenticated
/// supervisor connection). Historical PID/port records are not accepted by this API.
pub trait RuntimeHost {
    type Handle;
    fn verify_materials(&mut self, materials: &[Material]) -> Result<(), DomainError>;
    fn verify_progress(&mut self, materials: &[Material], _progress: &mut dyn FnMut(u8)) -> Result<(), DomainError> {
        self.verify_materials(materials)
    }
    fn loading_phase(&mut self, _handle: &Self::Handle) -> Option<&'static str> { None }
    fn observe(&mut self, plan: &LaunchPlan) -> Result<Observation, DomainError>;
    fn spawn(&mut self, plan: &LaunchPlan) -> Result<Self::Handle, DomainError>;
    /// Verifies both process lifetime and executable identity; false means exited.
    fn owned_alive(&mut self, handle: &Self::Handle) -> Result<bool, DomainError>;
    /// Called after the cancelled HTTP stream is closed. Idle observation settles
    /// generation without releasing the authenticated process or its loaded model.
    fn generation_idle(&mut self, _handle: &Self::Handle, _plan: &LaunchPlan) -> Result<bool, DomainError> {
        Err(DomainError::Unavailable("Owned generation status unavailable".into()))
    }
    /// Returns only after verified exit. Never signals a process found just by port.
    fn stop(&mut self, handle: &mut Self::Handle) -> Result<(), DomainError>;
    /// Optional authenticated runtime observation, never a control capability.
    fn resources(
        &mut self,
        _handle: &Self::Handle,
    ) -> Result<Option<serde_json::Value>, DomainError> {
        Ok(None)
    }
}
pub struct RuntimeController<H: RuntimeHost> {
    host: H,
    owned: Option<H::Handle>,
    plan_hash: Option<String>,
    state: RuntimeState,
}
impl<H: RuntimeHost> RuntimeController<H> {
    pub fn new(host: H) -> Self {
        Self {
            host,
            owned: None,
            plan_hash: None,
            state: RuntimeState::Inspect,
        }
    }
    pub fn state(&self) -> &RuntimeState {
        &self.state
    }
    pub fn owns_process(&self) -> bool {
        self.owned.is_some()
    }
    pub fn loading_phase(&mut self) -> Option<&'static str> {
        self.host.loading_phase(self.owned.as_ref()?)
    }
    pub fn owned_alive(&mut self) -> Result<bool, DomainError> {
        match self.owned.as_ref() {
            Some(handle) => self.host.owned_alive(handle),
            None => Ok(false),
        }
    }
    pub fn host(&self) -> &H {
        &self.host
    }
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }
    /// Reconcile an authenticated owned exit before admitting another request.
    /// Keep the handle until its supervisor/tree has verifiably settled; a dead
    /// model PID alone does not prove that all owned descendants have exited.
    pub fn reap_exited_owned_runtime(&mut self) -> Result<bool, DomainError> {
        let Some(handle) = self.owned.as_ref() else {
            return Ok(false);
        };
        if self.host.owned_alive(handle)? {
            return Ok(false);
        }
        self.stop_owned_runtime()?;
        Ok(true)
    }
    pub fn owned_resources(&mut self) -> Result<Option<serde_json::Value>, DomainError> {
        let handle = self.owned.as_ref().ok_or(DomainError::PermissionDenied)?;
        if !self.host.owned_alive(handle)? {
            return Err(DomainError::Unavailable("Owned runtime exited".into()));
        }
        self.host.resources(handle)
    }
    /// Readiness permits external HTTP use; it never installs an ownership handle.
    pub fn observe_external_runtime(
        &mut self,
        plan: &LaunchPlan,
    ) -> Result<Observation, DomainError> {
        let observation = self.host.observe(plan)?;
        self.state = match &observation {
            Observation::Healthy(id) if id.matches(plan) => RuntimeState::Ready,
            Observation::Vacant => RuntimeState::Inspect,
            _ => RuntimeState::Failed("external runtime identity/context conflict".into()),
        };
        Ok(observation)
    }
    pub fn start_owned_runtime(&mut self, plan: &LaunchPlan) -> Result<(), DomainError> {
        if self.owned.is_some() {
            return Err(DomainError::Conflict);
        }
        let result = self.start(plan);
        if let Err(error) = &result {
            self.state = RuntimeState::Failed(format!("{error:?}"));
        }
        result
    }
    fn start(&mut self, plan: &LaunchPlan) -> Result<(), DomainError> {
        if !plan
            .materials
            .iter()
            .any(|m| m.path == plan.command && m.kind == "executable")
            || !plan
                .materials
                .iter()
                .any(|m| m.path == plan.model_path && m.kind == "weight")
            || plan.projector.as_ref().is_some_and(|p| {
                !plan
                    .materials
                    .iter()
                    .any(|m| &m.path == p && m.kind == "projector")
            })
        {
            return Err(DomainError::Unavailable(
                "incomplete pinned runtime material".into(),
            ));
        }
        let seal = plan.fingerprint()?;
        if seal != plan.plan_sha256 {
            return Err(DomainError::Conflict);
        }
        self.host.verify_materials(&plan.runtime_materials()?)?;
        if self.host.observe(plan)? != Observation::Vacant {
            return Err(DomainError::Conflict);
        }
        self.state = RuntimeState::Loading;
        self.owned = Some(self.host.spawn(plan)?);
        self.plan_hash = Some(plan.plan_sha256.clone());
        Ok(())
    }
    /// Polling is clock/deadline-controlled by the composition host. A slow prefill
    /// is not a load failure; only verified exit, mismatch or explicit timeout is.
    pub fn poll_ready(&mut self, plan: &LaunchPlan) -> Result<bool, DomainError> {
        if self.plan_hash.as_deref() != Some(&plan.plan_sha256) {
            return Err(DomainError::Conflict);
        }
        let handle = self.owned.as_ref().ok_or(DomainError::PermissionDenied)?;
        match self.host.owned_alive(handle) {
            Ok(true) => (),
            Ok(false) => {
                self.stop_owned_runtime()?;
                self.state =
                    RuntimeState::Failed("owned runtime exited during load/generation".into());
                return Err(DomainError::Unavailable("runtime exited".into()));
            }
            Err(error) => {
                self.state = RuntimeState::Failed("owned runtime identity conflict".into());
                return Err(error);
            }
        }
        // A health HTTP deadline is not the owned model's load deadline.
        // Ownership and liveness were verified above; keep polling a slow
        // loading child while the composition host enforces its overall bound.
        let observation = match self.host.observe(plan) {
            Err(DomainError::TimedOut) => {
                self.state = RuntimeState::Loading;
                return Ok(false);
            }
            other => other?,
        };
        match observation {
            Observation::Healthy(id) if id.matches(plan) => {
                self.state = RuntimeState::Ready;
                Ok(true)
            }
            Observation::Healthy(_) => {
                self.state = RuntimeState::Failed("loaded model/context conflict".into());
                Err(DomainError::Conflict)
            }
            _ => {
                self.state = RuntimeState::Loading;
                Ok(false)
            }
        }
    }
    pub fn stop_owned_runtime(&mut self) -> Result<(), DomainError> {
        let Some(handle) = self.owned.as_mut() else {
            return Ok(());
        };
        self.state = RuntimeState::Stopping;
        self.host.stop(handle)?;
        self.owned = None;
        self.plan_hash = None;
        self.state = RuntimeState::Inspect;
        Ok(())
    }
    pub fn poll_generation_idle(&mut self, plan: &LaunchPlan) -> Result<bool, DomainError> {
        if self.plan_hash.as_deref() != Some(&plan.plan_sha256) {
            return Err(DomainError::Conflict);
        }
        let handle = self.owned.as_ref().ok_or(DomainError::PermissionDenied)?;
        if !self.host.owned_alive(handle)? {
            return Err(DomainError::Unavailable("Owned runtime exited".into()));
        }
        let idle = self.host.generation_idle(handle, plan)?;
        if !self.host.owned_alive(handle)? {
            return Err(DomainError::Unavailable("Owned runtime exited".into()));
        }
        Ok(idle)
    }
    pub fn timeout_loading(&mut self) -> Result<(), DomainError> {
        self.stop_owned_runtime()?;
        self.state = RuntimeState::Failed("load timeout".into());
        Err(DomainError::TimedOut)
    }
}

/// A linear handoff keeps the authenticated supervisor handle alive. It cannot be
/// cloned, serialized, constructed from a PID, or replayed after host death.
pub struct RuntimeHandoff<H: RuntimeHost> {
    controller: RuntimeController<H>,
}
impl<H: RuntimeHost> RuntimeController<H> {
    pub fn handoff(self) -> RuntimeHandoff<H> {
        RuntimeHandoff { controller: self }
    }
}
impl<H: RuntimeHost> RuntimeHandoff<H> {
    /// Failure returns the still-owned handoff so the host can retry or stop it.
    pub fn restore(
        mut self,
        plan: &LaunchPlan,
    ) -> Result<RuntimeController<H>, (DomainError, Self)> {
        let verify = (|| {
            if plan.fingerprint()? != plan.plan_sha256
                || self.controller.plan_hash.as_deref() != Some(&plan.plan_sha256)
            {
                return Err(DomainError::Conflict);
            }
            self.controller
                .host
                .verify_materials(&plan.runtime_materials()?)?;
            self.controller.poll_ready(plan)?;
            Ok(())
        })();
        match verify {
            Ok(()) => Ok(self.controller),
            Err(error) => Err((error, self)),
        }
    }
    pub fn stop(mut self) -> Result<(), DomainError> {
        self.controller.stop_owned_runtime()
    }
}
