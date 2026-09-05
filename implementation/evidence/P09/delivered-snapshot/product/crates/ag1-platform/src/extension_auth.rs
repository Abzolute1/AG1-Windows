//! Native credential lifecycle for explicitly admitted extension OAuth hooks.
//! Run on the blocking host actor; the backend is the shared P06/P05 vault.
use crate::{
    extension_host::{Bridge, NativeStream, ProviderInvocation},
    extension_providers::OAuthTarget,
    provider::auth::{CredentialBackend, CredentialSource, REVOKED_CREDENTIAL_TYPE},
    resource_fs::ScanControl,
};
use ag1_core::{Clock, DomainError, PortFuture, SecretBytes, resources::packages};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

pub struct Grant {
    pub provider: String,
    pub hook_owners: BTreeSet<String>,
}
#[derive(Default)]
pub struct LoginOptions {
    pub manual_code_input: bool,
}
pub struct Service<B> {
    backend: B,
    owners: BTreeMap<String, BTreeSet<String>>,
    clock: Arc<dyn Clock>,
    runtime: tokio::runtime::Runtime,
}
fn invalid() -> DomainError {
    DomainError::InvalidInput("extension OAuth credential shape".into())
}
fn parse(raw: &SecretBytes) -> Result<Value, DomainError> {
    if raw.expose().len() > 65536 {
        return Err(DomainError::LimitExceeded(
            "extension OAuth credential".into(),
        ));
    }
    let value: Value = serde_json::from_slice(raw.expose()).map_err(|_| invalid())?;
    packages::validate_settings_document(&value).map_err(|_| invalid())?;
    if !value.is_object() {
        return Err(invalid());
    }
    Ok(value)
}
fn oauth(value: &Value) -> Result<f64, DomainError> {
    let expires = value["expires"]
        .as_f64()
        .filter(|n| n.is_finite() && n.abs() <= 9_007_199_254_740_991.0)
        .ok_or_else(invalid)?;
    if value["type"] != "oauth" || !value["access"].is_string() || !value["refresh"].is_string() {
        return Err(invalid());
    }
    Ok(expires)
}
fn encoded(value: &Value) -> Result<SecretBytes, DomainError> {
    let raw = SecretBytes::new(serde_json::to_vec(value).map_err(|_| invalid())?);
    let _ = parse(&raw)?;
    Ok(raw)
}
fn hook_error(error: DomainError) -> DomainError {
    match error {
        DomainError::Cancelled
        | DomainError::TimedOut
        | DomainError::Conflict
        | DomainError::PermissionDenied => error,
        _ => DomainError::Unavailable("extension OAuth hook failed".into()),
    }
}
impl<B: CredentialBackend> Service<B> {
    pub fn new(backend: B, grants: Vec<Grant>, clock: Arc<dyn Clock>) -> Result<Self, DomainError> {
        if grants.len() > 128 {
            return Err(DomainError::LimitExceeded("extension OAuth grants".into()));
        }
        let mut owners = BTreeMap::new();
        for grant in grants {
            if grant.provider.is_empty()
                || grant.provider.len() > 128
                || grant.provider.chars().any(char::is_control)
                || grant.hook_owners.len() > 64
                || grant
                    .hook_owners
                    .iter()
                    .any(|p| p.len() > 8192 || !std::path::Path::new(p).is_absolute())
                || owners.insert(grant.provider, grant.hook_owners).is_some()
            {
                return Err(DomainError::PermissionDenied);
            }
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| DomainError::Unavailable("extension OAuth executor".into()))?;
        Ok(Self {
            backend,
            owners,
            clock,
            runtime,
        })
    }
    fn admit(&self, target: &OAuthTarget, control: &ScanControl<'_>) -> Result<(), DomainError> {
        control.check()?;
        target.validate()?;
        if !self
            .owners
            .get(&target.binding().provider)
            .is_some_and(|owners| owners.contains(&target.binding().extension))
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    fn wait<T>(
        &self,
        future: PortFuture<'_, T>,
        control: &ScanControl<'_>,
    ) -> Result<T, DomainError> {
        control.check()?;
        self.runtime.block_on(async {
            let mut future = future;
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            loop {
                tokio::select! {
                    result=&mut future => return result,
                    _=tick.tick()=>control.check()?,
                }
            }
        })
    }
    fn invoke(
        &self,
        target: &OAuthTarget,
        hook: &str,
        args: &Value,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
        callbacks: &mut dyn Bridge,
    ) -> Result<Value, DomainError> {
        self.admit(target, control)?;
        if !target.supports(hook) {
            return Err(DomainError::Unavailable(
                "extension OAuth hook is unavailable".into(),
            ));
        }
        if stream.cancelled()? {
            return Err(DomainError::Cancelled);
        }
        let value = stream
            .invoke_provider(
                ProviderInvocation {
                    deadline: Some(control.deadline),
                    name: &target.binding().provider,
                    registration: target.token(),
                    hook: &format!("oauth.{hook}"),
                    arguments: args,
                },
                callbacks,
            )
            .map_err(hook_error)?
            .ok_or_else(invalid)?;
        self.admit(target, control)?;
        if stream.cancelled()? {
            return Err(DomainError::Cancelled);
        }
        packages::validate_settings_document(&value).map_err(|_| invalid())?;
        if value.to_string().len() > 65536 {
            return Err(DomainError::LimitExceeded(
                "extension OAuth hook result".into(),
            ));
        }
        Ok(value)
    }
    /// Login is an explicit native action. Interactive hook callbacks are routed
    /// through the caller's narrowly granted native UI bridge.
    pub fn login(
        &self,
        target: &OAuthTarget,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
        callbacks: &mut dyn Bridge,
    ) -> Result<(), DomainError> {
        self.login_with_options(target, &LoginOptions::default(), control, stream, callbacks)
    }
    pub fn login_with_options(
        &self,
        target: &OAuthTarget,
        options: &LoginOptions,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
        callbacks: &mut dyn Bridge,
    ) -> Result<(), DomainError> {
        self.admit(target, control)?;
        // Do not hold the vault lock while the user completes interactive login.
        // Capture a revision first so a later login cannot overwrite a newer one.
        let before = self.wait(self.backend.load(&target.binding().provider), control)?;
        let mut value = self.invoke(
            target,
            "login",
            &json!({"manualCodeInput":options.manual_code_input}),
            control,
            stream,
            callbacks,
        )?;
        if !value.is_object() {
            return Err(invalid());
        }
        value["type"] = json!("oauth");
        oauth(&value)?;
        let raw = encoded(&value)?;
        let _lease = self.wait(self.backend.acquire(&target.binding().provider), control)?;
        let current = self.wait(self.backend.load(&target.binding().provider), control)?;
        if current.as_ref().map(|r| r.revision) != before.as_ref().map(|r| r.revision) {
            return Err(DomainError::Conflict);
        }
        self.admit(target, control)?;
        // Once commit starts, await its durable acknowledgement. Cancelling a
        // queued storage future would make credential publication uncertain.
        match current {
            Some(record) => self.runtime.block_on(self.backend.compare_exchange(
                &target.binding().provider,
                record.revision,
                raw,
            )),
            None => self
                .runtime
                .block_on(self.backend.create(&target.binding().provider, raw)),
        }
    }
    /// Native user action. Revocation retains a non-secret revision tombstone,
    /// including for an absent credential, to fence an older interactive login.
    /// Runtime/environment sources remain separately configured native inputs.
    pub fn logout(
        &self,
        target: &OAuthTarget,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        self.admit(target, control)?;
        let provider = &target.binding().provider;
        let _lease = self.wait(self.backend.acquire(provider), control)?;
        let current = self.wait(self.backend.load(provider), control)?;
        if let Some(record) = &current
            && parse(&record.raw)?["type"] == REVOKED_CREDENTIAL_TYPE
        {
            return Ok(());
        }
        self.admit(target, control)?;
        let raw = encoded(&json!({"type":REVOKED_CREDENTIAL_TYPE}))?;
        match current {
            Some(record) => self.runtime.block_on(self.backend.compare_exchange(
                provider,
                record.revision,
                raw,
            )),
            None => self.runtime.block_on(self.backend.create(provider, raw)),
        }
    }
    /// Resolve precedence without exposing refresh credentials to the requester.
    /// Only the admitted OAuth owner receives its private hook arguments.
    pub fn key(
        &self,
        target: &OAuthTarget,
        include_fallback: bool,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
        callbacks: &mut dyn Bridge,
    ) -> Result<Option<SecretBytes>, DomainError> {
        self.admit(target, control)?;
        let provider = &target.binding().provider;
        for source in [
            CredentialSource::Runtime,
            CredentialSource::SecretFile,
            CredentialSource::Environment,
        ] {
            if let Some(key) = self
                .wait(self.backend.resolve(provider, source), control)?
                .filter(|k| !k.expose().is_empty())
            {
                return Ok(Some(key));
            }
        }
        let _lease = self.wait(self.backend.acquire(provider), control)?;
        let Some(record) = self.wait(self.backend.load(provider), control)? else {
            return if include_fallback {
                self.wait(
                    self.backend.resolve(provider, CredentialSource::Fallback),
                    control,
                )
            } else {
                Ok(None)
            };
        };
        let mut value = parse(&record.raw)?;
        if value["type"] == REVOKED_CREDENTIAL_TYPE {
            return if include_fallback {
                self.wait(
                    self.backend.resolve(provider, CredentialSource::Fallback),
                    control,
                )
            } else {
                Ok(None)
            };
        }
        if value["type"] == "api_key" {
            return self.wait(
                self.backend.resolve(provider, CredentialSource::Stored),
                control,
            );
        }
        let expiry = oauth(&value)?;
        if expiry <= self.clock.now_ms() as f64 {
            value = self.invoke(
                target,
                "refreshToken",
                &json!({"credentials":value}),
                control,
                stream,
                callbacks,
            )?;
            if !value.is_object() {
                return Err(invalid());
            }
            value["type"] = json!("oauth");
            if oauth(&value)? <= self.clock.now_ms() as f64 {
                return Err(invalid());
            }
            let raw = encoded(&value)?;
            // Validate the provider's key before changing its persisted record.
            let key = self.api_key(target, &value, control, stream, callbacks)?;
            self.admit(target, control)?;
            self.runtime.block_on(
                self.backend
                    .compare_exchange(provider, record.revision, raw),
            )?;
            return Ok(Some(key));
        }
        self.api_key(target, &value, control, stream, callbacks)
            .map(Some)
    }
    fn api_key(
        &self,
        target: &OAuthTarget,
        value: &Value,
        control: &ScanControl<'_>,
        stream: &mut dyn NativeStream,
        callbacks: &mut dyn Bridge,
    ) -> Result<SecretBytes, DomainError> {
        let key = self.invoke(
            target,
            "getApiKey",
            &json!({"credentials":value}),
            control,
            stream,
            callbacks,
        )?;
        let key = key
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 65536)
            .ok_or_else(invalid)?;
        Ok(SecretBytes::new(key.as_bytes().to_vec()))
    }
}
impl<B: CredentialBackend> crate::extension_providers::OAuth for Service<B> {
    fn credentials(
        &mut self,
        provider: &str,
        extension: &str,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        self.model_credentials(provider, extension, control)
    }
}
impl<B: CredentialBackend> crate::extension_providers::OAuth for std::sync::Arc<Service<B>> {
    fn credentials(
        &mut self,
        provider: &str,
        extension: &str,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        self.model_credentials(provider, extension, control)
    }
}
impl<B: CredentialBackend> Service<B> {
    fn model_credentials(
        &self,
        provider: &str,
        extension: &str,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        control.check()?;
        if !self
            .owners
            .get(provider)
            .is_some_and(|owners| owners.contains(extension))
        {
            return Err(DomainError::PermissionDenied);
        }
        let _lease = self.wait(self.backend.acquire(provider), control)?;
        let Some(record) = self.wait(self.backend.load(provider), control)? else {
            return Ok(None);
        };
        let value = parse(&record.raw)?;
        if value["type"] != "oauth" {
            return Ok(None);
        }
        oauth(&value)?;
        Ok(Some(value))
    }
}
