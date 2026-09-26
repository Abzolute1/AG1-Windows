//! Private settings and credential input references. Original input bytes stay
//! immutable; native settings and token refresh state live in the new state dir.
use super::*;
use crate::session::commands::QueueMode;
use std::os::unix::fs::OpenOptionsExt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub path: PathBuf,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inputs {
    pub settings: Option<Input>,
    pub credentials: Option<Input>,
}
impl Input {
    pub fn read(&self) -> Result<Vec<u8>> {
        if !self.path.is_absolute() {
            return Err(DomainError::PermissionDenied);
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.path)
            .map_err(crate::io_error)?;
        if !file.metadata().map_err(crate::io_error)?.is_file() {
            return Err(DomainError::PermissionDenied);
        }
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(crate::io_error)?;
        if bytes.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded(
                "Private configuration input".into(),
            ));
        }
        if ag1_storage::digest(&bytes) != self.sha256 {
            return Err(DomainError::Conflict);
        }
        Ok(bytes)
    }
}

#[derive(Clone, Default)]
pub(super) struct Settings {
    pub thinking: Option<String>,
    pub context: Option<u64>,
    pub compaction: ag1_core::context::compaction::Settings,
    pub retry: ag1_core::runtime::retry::Settings,
    pub steering: QueueMode,
    pub follow_up: QueueMode,
}
fn invalid() -> DomainError {
    DomainError::InvalidInput("Imported native settings".into())
}
fn number(value: Option<&Value>) -> Result<Option<u64>> {
    value.map(|v| v.as_u64().ok_or_else(invalid)).transpose()
}
fn flag(value: Option<&Value>) -> Result<Option<bool>> {
    value.map(|v| v.as_bool().ok_or_else(invalid)).transpose()
}
fn queue(value: Option<&Value>) -> Result<QueueMode> {
    match value.and_then(Value::as_str) {
        None if value.is_none() => Ok(QueueMode::OneAtATime),
        Some("all") => Ok(QueueMode::All),
        Some("one-at-a-time") => Ok(QueueMode::OneAtATime),
        _ => Err(invalid()),
    }
}
impl Settings {
    pub(super) fn load(config: &Config, state: &Path) -> Result<Self> {
        let Some(input) = config.imported.as_ref().and_then(|i| i.settings.as_ref()) else {
            return Ok(Self::default());
        };
        let raw = input.read()?;
        let directory = state.join("home/.ag1/agent");
        crate::cutover::private_dir(&directory)?;
        let path = directory.join("settings.json");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(&raw)
                    .and_then(|_| file.sync_all())
                    .map_err(crate::io_error)?;
                fs::File::open(&directory)
                    .and_then(|f| f.sync_all())
                    .map_err(crate::io_error)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(crate::io_error(e)),
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(crate::io_error)?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(crate::io_error)?;
        if bytes.len() > 1024 * 1024 {
            return Err(invalid());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        let object = value.as_object().ok_or_else(invalid)?;
        let mut settings = Self::default();
        if let Some(thinking) = object.get("defaultThinkingLevel") {
            let thinking = thinking
                .as_str()
                .filter(|s| ["off", "minimal", "low", "medium", "high", "xhigh", "max"].contains(s))
                .ok_or_else(invalid)?;
            settings.thinking = Some(thinking.into());
        }
        settings.context = number(object.get("contextWindow"))?;
        settings.steering = queue(object.get("steeringMode"))?;
        settings.follow_up = queue(object.get("followUpMode"))?;
        if let Some(compaction) = object.get("compaction") {
            let c = compaction.as_object().ok_or_else(invalid)?;
            settings.compaction.enabled =
                flag(c.get("enabled"))?.unwrap_or(settings.compaction.enabled);
            settings.compaction.reserve_tokens =
                number(c.get("reserveTokens"))?.unwrap_or(settings.compaction.reserve_tokens);
            settings.compaction.keep_recent_tokens = number(c.get("keepRecentTokens"))?
                .unwrap_or(settings.compaction.keep_recent_tokens);
        }
        if let Some(retry) = object.get("retry") {
            let r = retry.as_object().ok_or_else(invalid)?;
            settings.retry.enabled = flag(r.get("enabled"))?.unwrap_or(settings.retry.enabled);
            settings.retry.max_retries = number(r.get("maxRetries"))?
                .map(u32::try_from)
                .transpose()
                .map_err(|_| invalid())?
                .unwrap_or(settings.retry.max_retries);
            settings.retry.base_delay_ms =
                number(r.get("baseDelayMs"))?.unwrap_or(settings.retry.base_delay_ms);
        }
        Ok(settings)
    }
}

type Coordinator = crate::provider::auth::AuthCoordinator<
    crate::provider::credential_storage::StorageCredentialBackend<Store>,
    Refresh,
>;
struct Refresh(Option<crate::provider::auth::HttpOAuthRefresh>);
impl crate::provider::auth::OAuthRefresh for Refresh {
    fn refresh<'a>(
        &'a self,
        service: crate::provider::auth::OAuthService,
        record: &'a SecretBytes,
    ) -> PortFuture<'a, SecretBytes> {
        Box::pin(async move {
            self.0
                .as_ref()
                .ok_or_else(|| {
                    DomainError::Unavailable("No supported OAuth refresh endpoint".into())
                })?
                .refresh(service, record)
                .await
        })
    }
}
#[derive(Clone)]
pub(super) struct Vault {
    auth: Arc<Coordinator>,
    handle: tokio::runtime::Handle,
}
fn service(provider: &str) -> Option<crate::provider::auth::OAuthService> {
    use crate::provider::auth::OAuthService::*;
    match provider {
        "anthropic" => Some(Anthropic),
        "openai-codex" => Some(Codex),
        "github-copilot" => Some(GitHubCopilot),
        "google-gemini-cli" => Some(GeminiCli),
        "google-antigravity" => Some(Antigravity),
        _ => None,
    }
}
fn records(input: &Input) -> Result<serde_json::Map<String, Value>> {
    let value: Value = serde_json::from_slice(&input.read()?).map_err(|_| invalid())?;
    let object = value.as_object().ok_or_else(invalid)?;
    if object.len() > 128 {
        return Err(invalid());
    }
    for (provider, record) in object {
        if provider.is_empty()
            || provider.len() > 128
            || !provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        {
            return Err(invalid());
        }
        match record["type"].as_str() {
            Some("api_key") if record["key"].is_string() => (),
            Some("oauth")
                if service(provider).is_some()
                    && record["access"].is_string()
                    && record["refresh"].is_string()
                    && record["expires"].is_i64() => {}
            _ => return Err(invalid()),
        }
    }
    Ok(object.clone())
}
impl Vault {
    pub(super) async fn load(config: &Config, state: &Path) -> Result<Option<Self>> {
        use crate::provider::{auth::*, credential_storage::StorageCredentialBackend};
        let Some(input) = config
            .imported
            .as_ref()
            .and_then(|i| i.credentials.as_ref())
        else {
            return Ok(None);
        };
        let records = records(input)?;
        // Build an explicit refresh-origin allowlist without making requests or
        // consuming refresh tokens during import. Generation owns any refresh.
        let mut origins = Vec::new();
        for (provider, record) in &records {
            if record["type"] == "oauth"
                && let Some(service) = service(provider)
            {
                match refresh_request(
                    service,
                    &SecretBytes::new(serde_json::to_vec(record).map_err(|_| invalid())?),
                ) {
                    Ok(request) => {
                        origins.push(crate::provider::http::endpoint_origin(&request.url)?)
                    }
                    Err(DomainError::Unavailable(_)) => (), // P07 placeholder-provider boundary.
                    Err(e) => return Err(e),
                }
            }
        }
        let store = Store::open(&state.join("credential-store"))?;
        let backend = StorageCredentialBackend::new(store, Arc::new(BlockingPool::new(1, 32)?));
        for (provider, record) in records {
            if backend.load(&provider).await?.is_none() {
                backend
                    .import(
                        &provider,
                        SecretBytes::new(serde_json::to_vec(&record).map_err(|_| invalid())?),
                    )
                    .await?;
            }
        }
        let clock: Arc<dyn Clock> = Arc::new(WallClock);
        let refresh = if origins.is_empty() {
            None
        } else {
            Some(HttpOAuthRefresh::new(
                ProviderHttp::new(&origins, Duration::from_secs(10))?,
                clock.clone(),
            ))
        };
        Ok(Some(Self {
            auth: Arc::new(AuthCoordinator::new(backend, Refresh(refresh), clock)),
            handle: tokio::runtime::Handle::current(),
        }))
    }
    pub(super) async fn resolve(
        &self,
        model: &Value,
    ) -> Result<Option<crate::extension_models::Credential>> {
        use crate::{extension_models::Credential, provider::auth::OAuthService};
        let provider = model["provider"].as_str().ok_or_else(invalid)?;
        let Some(resolved) = self
            .auth
            .resolve(provider, service(provider), false)
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(match resolved.oauth {
            Some(OAuthService::GeminiCli | OAuthService::Antigravity) => {
                Credential::GeminiCli(resolved.value)
            }
            Some(_) => Credential::Bearer(resolved.value),
            None => Credential::ApiKey(resolved.value),
        }))
    }
    // Extension brokers run on blocking host workers, never renderer/runtime threads.
    pub(super) fn resolve_blocking(
        &self,
        model: &Value,
    ) -> Result<Option<crate::extension_models::Credential>> {
        self.handle.block_on(self.resolve(model))
    }
}
/// Read only after the native host has released its vault writer. This projects
/// current refreshed credentials into the new TS destination through its archive.
pub(crate) fn export_credentials(state: &Path) -> Result<Option<Value>> {
    let config: Config =
        serde_json::from_slice(&fs::read(state.join("host.json")).map_err(crate::io_error)?)
            .map_err(|_| invalid())?;
    let Some(input) = config
        .imported
        .as_ref()
        .and_then(|i| i.credentials.as_ref())
    else {
        return Ok(None);
    };
    let mut records = records(input)?;
    if !state.join("credential-store").exists() {
        return Ok(Some(Value::Object(records)));
    }
    let store = Store::open(&state.join("credential-store"))?;
    for (provider, record) in &mut records {
        let bytes = store
            .metadata("provider-credential", provider)?
            .ok_or(DomainError::NotFound)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if !value["revision"].is_u64() || !value["credential"].is_object() {
            return Err(invalid());
        }
        *record = value["credential"].clone();
    }
    Ok(Some(Value::Object(records)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(settings: &[u8], credentials: &[u8]) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let input = |name: &str, raw: &[u8]| {
            let path = dir.path().join(name);
            fs::write(&path, raw).unwrap();
            Input {
                path,
                sha256: ag1_storage::digest(raw),
            }
        };
        let mut config: Config = serde_json::from_value(json!({"models":[],"selected":"fixture","credential_env":null,"system_prompt":"","fixture":true})).unwrap();
        config.imported = Some(Inputs {
            settings: Some(input("settings-input.json", settings)),
            credentials: Some(input("credentials-input.json", credentials)),
        });
        fs::write(
            dir.path().join("host.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        (dir, config)
    }
    #[test]
    fn explicit_false_zero_and_native_settings_changes_survive_restart() {
        let raw = br#"{"defaultThinkingLevel":"medium","contextWindow":0,"compaction":{"enabled":false,"reserveTokens":0,"keepRecentTokens":0},"retry":{"enabled":false,"maxRetries":0,"baseDelayMs":0},"steeringMode":"all","followUpMode":"all","unknown":[3,1,2]}"#;
        let (dir, config) = setup(raw, b"{}");
        let settings = Settings::load(&config, dir.path()).unwrap();
        assert_eq!(settings.thinking.as_deref(), Some("medium"));
        assert_eq!(settings.context, Some(0));
        assert!(!settings.compaction.enabled);
        assert_eq!(settings.compaction.reserve_tokens, 0);
        assert_eq!(settings.compaction.keep_recent_tokens, 0);
        assert!(!settings.retry.enabled);
        assert_eq!(settings.retry.max_retries, 0);
        assert_eq!(settings.retry.base_delay_ms, 0);
        assert!(matches!(settings.steering, QueueMode::All));
        assert!(matches!(settings.follow_up, QueueMode::All));
        let current = dir.path().join("home/.ag1/agent/settings.json");
        assert_eq!(fs::read(&current).unwrap(), raw);
        fs::write(&current, br#"{"defaultThinkingLevel":"high"}"#).unwrap();
        assert_eq!(
            Settings::load(&config, dir.path())
                .unwrap()
                .thinking
                .as_deref(),
            Some("high")
        );
        assert_eq!(
            config
                .imported
                .as_ref()
                .unwrap()
                .settings
                .as_ref()
                .unwrap()
                .read()
                .unwrap(),
            raw
        );
        fs::remove_file(&current).unwrap();
        std::os::unix::fs::symlink(dir.path().join("settings-input.json"), &current).unwrap();
        assert!(Settings::load(&config, dir.path()).is_err());
    }
    #[tokio::test]
    async fn vault_import_is_private_refresh_survives_restart_and_rollback_uses_current_records() {
        use crate::{
            extension_models::Credential,
            provider::{auth::CredentialBackend, credential_storage::StorageCredentialBackend},
        };
        let raw = br#"{"fixture":{"type":"api_key","key":"original-fixture-secret"},"openai-codex":{"type":"oauth","access":"unexpired-access","refresh":"unused-refresh","expires":9223372036854775807,"extra":false}}"#;
        let (dir, config) = setup(b"{}", raw);
        let vault = Vault::load(&config, dir.path()).await.unwrap().unwrap();
        let Credential::ApiKey(key) = vault
            .resolve(&json!({"provider":"fixture"}))
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("API key mapping")
        };
        assert_eq!(key.expose(), b"original-fixture-secret");
        let Credential::Bearer(key) = vault
            .resolve(&json!({"provider":"openai-codex"}))
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("OAuth mapping")
        };
        assert_eq!(key.expose(), b"unexpired-access");
        drop(vault);
        let backend = StorageCredentialBackend::new(
            Store::open(&dir.path().join("credential-store")).unwrap(),
            Arc::new(BlockingPool::new(1, 8).unwrap()),
        );
        backend
            .compare_exchange(
                "fixture",
                0,
                SecretBytes::new(br#"{"type":"api_key","key":"new-native-secret"}"#.to_vec()),
            )
            .await
            .unwrap();
        drop(backend);
        let vault = Vault::load(&config, dir.path()).await.unwrap().unwrap();
        let Credential::ApiKey(key) = vault
            .resolve(&json!({"provider":"fixture"}))
            .await
            .unwrap()
            .unwrap()
        else {
            panic!("API key mapping")
        };
        assert_eq!(key.expose(), b"new-native-secret");
        drop(vault);
        let exported = export_credentials(dir.path()).unwrap().unwrap();
        assert_eq!(exported["fixture"]["key"], "new-native-secret");
        assert_eq!(exported["openai-codex"]["extra"], false);
        assert_eq!(
            config
                .imported
                .as_ref()
                .unwrap()
                .credentials
                .as_ref()
                .unwrap()
                .read()
                .unwrap(),
            raw
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(dir.path().join("credential-store"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}
