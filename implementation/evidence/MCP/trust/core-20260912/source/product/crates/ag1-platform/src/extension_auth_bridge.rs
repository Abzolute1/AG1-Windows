//! Separate native grants for key disclosure and private model-hook credentials.
use crate::{
    extension_auth::Service,
    extension_custom_models::Credentials,
    extension_host::{Bridge, DenyBridge, ExtensionHost, NativeRequest, NativeStream},
    extension_inference::InferenceInput,
    extension_models::{self, Credential},
    extension_providers::Catalog,
    provider::auth::CredentialBackend,
    resource_fs::ScanControl,
};
use ag1_core::{DomainError, resources::packages, tools::registry::Scope};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub struct KeyGrant {
    pub provider: String,
    pub readers: BTreeSet<String>,
    pub model_ids: BTreeSet<String>,
    pub provider_lookup: bool,
}

#[derive(Clone, Copy)]
pub enum HttpKeyKind {
    ApiKey,
    Bearer,
    GeminiCli,
}
pub struct HttpGrant {
    pub provider: String,
    pub model_ids: BTreeSet<String>,
    pub origins: BTreeSet<String>,
    pub kind: HttpKeyKind,
}
/// OAuth keys used by native HTTP remain in native secret-bearing request types.
/// These endpoint grants do not authorize an API-key response to JavaScript.
pub struct HttpCredentials<B, F> {
    service: Arc<Service<B>>,
    catalog: Catalog,
    grants: BTreeMap<String, HttpGrant>,
    fallback: F,
}
impl<B: CredentialBackend, F: extension_models::Credentials> HttpCredentials<B, F> {
    pub fn new(
        catalog: Catalog,
        service: Arc<Service<B>>,
        grants: Vec<HttpGrant>,
        fallback: F,
    ) -> Result<Self, DomainError> {
        if grants.len() > 128 {
            return Err(DomainError::LimitExceeded(
                "native HTTP credential grants".into(),
            ));
        }
        let mut indexed = BTreeMap::new();
        for grant in grants {
            if !valid_name(&grant.provider)
                || grant.model_ids.len() > 256
                || grant.origins.len() > 64
                || grant.model_ids.iter().any(|id| !valid_name(id))
            {
                return Err(DomainError::PermissionDenied);
            }
            for origin in &grant.origins {
                if crate::provider::http::endpoint_origin(origin)? != *origin {
                    return Err(DomainError::InvalidInput(
                        "native credential origin grant".into(),
                    ));
                }
            }
            if indexed.insert(grant.provider.clone(), grant).is_some() {
                return Err(DomainError::Conflict);
            }
        }
        Ok(Self {
            service,
            catalog,
            grants: indexed,
            fallback,
        })
    }
}
impl<B: CredentialBackend, F: extension_models::Credentials> extension_models::Credentials
    for HttpCredentials<B, F>
{
    fn resolve(&mut self, _: &Value, _: &ScanControl<'_>) -> Result<Credential, DomainError> {
        Err(DomainError::Unavailable(
            "extension OAuth credentials require the native hook transport".into(),
        ))
    }
    fn resolve_with_stream(
        &mut self,
        model: &Value,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Credential, DomainError> {
        control.check()?;
        if model["provider"] != input.provider || model["id"] != input.model {
            return Err(DomainError::PermissionDenied);
        }
        let Some(target) =
            self.catalog
                .oauth_target(input.scope, input.host_operation, input.provider)?
        else {
            return self
                .fallback
                .resolve_with_stream(model, input, control, stream);
        };
        let origin = crate::provider::http::endpoint_origin(
            model["baseUrl"]
                .as_str()
                .ok_or(DomainError::PermissionDenied)?,
        )?;
        let grant = self
            .grants
            .get(input.provider)
            .filter(|g| g.model_ids.contains(input.model) && g.origins.contains(&origin))
            .ok_or(DomainError::PermissionDenied)?;
        if !self
            .catalog
            .models()?
            .iter()
            .any(|current| current == model)
        {
            return Err(DomainError::Conflict);
        }
        let Some(key) = self
            .service
            .key(&target, false, control, stream, &mut DenyBridge)?
        else {
            return self
                .fallback
                .resolve_with_stream(model, input, control, stream);
        };
        target.validate()?;
        Ok(match grant.kind {
            HttpKeyKind::ApiKey => Credential::ApiKey(key),
            HttpKeyKind::Bearer => Credential::Bearer(key),
            HttpKeyKind::GeminiCli => Credential::GeminiCli(key),
        })
    }
}
pub struct KeyBridge<B, F> {
    service: Arc<Service<B>>,
    catalog: Catalog,
    scope: Scope,
    host_prefix: String,
    grants: BTreeMap<String, KeyGrant>,
    fallback: F,
}
fn key_method(method: &str) -> bool {
    matches!(method, "auth.getApiKey" | "auth.getApiKeyForProvider")
}
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}
impl<B: CredentialBackend, F: Bridge> KeyBridge<B, F> {
    pub fn new(
        host: &ExtensionHost,
        catalog: Catalog,
        service: Arc<Service<B>>,
        grants: Vec<KeyGrant>,
        fallback: F,
    ) -> Result<Self, DomainError> {
        if grants.len() > 128 {
            return Err(DomainError::LimitExceeded(
                "extension key disclosure grants".into(),
            ));
        }
        let host_prefix = format!("extension-{}-", host.identity());
        let mut indexed = BTreeMap::new();
        for grant in grants {
            if !valid_name(&grant.provider)
                || grant.readers.len() > 64
                || grant.model_ids.len() > 256
                || grant
                    .readers
                    .iter()
                    .any(|reader| !host.admits_extension(reader))
                || grant.model_ids.iter().any(|id| !valid_name(id))
            {
                return Err(DomainError::PermissionDenied);
            }
            // Verify the catalog's scope/incarnation, including before providers bind.
            catalog.oauth_target(host.scope(), &host_prefix, &grant.provider)?;
            if indexed.insert(grant.provider.clone(), grant).is_some() {
                return Err(DomainError::Conflict);
            }
        }
        Ok(Self {
            service,
            catalog,
            scope: host.scope().clone(),
            host_prefix,
            grants: indexed,
            fallback,
        })
    }
    fn key(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        control.check()?;
        if r.scope != &self.scope
            || !r.operation.starts_with(&self.host_prefix)
            || !matches!(
                r.phase,
                "event" | "command" | "shortcut" | "tool.execute" | "provider"
            )
        {
            return Err(DomainError::PermissionDenied);
        }
        packages::validate_settings_document(r.arguments)?;
        if r.arguments.to_string().len() > 65536 {
            return Err(DomainError::LimitExceeded("extension key selector".into()));
        }
        let provider = if r.method == "auth.getApiKey" {
            r.arguments["model"]["provider"].as_str()
        } else {
            r.arguments["provider"].as_str()
        }
        .ok_or(DomainError::PermissionDenied)?;
        let grant = self
            .grants
            .get(provider)
            .filter(|grant| grant.readers.contains(r.extension))
            .ok_or(DomainError::PermissionDenied)?;
        if r.method == "auth.getApiKeyForProvider" {
            if !grant.provider_lookup {
                return Err(DomainError::PermissionDenied);
            }
        } else {
            let id = r.arguments["model"]["id"]
                .as_str()
                .ok_or(DomainError::PermissionDenied)?;
            if !grant.model_ids.contains(id)
                || !self
                    .catalog
                    .models()?
                    .iter()
                    .any(|m| m["provider"] == provider && m["id"] == id)
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        let Some(target) = self.catalog.oauth_target(r.scope, r.operation, provider)? else {
            // Stock native credential handling remains a separately supplied port,
            // reached only after the same disclosure grant has matched.
            return self.fallback.request_stream(r, control, stream);
        };
        // A refresh/get-key hook cannot recurse into auth while holding the vault
        // lease or acquire unrelated parent authority through a callback.
        let key = self
            .service
            .key(&target, false, control, stream, &mut DenyBridge)?;
        key.map(|key| {
            std::str::from_utf8(key.expose())
                .map(|value| json!(value))
                .map_err(|_| DomainError::InvalidInput("extension API key UTF-8".into()))
        })
        .transpose()
    }
}
impl<B: CredentialBackend, F: Bridge> Bridge for KeyBridge<B, F> {
    fn request(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        if key_method(r.method) {
            Err(DomainError::PermissionDenied)
        } else {
            self.fallback.request(r, control)
        }
    }
    fn request_stream(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<Value>, DomainError> {
        if key_method(r.method) {
            self.key(r, control, stream)
        } else {
            self.fallback.request_stream(r, control, stream)
        }
    }
    fn event(&mut self, scope: &Scope, operation: &str, event: &Value) -> Result<(), DomainError> {
        self.fallback.event(scope, operation, event)
    }
}

pub struct RecipientGrant {
    pub provider: String,
    pub hook_owners: BTreeSet<String>,
}
/// Supplies keys only to independently admitted custom-stream recipients. It does
/// not grant the calling extension permission to receive an API-key response.
pub struct ModelCredentials<B, F> {
    service: Arc<Service<B>>,
    catalog: Catalog,
    recipients: BTreeMap<String, BTreeSet<String>>,
    fallback: F,
}
impl<B: CredentialBackend, F: Credentials> ModelCredentials<B, F> {
    pub fn new(
        catalog: Catalog,
        service: Arc<Service<B>>,
        grants: Vec<RecipientGrant>,
        fallback: F,
    ) -> Result<Self, DomainError> {
        if grants.len() > 128 {
            return Err(DomainError::LimitExceeded(
                "extension credential recipient grants".into(),
            ));
        }
        let mut recipients = BTreeMap::new();
        for grant in grants {
            if !valid_name(&grant.provider)
                || grant.hook_owners.len() > 64
                || grant
                    .hook_owners
                    .iter()
                    .any(|p| p.len() > 8192 || !std::path::Path::new(p).is_absolute())
                || recipients
                    .insert(grant.provider, grant.hook_owners)
                    .is_some()
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        Ok(Self {
            service,
            catalog,
            recipients,
            fallback,
        })
    }
}
impl<B: CredentialBackend, F: Credentials> Credentials for ModelCredentials<B, F> {
    fn options(
        &mut self,
        _: &str,
        _: &Value,
        _: Value,
        _: &ScanControl<'_>,
    ) -> Result<Value, DomainError> {
        Err(DomainError::Unavailable(
            "extension OAuth credentials require the native hook transport".into(),
        ))
    }
    fn options_with_stream(
        &mut self,
        hook_owner: &str,
        model: &Value,
        mut options: Value,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Value, DomainError> {
        control.check()?;
        if model["provider"] != input.provider || model["id"] != input.model {
            return Err(DomainError::PermissionDenied);
        }
        let Some(target) =
            self.catalog
                .oauth_target(input.scope, input.host_operation, input.provider)?
        else {
            return self
                .fallback
                .options_with_stream(hook_owner, model, options, input, control, stream);
        };
        if !self
            .recipients
            .get(input.provider)
            .is_some_and(|owners| owners.contains(hook_owner))
        {
            return Err(DomainError::PermissionDenied);
        }
        if let Some(key) = self
            .service
            .key(&target, false, control, stream, &mut DenyBridge)?
        {
            if !options.is_object() {
                return Err(DomainError::InvalidInput("native model options".into()));
            }
            options["apiKey"] = json!(
                std::str::from_utf8(key.expose())
                    .map_err(|_| DomainError::InvalidInput("extension API key UTF-8".into()))?
            );
            target.validate()?;
            Ok(options)
        } else {
            self.fallback
                .options_with_stream(hook_owner, model, options, input, control, stream)
        }
    }
}
