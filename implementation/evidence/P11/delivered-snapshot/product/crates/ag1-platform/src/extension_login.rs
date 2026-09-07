//! Native login entrypoint and narrowly scoped OAuth UI callbacks. No extension
//! request gets this controller capability by registering a provider.
use crate::{
    extension_auth::{LoginOptions, Service},
    extension_host::{Bridge, ExtensionHost, NativeRequest, NativeStream, ProviderInvocation},
    extension_providers::{Catalog, OAuthTarget},
    provider::auth::CredentialBackend,
    resource_fs::ScanControl,
};
use ag1_core::{DomainError, SecretBytes, resources::packages};
use serde_json::{Value, json};

pub trait LoginUi {
    fn auth(&mut self, info: &Value, control: &ScanControl<'_>) -> Result<(), DomainError>;
    fn prompt(
        &mut self,
        prompt: &Value,
        control: &ScanControl<'_>,
    ) -> Result<SecretBytes, DomainError>;
    fn progress(&mut self, message: &str, control: &ScanControl<'_>) -> Result<(), DomainError>;
    fn supports_manual_code(&self) -> bool {
        false
    }
    fn manual_code(&mut self, _: &ScanControl<'_>) -> Result<SecretBytes, DomainError> {
        Err(DomainError::Unavailable(
            "manual OAuth code entry is unavailable".into(),
        ))
    }
}
struct UiBridge<'a> {
    target: &'a OAuthTarget,
    ui: &'a mut dyn LoginUi,
}
fn invalid() -> DomainError {
    DomainError::InvalidInput("extension OAuth UI request".into())
}
fn text(value: SecretBytes, allow_empty: bool) -> Result<Value, DomainError> {
    if value.expose().len() > 65536 || !allow_empty && value.expose().is_empty() {
        return Err(invalid());
    }
    Ok(json!(
        std::str::from_utf8(value.expose()).map_err(|_| invalid())?
    ))
}
impl Bridge for UiBridge<'_> {
    fn request(
        &mut self,
        r: NativeRequest<'_>,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        control.check()?;
        self.target.validate()?;
        if !self.target.accepts_callback(&r) {
            return Err(DomainError::PermissionDenied);
        }
        packages::validate_settings_document(r.arguments)?;
        if r.arguments.to_string().len() > 65536 {
            return Err(invalid());
        }
        let answer = match r.method {
            "oauth.onAuth" => {
                let info = &r.arguments["info"];
                let raw = info["url"]
                    .as_str()
                    .filter(|s| s.len() <= 8192)
                    .ok_or_else(invalid)?;
                let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
                if !matches!(url.scheme(), "https" | "http")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || info.get("instructions").is_some_and(|v| !v.is_string())
                {
                    return Err(invalid());
                }
                self.ui.auth(info, control)?;
                None
            }
            "oauth.onPrompt" => {
                let prompt = &r.arguments["prompt"];
                if !prompt["message"].is_string()
                    || prompt.get("placeholder").is_some_and(|v| !v.is_string())
                    || prompt.get("allowEmpty").is_some_and(|v| !v.is_boolean())
                {
                    return Err(invalid());
                }
                Some(text(
                    self.ui.prompt(prompt, control)?,
                    prompt["allowEmpty"] == true,
                )?)
            }
            "oauth.onProgress" => {
                let message = r.arguments["message"].as_str().ok_or_else(invalid)?;
                self.ui.progress(message, control)?;
                None
            }
            "oauth.onManualCodeInput" if self.ui.supports_manual_code() => {
                Some(text(self.ui.manual_code(control)?, false)?)
            }
            _ => return Err(DomainError::PermissionDenied),
        };
        control.check()?;
        self.target.validate()?;
        Ok(answer)
    }
}
struct Root<'a, 'b> {
    host: &'a mut ExtensionHost,
    target: &'a OAuthTarget,
    control: &'a ScanControl<'b>,
}
impl NativeStream for Root<'_, '_> {
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
        self.target.validate()?;
        if invocation.name != self.target.binding().provider
            || invocation.registration != self.target.token()
            || invocation.hook != "oauth.login"
        {
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
        self.host.call(&scope,"provider",&json!({"name":invocation.name,"registration":invocation.registration,"hook":invocation.hook,"arguments":invocation.arguments}),&control,bridge)
    }
}
/// Called by the native controller after its user login action. UI responses and
/// credentials never become journal/tool evidence. The helper sees only the
/// callback capabilities explicitly supplied for this one admitted login.
pub fn login<B: CredentialBackend>(
    service: &Service<B>,
    catalog: &Catalog,
    host: &mut ExtensionHost,
    provider: &str,
    ui: &mut dyn LoginUi,
    control: &ScanControl<'_>,
) -> Result<(), DomainError> {
    control.check()?;
    let target = catalog
        .oauth_target_for_host(host, provider)?
        .ok_or(DomainError::PermissionDenied)?;
    let options = LoginOptions {
        manual_code_input: ui.supports_manual_code(),
    };
    service.login_with_options(
        &target,
        &options,
        control,
        &mut Root {
            host,
            target: &target,
            control,
        },
        &mut UiBridge {
            target: &target,
            ui,
        },
    )
}

pub fn logout<B: CredentialBackend>(
    service: &Service<B>,
    catalog: &Catalog,
    host: &ExtensionHost,
    provider: &str,
    control: &ScanControl<'_>,
) -> Result<(), DomainError> {
    control.check()?;
    let target = catalog
        .oauth_target_for_host(host, provider)?
        .ok_or(DomainError::PermissionDenied)?;
    service.logout(&target, control)
}
