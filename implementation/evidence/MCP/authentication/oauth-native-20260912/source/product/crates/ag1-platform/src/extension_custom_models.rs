//! Explicit native grants for dynamic provider inference. A registered provider
//! never grants itself a caller, endpoint, hook owner or credential capability.
use crate::{
    extension_host::NativeStream,
    extension_inference::{
        InferenceInput, ModelResolver, ResolvedCustomGeneration, ResolvedGeneration,
    },
    extension_providers::Catalog,
    provider::http::endpoint_origin,
    resource_fs::ScanControl,
};
use ag1_core::{Clock, DomainError, resources::packages};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub struct Config {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub callers: BTreeSet<String>,
    pub hook_owners: BTreeSet<String>,
    pub origins: BTreeSet<String>,
    pub options: Value,
    pub allowed_options: BTreeSet<String>,
    pub max_output_bytes: usize,
    pub overall_ms: u64,
}
/// Called after the native model/caller/hook/endpoint grants have matched. The
/// returned private options go only to that provider hook, not the requesting JS
/// extension. Credential selection must use the native vault/configuration.
pub trait Credentials {
    fn options_with_stream(
        &mut self,
        hook_owner: &str,
        model: &Value,
        options: Value,
        _input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        _stream: &mut dyn NativeStream,
    ) -> Result<Value, DomainError> {
        self.options(hook_owner, model, options, control)
    }
    fn options(
        &mut self,
        hook_owner: &str,
        model: &Value,
        options: Value,
        control: &ScanControl<'_>,
    ) -> Result<Value, DomainError>;
}
pub struct CustomModels<R, C> {
    catalog: Catalog,
    configs: BTreeMap<(String, String), Config>,
    fallback: R,
    credentials: C,
    clock: Arc<dyn Clock>,
}
fn invalid(message: &str) -> DomainError {
    DomainError::InvalidInput(message.into())
}
impl<R: ModelResolver, C: Credentials> CustomModels<R, C> {
    pub fn new(
        catalog: Catalog,
        configs: Vec<Config>,
        fallback: R,
        credentials: C,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, DomainError> {
        if configs.len() > 256 {
            return Err(DomainError::LimitExceeded("custom model grants".into()));
        }
        let mut indexed = BTreeMap::new();
        for config in configs {
            if config.provider.is_empty()
                || config.provider.len() > 256
                || config.model.is_empty()
                || config.model.len() > 256
                || config.endpoint.is_empty()
                || config.endpoint.len() > 2048
                || config.endpoint.chars().any(char::is_control)
                || config.callers.len() > 64
                || config.hook_owners.len() > 64
                || config.origins.len() > 32
                || config.allowed_options.len() > 64
                || !(1024..=512 * 1024).contains(&config.max_output_bytes)
                || config.overall_ms == 0
                || config.overall_ms > 86_400_000
            {
                return Err(invalid("custom model grant"));
            }
            for owner in config.callers.iter().chain(&config.hook_owners) {
                if !std::path::Path::new(owner).is_absolute() || owner.len() > 8192 {
                    return Err(invalid("custom model extension owner"));
                }
            }
            for origin in &config.origins {
                if endpoint_origin(origin)? != *origin {
                    return Err(invalid("custom model origin"));
                }
            }
            for key in &config.allowed_options {
                if key.len() > 128
                    || [
                        "apiKey",
                        "headers",
                        "baseUrl",
                        "fetch",
                        "signal",
                        "onPayload",
                        "onResponse",
                        "azureEndpoint",
                        "azureApiKey",
                    ]
                    .contains(&key.as_str())
                {
                    return Err(invalid("custom model option grant"));
                }
            }
            packages::validate_settings_document(&config.options)?;
            if !config.options.is_object() || config.options.to_string().len() > 16384 {
                return Err(invalid("custom model options"));
            }
            if indexed
                .insert((config.provider.clone(), config.model.clone()), config)
                .is_some()
            {
                return Err(DomainError::Conflict);
            }
        }
        Ok(Self {
            catalog,
            configs: indexed,
            fallback,
            credentials,
            clock,
        })
    }
}
impl<R: ModelResolver, C: Credentials> CustomModels<R, C> {
    fn require_native(&self, input: InferenceInput<'_>) -> Result<(), DomainError> {
        if self
            .catalog
            .custom_target(
                input.scope,
                input.host_operation,
                input.provider,
                input.model,
            )?
            .is_some()
        {
            return Err(DomainError::Unavailable(
                "custom provider requires scheduled hook execution".into(),
            ));
        }
        Ok(())
    }
    fn resolve_custom_inner(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: Option<&mut dyn NativeStream>,
    ) -> Result<Option<ResolvedCustomGeneration>, DomainError> {
        control.check()?;
        let Some(target) = self.catalog.custom_target(
            input.scope,
            input.host_operation,
            input.provider,
            input.model,
        )?
        else {
            return match stream {
                Some(stream) => self
                    .fallback
                    .resolve_custom_with_stream(input, control, stream),
                None => self.fallback.resolve_custom(input, control),
            };
        };
        let config = self
            .configs
            .get(&(input.provider.into(), input.model.into()))
            .ok_or(DomainError::PermissionDenied)?;
        if !config.callers.contains(input.extension)
            || !config.hook_owners.contains(&target.binding().extension)
            || !config.origins.contains(&endpoint_origin(
                target.model()["baseUrl"]
                    .as_str()
                    .ok_or_else(|| invalid("custom model URL"))?,
            )?)
        {
            return Err(DomainError::PermissionDenied);
        }
        let requested = input
            .options
            .as_object()
            .ok_or_else(|| invalid("custom model options"))?;
        if requested
            .keys()
            .any(|k| !config.allowed_options.contains(k))
        {
            return Err(DomainError::PermissionDenied);
        }
        let mut options = config.options.clone();
        for (key, value) in requested {
            options[key] = value.clone();
        }
        let options = match stream {
            Some(stream) => self.credentials.options_with_stream(
                &target.binding().extension,
                target.model(),
                options,
                input,
                control,
                stream,
            ),
            None => self.credentials.options(
                &target.binding().extension,
                target.model(),
                options,
                control,
            ),
        }?;
        packages::validate_settings_document(&options)?;
        if !options.is_object() || options.to_string().len() > 65536 {
            return Err(invalid("custom provider private options"));
        }
        target.validate()?;
        control.check()?;
        Ok(Some(ResolvedCustomGeneration {
            target,
            endpoint: config.endpoint.clone(),
            options,
            timestamp_ms: self.clock.now_ms(),
            max_output_bytes: config.max_output_bytes,
            overall_ms: config.overall_ms,
        }))
    }
}
impl<R: ModelResolver, C: Credentials> ModelResolver for CustomModels<R, C> {
    fn resolve_custom(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<ResolvedCustomGeneration>, DomainError> {
        self.resolve_custom_inner(input, control, None)
    }
    fn resolve_custom_with_stream(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<Option<ResolvedCustomGeneration>, DomainError> {
        self.resolve_custom_inner(input, control, Some(stream))
    }
    fn resolve_with_stream(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<ResolvedGeneration, DomainError> {
        self.require_native(input)?;
        self.fallback.resolve_with_stream(input, control, stream)
    }
    fn resolve(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
    ) -> Result<ResolvedGeneration, DomainError> {
        self.require_native(input)?;
        self.fallback.resolve(input, control)
    }
}
