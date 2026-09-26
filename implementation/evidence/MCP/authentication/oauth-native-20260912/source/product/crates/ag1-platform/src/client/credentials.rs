//! Host-only credential selection; wire models never carry credentials.
use super::*;
use crate::{
    extension_models::{Credential, Credentials},
    provider::signing::AwsCredentials,
    resource_fs::ScanControl,
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Auth {
    /// Reuse the user's explicitly connected subscription session. The native
    /// host reads current credentials; renderer/model metadata only hold a path.
    CodexSubscription { session_file: PathBuf, #[serde(default)] codex_cli: Option<PathBuf> },
    ApiKey {
        env: String,
    },
    Bearer {
        env: String,
        #[serde(default)]
        anthropic_oauth: bool,
    },
    GeminiCli {
        env: String,
    },
    Aws {
        access_key_env: String,
        secret_key_env: String,
        session_token_env: Option<String>,
        region: String,
    },
}
#[derive(Clone)]
pub struct HostCredentials {
    pub providers: BTreeMap<String, Auth>,
    pub fallback: Option<String>,
    pub(super) vault: Option<super::imported::Vault>,
}
fn secret(name: &str) -> Result<SecretBytes> {
    if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
        return Err(DomainError::InvalidInput(
            "credential environment variable name".into(),
        ));
    }
    Ok(SecretBytes::new(
        std::env::var(name)
            .map_err(|_| {
                DomainError::Unavailable("Configured provider credential is missing".into())
            })?
            .into_bytes(),
    ))
}
impl HostCredentials {
    pub(super) async fn resolve_async(&self, model: &Value) -> Result<Credential> {
        if let Some(Auth::CodexSubscription { session_file, codex_cli }) = self.providers.get(model["provider"].as_str().unwrap_or("")) {
            let (path, cli, model)=(session_file.clone(),codex_cli.clone(),model.clone());
            return tokio::task::spawn_blocking(move || crate::provider::codex_subscription::credential_with_refresh(&path,cli.as_deref(),&model).map(Credential::Bearer)).await.map_err(|_|DomainError::Unavailable("ChatGPT sign-in worker failed".into()))?;
        }
        if !self
            .providers
            .contains_key(model["provider"].as_str().unwrap_or(""))
            && let Some(vault) = &self.vault
            && let Some(value) = vault.resolve(model).await?
        {
            return Ok(value);
        }
        self.explicit(model)
    }
    pub fn credential(&self, model: &Value) -> Result<Credential> {
        if !self
            .providers
            .contains_key(model["provider"].as_str().unwrap_or(""))
            && let Some(vault) = &self.vault
            && let Some(value) = vault.resolve_blocking(model)?
        {
            return Ok(value);
        }
        self.explicit(model)
    }
    fn explicit(&self, model: &Value) -> Result<Credential> {
        Ok(
            match self
                .providers
                .get(model["provider"].as_str().ok_or(DomainError::NotFound)?)
            {
                Some(Auth::CodexSubscription { session_file, codex_cli }) => Credential::Bearer(crate::provider::codex_subscription::credential_with_refresh(session_file, codex_cli.as_deref(), model)?),
                Some(Auth::ApiKey { env }) => Credential::ApiKey(secret(env)?),
                Some(Auth::Bearer { env, .. }) => Credential::Bearer(secret(env)?),
                Some(Auth::GeminiCli { env }) => Credential::GeminiCli(secret(env)?),
                Some(Auth::Aws {
                    access_key_env,
                    secret_key_env,
                    session_token_env,
                    region,
                }) => Credential::Aws {
                    credentials: AwsCredentials {
                        access_key: secret(access_key_env)?,
                        secret_key: secret(secret_key_env)?,
                        session_token: session_token_env
                            .as_ref()
                            .map(|name| secret(name))
                            .transpose()?,
                    },
                    region: region.clone(),
                    date: chrono::DateTime::from_timestamp_millis(WallClock.now_ms() as i64)
                        .ok_or(DomainError::Conflict)?
                        .format("%Y%m%dT%H%M%SZ")
                        .to_string(),
                },
                None => Credential::ApiKey(
                    self.fallback
                        .as_ref()
                        .map(|name| secret(name))
                        .transpose()?
                        .unwrap_or_else(|| SecretBytes::new(vec![])),
                ),
            },
        )
    }
    pub fn anthropic_oauth(&self, model: &Value) -> bool {
        matches!(
            self.providers.get(model["provider"].as_str().unwrap_or("")),
            Some(Auth::Bearer {
                anthropic_oauth: true,
                ..
            })
        )
    }
}
impl Credentials for HostCredentials {
    fn resolve(&mut self, model: &Value, control: &ScanControl<'_>) -> Result<Credential> {
        control.check()?;
        self.credential(model)
    }
}
pub fn borrowed(credential: &Credential) -> HttpCredential<'_> {
    match credential {
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
    }
}
