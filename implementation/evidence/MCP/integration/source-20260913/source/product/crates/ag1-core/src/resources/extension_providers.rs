//! Dynamic extension provider catalog. Registration is data, not a network,
//! credential, inference or parent-state capability. The host binds hook versions
//! to the admitted helper incarnation and performs all scheduled execution.
use crate::{DomainError, tools::schema::bounded};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HookBinding {
    pub provider: String,
    pub extension: String,
    pub registration: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub models: Vec<Value>,
    pub request_configs: BTreeMap<String, Value>,
    pub model_headers: BTreeMap<String, Value>,
    #[serde(default)]
    pub stream_bindings: BTreeMap<String, HookBinding>,
    #[serde(default)]
    pub oauth_bindings: BTreeMap<String, HookBinding>,
}
#[derive(Clone)]
pub struct Registration {
    pub binding: HookBinding,
    /// Native-only request configuration; do not return API keys/headers in a
    /// renderer or extension model catalog.
    pub config: Value,
}
pub trait Hooks {
    /// Return None when native credentials are absent. A supplied result is the
    /// provider's complete modified model list, validated before publication.
    fn modify_models(
        &mut self,
        binding: &HookBinding,
        models: &[Value],
    ) -> Result<Option<Vec<Value>>, DomainError>;
}
#[derive(Clone)]
pub struct Registry {
    baseline: Snapshot,
    snapshot: Snapshot,
    registrations: Vec<Registration>,
    generation: u64,
}
fn invalid(message: impl Into<String>) -> DomainError {
    DomainError::InvalidInput(message.into())
}
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(v)) => *v,
        Some(Value::String(v)) => !v.is_empty(),
        Some(Value::Number(v)) => v.as_f64() != Some(0.0),
        _ => true,
    }
}
fn name(value: &str) -> Result<(), DomainError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(invalid("extension provider name"))
    } else {
        Ok(())
    }
}
fn validate(snapshot: &Snapshot) -> Result<(), DomainError> {
    if snapshot.models.len() > 4096
        || snapshot.request_configs.len() > 256
        || snapshot.model_headers.len() > 4096
        || snapshot.stream_bindings.len() > 256
        || snapshot.oauth_bindings.len() > 256
    {
        return Err(DomainError::LimitExceeded(
            "extension provider catalog".into(),
        ));
    }
    bounded(
        &serde_json::to_value(snapshot).map_err(|_| invalid("provider catalog JSON"))?,
        16 * 1024 * 1024,
        64,
    )
}
fn validate_config(provider: &str, config: &Value) -> Result<(), DomainError> {
    name(provider)?;
    bounded(config, 1024 * 1024, 64)?;
    if !config.is_object() {
        return Err(invalid("extension provider configuration"));
    }
    if config["hooks"]["streamSimple"] == true && !truthy(config.get("api")) {
        return Err(invalid(format!(
            "Provider {provider}: \"api\" is required when registering streamSimple."
        )));
    }
    let models = config
        .get("models")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_array()
                .ok_or_else(|| invalid("extension provider models"))
        })
        .transpose()?;
    let Some(models) = models.filter(|v| !v.is_empty()) else {
        return Ok(());
    };
    if models.len() > 4096 {
        return Err(DomainError::LimitExceeded(
            "extension provider models".into(),
        ));
    }
    if !truthy(config.get("baseUrl")) {
        return Err(invalid(format!(
            "Provider {provider}: \"baseUrl\" is required when defining models."
        )));
    }
    if !truthy(config.get("apiKey")) && !truthy(config.get("oauth")) {
        return Err(invalid(format!(
            "Provider {provider}: \"apiKey\" or \"oauth\" is required when defining models."
        )));
    }
    for model in models {
        if !model.is_object() {
            return Err(invalid("extension provider model"));
        }
        if !truthy(model.get("api")) && !truthy(config.get("api")) {
            return Err(invalid(format!(
                "Provider {provider}, model {}: no \"api\" specified.",
                model["id"].as_str().unwrap_or("undefined")
            )));
        }
    }
    Ok(())
}
fn apply(
    snapshot: &mut Snapshot,
    registration: &Registration,
    hooks: &mut impl Hooks,
) -> Result<(), DomainError> {
    let config = &registration.config;
    let provider = &registration.binding.provider;
    if truthy(config.get("oauth")) {
        snapshot
            .oauth_bindings
            .insert(provider.clone(), registration.binding.clone());
    }
    if config["hooks"]["streamSimple"] == true {
        let api = config["api"]
            .as_str()
            .ok_or_else(|| invalid("extension provider API"))?;
        name(api)?;
        snapshot
            .stream_bindings
            .insert(api.into(), registration.binding.clone());
    }
    if ["apiKey", "headers", "authHeader"]
        .iter()
        .any(|key| truthy(config.get(*key)))
    {
        let mut request = json!({});
        for key in ["apiKey", "headers", "authHeader"] {
            if let Some(value) = config.get(key) {
                request[key] = value.clone();
            }
        }
        snapshot.request_configs.insert(provider.clone(), request);
    }
    if let Some(models) = config["models"].as_array().filter(|v| !v.is_empty()) {
        snapshot
            .models
            .retain(|model| model["provider"] != *provider);
        for definition in models {
            let id = definition["id"]
                .as_str()
                .ok_or_else(|| invalid("extension model id"))?;
            name(id)?;
            let key = format!("{provider}:{id}");
            match definition
                .get("headers")
                .filter(|v| v.as_object().is_some_and(|m| !m.is_empty()))
            {
                Some(headers) => {
                    snapshot.model_headers.insert(key, headers.clone());
                }
                None => {
                    snapshot.model_headers.remove(&key);
                }
            }
            let mut model = json!({"provider":provider,"baseUrl":config["baseUrl"],"api": if truthy(definition.get("api")) { &definition["api"] } else { &config["api"] }});
            for key in [
                "id",
                "name",
                "reasoning",
                "thinking",
                "input",
                "cost",
                "contextWindow",
                "maxTokens",
                "compat",
            ] {
                if let Some(value) = definition.get(key) {
                    model[key] = value.clone();
                }
            }
            snapshot.models.push(model);
        }
        if config["hooks"]["oauth"]["modifyModels"] == true
            && let Some(models) = hooks.modify_models(&registration.binding, &snapshot.models)?
        {
            snapshot.models = models;
        }
    } else if truthy(config.get("baseUrl")) || truthy(config.get("headers")) {
        for model in &mut snapshot.models {
            if model["provider"] == *provider
                && let Some(base) = config.get("baseUrl").filter(|v| !v.is_null())
            {
                model["baseUrl"] = base.clone();
            }
        }
    }
    validate(snapshot)
}
impl Registry {
    pub fn new(baseline: Snapshot) -> Result<Self, DomainError> {
        validate(&baseline)?;
        Ok(Self {
            snapshot: baseline.clone(),
            baseline,
            registrations: Vec::new(),
            generation: 0,
        })
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
    pub fn registrations(&self) -> &[Registration] {
        &self.registrations
    }
    /// Rebuild from the admitted baseline after credential changes. Hook failure
    /// leaves the prior catalog and generation intact; bindings retain identity.
    pub fn refresh(&mut self, expected: u64, hooks: &mut impl Hooks) -> Result<(), DomainError> {
        if expected != self.generation {
            return Err(DomainError::Conflict);
        }
        let generation = expected.checked_add(1).ok_or(DomainError::Conflict)?;
        let mut snapshot = self.baseline.clone();
        for registration in &self.registrations {
            apply(&mut snapshot, registration, hooks)?;
        }
        self.snapshot = snapshot;
        self.generation = generation;
        Ok(())
    }
    pub fn register(
        &mut self,
        expected: u64,
        extension: &str,
        provider: &str,
        config: Value,
        hooks: &mut impl Hooks,
    ) -> Result<HookBinding, DomainError> {
        if expected != self.generation {
            return Err(DomainError::Conflict);
        }
        if extension.is_empty() || extension.len() > 8192 || extension.contains('\0') {
            return Err(invalid("extension provider owner"));
        }
        validate_config(provider, &config)?;
        let retained = self
            .registrations
            .iter()
            .filter(|r| r.binding.provider != provider)
            .map(|r| r.config.to_string().len())
            .sum::<usize>();
        if retained + config.to_string().len() > 16 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "extension provider configuration retention".into(),
            ));
        }
        if !self
            .registrations
            .iter()
            .any(|r| r.binding.provider == provider)
            && self.registrations.len() >= 256
        {
            return Err(DomainError::LimitExceeded(
                "extension provider registrations".into(),
            ));
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let registration = Registration {
            binding: HookBinding {
                provider: provider.into(),
                extension: extension.into(),
                registration: generation,
            },
            config,
        };
        let mut snapshot = self.snapshot.clone();
        apply(&mut snapshot, &registration, hooks)?;
        if let Some(existing) = self
            .registrations
            .iter_mut()
            .find(|r| r.binding.provider == provider)
        {
            *existing = registration.clone();
        } else {
            self.registrations.push(registration.clone());
        }
        self.snapshot = snapshot;
        self.generation = generation;
        Ok(registration.binding)
    }
    pub fn unregister(
        &mut self,
        expected: u64,
        provider: &str,
        hooks: &mut impl Hooks,
    ) -> Result<(), DomainError> {
        if expected != self.generation {
            return Err(DomainError::Conflict);
        }
        name(provider)?;
        if !self
            .registrations
            .iter()
            .any(|r| r.binding.provider == provider)
        {
            return Ok(());
        }
        let mut snapshot = self.baseline.clone();
        for registration in self
            .registrations
            .iter()
            .filter(|r| r.binding.provider != provider)
        {
            apply(&mut snapshot, registration, hooks)?;
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        self.registrations
            .retain(|r| r.binding.provider != provider);
        self.snapshot = snapshot;
        self.generation = generation;
        Ok(())
    }
}
