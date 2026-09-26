//! Concrete native model resolver for optional extension inference. Configuration,
//! credential lookup and option grants are supplied by the composition owner.
use crate::{
    extension_host::NativeStream,
    extension_inference::{InferenceInput, ModelResolver, ResolvedGeneration},
    provider::{
        generation::{GenerationLimits, GenerationRequest},
        http::{HostHeaders, endpoint_origin},
        request_http::{HttpCredential, generation_http_request_with_headers},
        requests::RequestContext,
        signing::AwsCredentials,
    },
    resource_fs::ScanControl,
};
use ag1_core::{Clock, DomainError, SecretBytes, provider::ApiFamily, tools::registry::Scope};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
/// Never serialized or returned to JavaScript.
pub enum Credential {
    ApiKey(SecretBytes),
    Bearer(SecretBytes),
    GeminiCli(SecretBytes),
    Aws {
        credentials: AwsCredentials,
        region: String,
        date: String,
    },
}
pub trait Credentials {
    fn resolve_with_stream(
        &mut self,
        model: &Value,
        _input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        _stream: &mut dyn NativeStream,
    ) -> Result<Credential, DomainError> {
        self.resolve(model, control)
    }
    /// Host vault/OAuth integration may refresh the selected credential. It must
    /// observe control and must not read arbitrary paths from extension arguments.
    fn resolve(
        &mut self,
        model: &Value,
        control: &ScanControl<'_>,
    ) -> Result<Credential, DomainError>;
}
pub struct ModelConfig {
    pub model: Value,
    pub endpoint: String,
    pub options: Value,
    pub allowed_options: BTreeSet<String>,
    pub headers: Option<HostHeaders>,
    pub limits: GenerationLimits,
    pub max_response_bytes: usize,
}
pub struct ConfiguredModels<C> {
    scope: Scope,
    extensions: BTreeSet<String>,
    models: BTreeMap<(String, String), ModelConfig>,
    credentials: C,
    clock: Arc<dyn Clock>,
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
impl<C: Credentials> ConfiguredModels<C> {
    pub fn new(
        scope: Scope,
        extensions: BTreeSet<String>,
        models: Vec<ModelConfig>,
        credentials: C,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, DomainError> {
        if extensions.len() > 64 || models.len() > 256 {
            return Err(DomainError::LimitExceeded(
                "extension model configuration".into(),
            ));
        }
        if extensions
            .iter()
            .any(|p| p.len() > 8192 || !std::path::Path::new(p).is_absolute())
        {
            return Err(invalid("extension model owner"));
        }
        let mut indexed = BTreeMap::new();
        for config in models {
            ag1_core::resources::packages::validate_settings_document(&config.model)?;
            ag1_core::resources::packages::validate_settings_document(&config.options)?;
            if config.model.to_string().len() > 65536
                || config.options.to_string().len() > 16384
                || !config.options.is_object()
                || config.allowed_options.len() > 64
            {
                return Err(DomainError::LimitExceeded(
                    "extension model configuration".into(),
                ));
            }
            for key in ["apiKey", "headers", "credentials"] {
                if config.model.get(key).is_some() {
                    return Err(invalid("credentials belong in the native credential port"));
                }
            }
            let provider = config.model["provider"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or_else(|| invalid("configured provider"))?;
            let model = config.model["id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or_else(|| invalid("configured model"))?;
            let _: ApiFamily = serde_json::from_value(config.model["api"].clone())
                .map_err(|_| invalid("configured API family"))?;
            endpoint_origin(
                config.model["baseUrl"]
                    .as_str()
                    .ok_or_else(|| invalid("configured model URL"))?,
            )?;
            if config.endpoint.is_empty()
                || config.endpoint.len() > 2048
                || config.endpoint.chars().any(char::is_control)
            {
                return Err(invalid("configured scheduler endpoint"));
            }
            if config.allowed_options.iter().any(|k| {
                k.len() > 128
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
                    .contains(&k.as_str())
            }) {
                return Err(invalid("extension option grant"));
            }
            if !(1024..=512 * 1024).contains(&config.limits.max_output_bytes)
                || config.limits.overall_ms == 0
                || config.limits.overall_ms > 86_400_000
                || config.limits.prefill_ms > 86_400_000
                || config.limits.idle_ms > 86_400_000
                || !(1024..=32 * 1024 * 1024).contains(&config.max_response_bytes)
            {
                return Err(invalid("extension generation limits"));
            }
            let key = (provider.into(), model.into());
            if indexed.insert(key, config).is_some() {
                return Err(DomainError::Conflict);
            }
        }
        Ok(Self {
            scope,
            extensions,
            models: indexed,
            credentials,
            clock,
        })
    }
}
impl<C: Credentials> ConfiguredModels<C> {
    fn resolve_inner(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: Option<&mut dyn NativeStream>,
    ) -> Result<ResolvedGeneration, DomainError> {
        control.check()?;
        if input.scope != &self.scope || !self.extensions.contains(input.extension) {
            return Err(DomainError::PermissionDenied);
        }
        let config = self
            .models
            .get(&(input.provider.into(), input.model.into()))
            .ok_or(DomainError::PermissionDenied)?;
        let options = input
            .options
            .as_object()
            .ok_or_else(|| invalid("extension model options"))?;
        if options.keys().any(|k| !config.allowed_options.contains(k)) {
            return Err(DomainError::PermissionDenied);
        }
        let mut merged = config.options.clone();
        for (key, value) in options {
            merged[key] = value.clone();
        }
        let tools = input
            .context
            .get("tools")
            .map(|v| {
                v.as_array()
                    .ok_or_else(|| invalid("extension inference tools"))
            })
            .transpose()?;
        let tool_names = tools
            .map(|v| {
                v.iter()
                    .map(|tool| {
                        tool["name"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 128)
                            .map(str::to_owned)
                            .ok_or_else(|| invalid("extension inference tool name"))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        if tool_names.len() > 256 {
            return Err(DomainError::LimitExceeded(
                "extension inference tools".into(),
            ));
        }
        let credential = match stream {
            Some(stream) => {
                self.credentials
                    .resolve_with_stream(&config.model, input, control, stream)?
            }
            None => self.credentials.resolve(&config.model, control)?,
        };
        let credential = match &credential {
            Credential::ApiKey(key) => HttpCredential::ApiKey(key),
            Credential::Bearer(key) => HttpCredential::Bearer(key),
            Credential::GeminiCli(key) => HttpCredential::GeminiCli(key),
            Credential::Aws {
                credentials,
                region,
                date,
            } => HttpCredential::Aws {
                credentials,
                region,
                date,
            },
        };
        let request = generation_http_request_with_headers(
            &RequestContext {
                model: &config.model,
                context: input.context,
                options: &merged,
                anthropic_oauth: config.model["provider"] == "anthropic"
                    && matches!(&credential, HttpCredential::Bearer(_)),
                google_project: None,
                request_id: input.operation,
            },
            credential,
            config.headers.as_ref(),
            config.max_response_bytes,
        )?;
        let family = serde_json::from_value(config.model["api"].clone())
            .map_err(|_| invalid("configured API family"))?;
        Ok(ResolvedGeneration {
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            timestamp_ms: self.clock.now_ms(),
            generation: GenerationRequest {
                request_id: input.operation.into(),
                tool_names,
                family,
                request,
                limits: GenerationLimits {
                    max_output_bytes: config.limits.max_output_bytes,
                    prefill_ms: config.limits.prefill_ms,
                    idle_ms: config.limits.idle_ms,
                    overall_ms: config.limits.overall_ms,
                },
            },
        })
    }
}
impl<C: Credentials> ModelResolver for ConfiguredModels<C> {
    fn resolve(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
    ) -> Result<ResolvedGeneration, DomainError> {
        self.resolve_inner(input, control, None)
    }
    fn resolve_with_stream(
        &mut self,
        input: InferenceInput<'_>,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
    ) -> Result<ResolvedGeneration, DomainError> {
        self.resolve_inner(input, control, Some(stream))
    }
}
