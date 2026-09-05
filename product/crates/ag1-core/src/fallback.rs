//! Explicit pre-generation fallback policy from the pinned NVIDIA adapter.
//! A selected generation has no transition back to candidate selection.
use crate::DomainError;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelRoute {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub configured_auth: bool,
}
impl ModelRoute {
    pub fn nvidia(&self) -> bool {
        ["nvidia", "nvidia-nim"].contains(&self.provider.as_str())
            || self.endpoint.contains("integrate.api.nvidia.com")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Auth,
    Probe,
    Selected,
    Failed,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SelectionStep {
    ResolveAuth(ModelRoute),
    ProbeNvidia(ModelRoute),
    Generate(ModelRoute),
}
pub struct FallbackSelection {
    candidates: Vec<ModelRoute>,
    index: usize,
    state: State,
    allow_fallback: bool,
}
impl FallbackSelection {
    pub fn new(primary: ModelRoute, registry: &[ModelRoute]) -> Result<Self, DomainError> {
        if registry.len() > 4096 {
            return Err(DomainError::LimitExceeded("model registry".into()));
        }
        let allow_fallback = primary.nvidia();
        let mut candidates = vec![primary];
        if allow_fallback {
            let cloud = registry.iter().find(|m| {
                m.provider == "openrouter" && m.model == "nvidia/nemotron-3-ultra-550b-a55b:free"
            });
            let local = registry.iter().find(|m| m.provider == "local");
            for candidate in [cloud, local].into_iter().flatten() {
                if candidate.configured_auth
                    && !candidates
                        .iter()
                        .any(|m| m.provider == candidate.provider && m.model == candidate.model)
                {
                    candidates.push(candidate.clone());
                }
            }
        }
        Ok(Self {
            candidates,
            index: 0,
            state: State::Auth,
            allow_fallback,
        })
    }
    pub fn current(&self) -> Option<&ModelRoute> {
        self.candidates.get(self.index)
    }
    fn advance(&mut self) -> Result<SelectionStep, DomainError> {
        self.index += 1;
        if self.allow_fallback
            && let Some(next) = self.current().cloned()
        {
            self.state = State::Auth;
            return Ok(SelectionStep::ResolveAuth(next));
        }
        self.state = State::Failed;
        Err(DomainError::Unavailable(
            "no configured provider candidate could authenticate".into(),
        ))
    }
    pub fn resolved_auth(&mut self, ok: bool) -> Result<SelectionStep, DomainError> {
        if self.state != State::Auth {
            return Err(DomainError::Conflict);
        }
        if !ok {
            return self.advance();
        }
        let candidate = self.current().cloned().ok_or(DomainError::NotFound)?;
        if candidate.nvidia() {
            self.state = State::Probe;
            Ok(SelectionStep::ProbeNvidia(candidate))
        } else {
            self.state = State::Selected;
            Ok(SelectionStep::Generate(candidate))
        }
    }
    pub fn preflight_status(&mut self, status: u16) -> Result<SelectionStep, DomainError> {
        if self.state != State::Probe {
            return Err(DomainError::Conflict);
        }
        if status == 403 {
            return self.advance();
        }
        if !(200..300).contains(&status) {
            self.state = State::Failed;
            return Err(DomainError::Unavailable(format!(
                "NVIDIA preflight HTTP {status}"
            )));
        }
        self.state = State::Selected;
        Ok(SelectionStep::Generate(
            self.current().cloned().ok_or(DomainError::NotFound)?,
        ))
    }
}
