//! Native dynamic provider registration for one admitted extension host. Catalog
//! mutation, OAuth access and eventual inference admission are distinct grants.
use crate::{
    extension_host::{Bridge, ExtensionHost, NativeRequest, NativeStream, ProviderInvocation},
    resource_fs::ScanControl,
};
use ag1_core::{
    DomainError,
    resources::{
        extension_providers::{HookBinding, Hooks, Registry, Snapshot},
        packages,
    },
    tools::registry::Scope,
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};
pub struct Grant {
    pub extension: String,
    pub providers: BTreeSet<String>,
}
/// Credentials are returned only for the named, admitted hook owner. Implementations
/// use the native vault and never resolve arbitrary credential files from RPC.
pub trait OAuth {
    fn credentials(
        &mut self,
        provider: &str,
        extension: &str,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError>;
}
pub struct NoOAuth;
impl OAuth for NoOAuth {
    fn credentials(
        &mut self,
        _: &str,
        _: &str,
        _: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        Ok(None)
    }
}
#[derive(Clone)]
struct Token {
    binding: HookBinding,
    token: String,
    oauth_hooks: BTreeSet<String>,
}
#[derive(Clone)]
struct State {
    registry: Registry,
    tokens: BTreeMap<u64, Token>,
}
/// Actor-local catalog shared with the native model resolver. Raw request configs
/// stay native; the JS acknowledgement carries model descriptors and hook tokens.
#[derive(Clone)]
pub struct Catalog {
    state: Rc<RefCell<State>>,
    scope: Scope,
    host: u64,
}
#[derive(Clone)]
pub struct CustomTarget {
    catalog: Catalog,
    generation: u64,
    model: Value,
    binding: HookBinding,
    token: String,
}
#[derive(Clone)]
pub struct OAuthTarget {
    catalog: Catalog,
    generation: u64,
    binding: HookBinding,
    token: String,
    hooks: BTreeSet<String>,
}
impl OAuthTarget {
    pub(crate) fn accepts_callback(&self, request: &NativeRequest<'_>) -> bool {
        request.scope == &self.catalog.scope
            && request
                .operation
                .starts_with(&format!("extension-{}-", self.catalog.host))
            && request.extension == self.binding.extension
            && request.phase == "provider"
    }
    pub fn binding(&self) -> &HookBinding {
        &self.binding
    }
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
    pub fn supports(&self, hook: &str) -> bool {
        self.hooks.contains(hook)
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        let state = self
            .catalog
            .state
            .try_borrow()
            .map_err(|_| DomainError::Conflict)?;
        if state.registry.generation() != self.generation
            || state
                .registry
                .snapshot()
                .oauth_bindings
                .get(&self.binding.provider)
                != Some(&self.binding)
            || !state
                .tokens
                .get(&self.binding.registration)
                .is_some_and(|token| token.binding == self.binding && token.token == self.token)
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}
impl CustomTarget {
    pub fn model(&self) -> &Value {
        &self.model
    }
    pub fn binding(&self) -> &HookBinding {
        &self.binding
    }
    pub(crate) fn token(&self) -> &str {
        &self.token
    }
    pub fn validate(&self) -> Result<(), DomainError> {
        let state = self
            .catalog
            .state
            .try_borrow()
            .map_err(|_| DomainError::Conflict)?;
        if state.registry.generation() != self.generation
            || !state
                .tokens
                .get(&self.binding.registration)
                .is_some_and(|t| t.binding == self.binding && t.token == self.token)
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}
impl Catalog {
    /// Native controller entrypoint; possession of the owned host establishes its
    /// scope/incarnation without fabricating a helper operation identifier.
    pub fn oauth_target_for_host(
        &self,
        host: &ExtensionHost,
        provider: &str,
    ) -> Result<Option<OAuthTarget>, DomainError> {
        if host.scope() != &self.scope || host.identity() != self.host {
            return Err(DomainError::PermissionDenied);
        }
        self.oauth_target(host.scope(), &format!("extension-{}-", self.host), provider)
    }
    /// Resolves a hook version only. The authentication service separately grants
    /// credential access and login to its admitted owner/caller.
    pub fn oauth_target(
        &self,
        scope: &Scope,
        host_operation: &str,
        provider: &str,
    ) -> Result<Option<OAuthTarget>, DomainError> {
        if scope != &self.scope || !host_operation.starts_with(&format!("extension-{}-", self.host))
        {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.state.try_borrow().map_err(|_| DomainError::Conflict)?;
        let Some(binding) = state.registry.snapshot().oauth_bindings.get(provider) else {
            return Ok(None);
        };
        let token = state
            .tokens
            .get(&binding.registration)
            .filter(|token| token.binding == *binding)
            .ok_or(DomainError::PermissionDenied)?;
        Ok(Some(OAuthTarget {
            catalog: self.clone(),
            generation: state.registry.generation(),
            binding: binding.clone(),
            token: token.token.clone(),
            hooks: token.oauth_hooks.clone(),
        }))
    }
    /// Resolves data for the native scheduler/credential owner. It does not grant
    /// model access, endpoint access or invocation merely by finding a hook.
    pub fn custom_target(
        &self,
        scope: &Scope,
        host_operation: &str,
        provider: &str,
        model_id: &str,
    ) -> Result<Option<CustomTarget>, DomainError> {
        if scope != &self.scope || !host_operation.starts_with(&format!("extension-{}-", self.host))
        {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.state.try_borrow().map_err(|_| DomainError::Conflict)?;
        let snapshot = state.registry.snapshot();
        let Some(model) = snapshot
            .models
            .iter()
            .find(|m| m["provider"] == provider && m["id"] == model_id)
        else {
            return Ok(None);
        };
        let Some(binding) = model["api"]
            .as_str()
            .and_then(|api| snapshot.stream_bindings.get(api))
        else {
            return Ok(None);
        };
        let token = state
            .tokens
            .get(&binding.registration)
            .filter(|t| t.binding == *binding)
            .ok_or(DomainError::PermissionDenied)?;
        Ok(Some(CustomTarget {
            catalog: self.clone(),
            generation: state.registry.generation(),
            model: model.clone(),
            binding: binding.clone(),
            token: token.token.clone(),
        }))
    }
    pub fn generation(&self) -> Result<u64, DomainError> {
        Ok(self
            .state
            .try_borrow()
            .map_err(|_| DomainError::Conflict)?
            .registry
            .generation())
    }
    pub fn models(&self) -> Result<Vec<Value>, DomainError> {
        let state = self.state.try_borrow().map_err(|_| DomainError::Conflict)?;
        public_models(state.registry.snapshot())
    }
}
fn public_models(snapshot: &Snapshot) -> Result<Vec<Value>, DomainError> {
    let models = snapshot.models.clone();
    for model in &models {
        if ["apiKey", "credentials", "authorization"]
            .iter()
            .any(|key| model.get(*key).is_some())
        {
            return Err(DomainError::PermissionDenied);
        }
    }
    let value = json!(models);
    packages::validate_settings_document(&value)?;
    if value.to_string().len() > 768 * 1024 {
        return Err(DomainError::LimitExceeded(
            "extension model catalog projection".into(),
        ));
    }
    Ok(models)
}
pub struct RegistryBridge<O, B> {
    scope: Scope,
    operation_prefix: String,
    grants: BTreeMap<String, BTreeSet<String>>,
    catalog: Catalog,
    oauth: O,
    fallback: B,
}
struct HookPort<'a, 'b, O, B> {
    tokens: &'a BTreeMap<u64, Token>,
    oauth: &'a mut O,
    fallback: &'a mut B,
    stream: &'a mut dyn NativeStream,
    control: &'a ScanControl<'b>,
}
impl<O: OAuth, B: Bridge> Hooks for HookPort<'_, '_, O, B> {
    fn modify_models(
        &mut self,
        binding: &HookBinding,
        models: &[Value],
    ) -> Result<Option<Vec<Value>>, DomainError> {
        self.control.check()?;
        let token = self
            .tokens
            .get(&binding.registration)
            .filter(|token| token.binding == *binding)
            .ok_or(DomainError::PermissionDenied)?;
        let Some(credentials) =
            self.oauth
                .credentials(&binding.provider, &binding.extension, self.control)?
        else {
            return Ok(None);
        };
        packages::validate_settings_document(&credentials)?;
        if credentials.to_string().len() > 65536 {
            return Err(DomainError::LimitExceeded(
                "extension OAuth credentials".into(),
            ));
        }
        let value = self
            .stream
            .invoke_provider(
                ProviderInvocation {
                    deadline: None,
                    name: &binding.provider,
                    registration: &token.token,
                    hook: "oauth.modifyModels",
                    arguments: &json!({"models":models,"credentials":credentials}),
                },
                self.fallback,
            )?
            .ok_or_else(|| {
                DomainError::InvalidInput("OAuth model hook returned undefined".into())
            })?;
        self.control.check()?;
        value
            .as_array()
            .cloned()
            .map(Some)
            .ok_or_else(|| DomainError::InvalidInput("OAuth model hook result".into()))
    }
}
impl<O: OAuth, B: Bridge> RegistryBridge<O, B> {
    pub fn new(
        host: &ExtensionHost,
        baseline: Snapshot,
        grants: Vec<Grant>,
        oauth: O,
        fallback: B,
    ) -> Result<Self, DomainError> {
        if grants.len() > 64
            || !baseline.stream_bindings.is_empty()
            || !baseline.oauth_bindings.is_empty()
        {
            return Err(DomainError::PermissionDenied);
        }
        let mut owners = BTreeMap::new();
        for grant in grants {
            if !host.admits_extension(&grant.extension)
                || grant.providers.len() > 256
                || grant.providers.iter().any(|name| {
                    name.is_empty() || name.len() > 256 || name.chars().any(char::is_control)
                })
                || owners.insert(grant.extension, grant.providers).is_some()
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        let registry = Registry::new(baseline)?;
        public_models(registry.snapshot())?;
        Ok(Self {
            scope: host.scope().clone(),
            operation_prefix: format!("extension-{}-", host.identity()),
            grants: owners,
            catalog: Catalog {
                scope: host.scope().clone(),
                host: host.identity(),
                state: Rc::new(RefCell::new(State {
                    registry,
                    tokens: BTreeMap::new(),
                })),
            },
            oauth,
            fallback,
        })
    }
    pub fn catalog(&self) -> Catalog {
        self.catalog.clone()
    }
    /// Native credential-change entrypoint. Rebuild atomically, then acknowledge
    /// the new public model view in the owning helper. No extension RPC grants
    /// access to this operation. Credential changes are independently durable.
    pub fn refresh(
        &mut self,
        host: &mut ExtensionHost,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        control.check()?;
        if host.scope() != &self.scope || host.identity() != self.catalog.host {
            return Err(DomainError::PermissionDenied);
        }
        if !host.is_alive() {
            return Err(DomainError::Unavailable("extension host stopped".into()));
        }
        let mut candidate = self
            .catalog
            .state
            .try_borrow()
            .map_err(|_| DomainError::Conflict)?
            .clone();
        let expected = candidate.registry.generation();
        candidate.registry.refresh(
            expected,
            &mut HookPort {
                tokens: &candidate.tokens,
                oauth: &mut self.oauth,
                fallback: &mut crate::extension_host::DenyBridge,
                stream: &mut RefreshRoot { host, control },
                control,
            },
        )?;
        let answer = self.publish(candidate, expected, control)?;
        let reply = host.call(
            &self.scope,
            "models.update",
            &answer,
            control,
            &mut crate::extension_host::DenyBridge,
        );
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                // Native publication has succeeded. A helper that did not accept
                // that view cannot keep serving its older model snapshot.
                let _ = host.stop();
                return Err(error);
            }
        };
        if reply.as_ref().map(|v| &v["generation"]) != Some(&answer["generation"]) {
            host.stop()?;
            return Err(DomainError::Unavailable(
                "extension model acknowledgement".into(),
            ));
        }
        Ok(())
    }
    fn change(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        control.check()?;
        if request.scope != &self.scope
            || !request.operation.starts_with(&self.operation_prefix)
            || !matches!(
                request.phase,
                "bind" | "event" | "command" | "shortcut" | "tool.execute" | "provider"
            )
        {
            return Err(DomainError::PermissionDenied);
        }
        packages::validate_settings_document(request.arguments)?;
        let provider = request.arguments["name"]
            .as_str()
            .ok_or(DomainError::PermissionDenied)?;
        if !self
            .grants
            .get(request.extension)
            .is_some_and(|names| names.contains(provider))
        {
            return Err(DomainError::PermissionDenied);
        }
        let mut candidate = self
            .catalog
            .state
            .try_borrow()
            .map_err(|_| DomainError::Conflict)?
            .clone();
        let expected = candidate.registry.generation();
        if request.method == "provider.register" {
            let token = request.arguments["registration"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or(DomainError::PermissionDenied)?;
            if !token.strip_prefix("provider-hook-").is_some_and(|suffix| {
                suffix
                    .parse::<u64>()
                    .is_ok_and(|value| value > 0 && value <= 9_007_199_254_740_991)
            }) {
                return Err(DomainError::PermissionDenied);
            }
            if candidate.tokens.values().any(|known| known.token == token) {
                return Err(DomainError::Conflict);
            }
            let registration = expected.checked_add(1).ok_or(DomainError::Conflict)?;
            candidate.tokens.insert(
                registration,
                Token {
                    binding: HookBinding {
                        provider: provider.into(),
                        extension: request.extension.into(),
                        registration,
                    },
                    token: token.into(),
                    oauth_hooks: ["login", "refreshToken", "getApiKey", "modifyModels"]
                        .into_iter()
                        .filter(|hook| request.arguments["config"]["hooks"]["oauth"][*hook] == true)
                        .map(str::to_owned)
                        .collect(),
                },
            );
            candidate.registry.register(
                expected,
                request.extension,
                provider,
                request.arguments["config"].clone(),
                &mut HookPort {
                    tokens: &candidate.tokens,
                    oauth: &mut self.oauth,
                    fallback: &mut self.fallback,
                    stream,
                    control,
                },
            )?;
        } else {
            candidate.registry.unregister(
                expected,
                provider,
                &mut HookPort {
                    tokens: &candidate.tokens,
                    oauth: &mut self.oauth,
                    fallback: &mut self.fallback,
                    stream,
                    control,
                },
            )?;
        }
        self.publish(candidate, expected, control).map(Some)
    }
    fn publish(
        &self,
        mut candidate: State,
        expected: u64,
        control: &ScanControl<'_>,
    ) -> Result<Value, DomainError> {
        let snapshot = candidate.registry.snapshot();
        let retained = candidate
            .registry
            .registrations()
            .iter()
            .map(|r| r.binding.registration)
            .chain(snapshot.stream_bindings.values().map(|b| b.registration))
            .chain(snapshot.oauth_bindings.values().map(|b| b.registration))
            .collect::<BTreeSet<_>>();
        candidate.tokens.retain(|id, _| retained.contains(id));
        if candidate.tokens.len() > 256 {
            return Err(DomainError::LimitExceeded(
                "extension provider hook versions".into(),
            ));
        }
        let answer = json!({"generation":candidate.registry.generation(),"models":public_models(snapshot)?,"retainRegistrations":candidate.tokens.values().map(|r|&r.token).collect::<Vec<_>>()});
        packages::validate_settings_document(&answer)?;
        if answer.to_string().len() > 900 * 1024 {
            return Err(DomainError::LimitExceeded(
                "extension provider acknowledgement".into(),
            ));
        }
        control.check()?;
        let mut state = self
            .catalog
            .state
            .try_borrow_mut()
            .map_err(|_| DomainError::Conflict)?;
        if state.registry.generation() != expected {
            return Err(DomainError::Conflict);
        }
        *state = candidate;
        Ok(answer)
    }
}
struct RefreshRoot<'a, 'b> {
    host: &'a mut ExtensionHost,
    control: &'a ScanControl<'b>,
}
impl NativeStream for RefreshRoot<'_, '_> {
    fn emit(&mut self, _: &Value) -> Result<(), DomainError> {
        Err(DomainError::PermissionDenied)
    }
    fn cancelled(&mut self) -> Result<bool, DomainError> {
        self.control.check()?;
        Ok(!self.host.is_alive())
    }
    fn invoke_provider(
        &mut self,
        invocation: ProviderInvocation<'_>,
        bridge: &mut dyn Bridge,
    ) -> Result<Option<Value>, DomainError> {
        if invocation.hook != "oauth.modifyModels" {
            return Err(DomainError::PermissionDenied);
        }
        let control = ScanControl {
            cancel: self.control.cancel,
            deadline: invocation
                .deadline
                .unwrap_or(self.control.deadline)
                .min(self.control.deadline),
        };
        let scope = self.host.scope().clone();
        self.host.call(
            &scope,
            "provider",
            &json!({
                "name":invocation.name, "registration":invocation.registration,
                "hook":invocation.hook, "arguments":invocation.arguments,
            }),
            &control,
            bridge,
        )
    }
}
impl<O: OAuth, B: Bridge> Bridge for RegistryBridge<O, B> {
    fn request(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        if matches!(request.method, "provider.register" | "provider.unregister") {
            Err(DomainError::PermissionDenied)
        } else {
            self.fallback.request(request, control)
        }
    }
    fn request_stream(
        &mut self,
        request: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        if matches!(request.method, "provider.register" | "provider.unregister") {
            self.change(request, control, stream)
        } else {
            self.fallback.request_stream(request, control, stream)
        }
    }
    fn event(&mut self, scope: &Scope, operation: &str, event: &Value) -> Result<(), DomainError> {
        self.fallback.event(scope, operation, event)
    }
}
