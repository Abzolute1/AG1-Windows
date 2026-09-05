use ag1_core::{Clock, DomainError, PortFuture, SecretBytes};
use ag1_platform::provider::{auth::*, http::ProviderHttp};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
struct FixtureClock;
fn codex_token() -> String {
    format!(
        "fixture.{}.fixture",
        URL_SAFE_NO_PAD
            .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"fixture-account"}}"#)
    )
}
impl Clock for FixtureClock {
    fn now_ms(&self) -> u64 {
        1_000_000
    }
}
#[derive(Clone, Default)]
struct Backend {
    records: Arc<Mutex<Records>>,
    sources: Arc<Mutex<Sources>>,
    vault_lock: Arc<tokio::sync::Mutex<()>>,
    writes: Arc<AtomicUsize>,
    fail_cas: bool,
}
type Records = BTreeMap<String, (u64, Vec<u8>)>;
type Sources = Vec<(String, CredentialSource, Vec<u8>)>;
impl Backend {
    fn set(&self, provider: &str, value: Value) {
        self.records
            .lock()
            .unwrap()
            .insert(provider.into(), (0, serde_json::to_vec(&value).unwrap()));
    }
    fn source(&self, provider: &str, source: CredentialSource, value: &str) {
        self.sources
            .lock()
            .unwrap()
            .push((provider.into(), source, value.as_bytes().to_vec()));
    }
    fn value(&self, provider: &str) -> Value {
        serde_json::from_slice(&self.records.lock().unwrap()[provider].1).unwrap()
    }
}
impl CredentialBackend for Backend {
    fn acquire<'a>(&'a self, _: &'a str) -> PortFuture<'a, Box<dyn CredentialLease + 'a>> {
        Box::pin(async move {
            Ok(Box::new(self.vault_lock.clone().lock_owned().await) as Box<dyn CredentialLease>)
        })
    }
    fn resolve<'a>(
        &'a self,
        provider: &'a str,
        source: CredentialSource,
    ) -> PortFuture<'a, Option<SecretBytes>> {
        Box::pin(async move {
            if source == CredentialSource::Stored {
                let records = self.records.lock().unwrap();
                return Ok(records
                    .get(provider)
                    .and_then(|(_, b)| serde_json::from_slice::<Value>(b).ok())
                    .and_then(|v| {
                        v["key"]
                            .as_str()
                            .map(|s| SecretBytes::new(s.as_bytes().to_vec()))
                    }));
            }
            Ok(self
                .sources
                .lock()
                .unwrap()
                .iter()
                .find(|(p, s, _)| p == provider && *s == source)
                .map(|(_, _, v)| SecretBytes::new(v.clone())))
        })
    }
    fn load<'a>(&'a self, provider: &'a str) -> PortFuture<'a, Option<CredentialRecord>> {
        Box::pin(async move {
            Ok(self
                .records
                .lock()
                .unwrap()
                .get(provider)
                .map(|(revision, raw)| CredentialRecord {
                    revision: *revision,
                    raw: SecretBytes::new(raw.clone()),
                }))
        })
    }
    fn compare_exchange<'a>(
        &'a self,
        provider: &'a str,
        revision: u64,
        raw: SecretBytes,
    ) -> PortFuture<'a, ()> {
        Box::pin(async move {
            if self.fail_cas {
                return Err(DomainError::Conflict);
            }
            let mut records = self.records.lock().unwrap();
            let old = records.get_mut(provider).ok_or(DomainError::NotFound)?;
            if old.0 != revision {
                return Err(DomainError::Conflict);
            }
            *old = (revision + 1, raw.expose().to_vec());
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
#[derive(Clone, Default)]
struct Refresh {
    calls: Arc<AtomicUsize>,
    fail: bool,
}
impl OAuthRefresh for Refresh {
    fn refresh<'a>(
        &'a self,
        service: OAuthService,
        raw: &'a SecretBytes,
    ) -> PortFuture<'a, SecretBytes> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            if self.fail {
                return Err(DomainError::Unavailable("fixture expired refresh".into()));
            }
            let old: Value = serde_json::from_slice(raw.expose()).unwrap();
            let access = if service == OAuthService::Codex {
                codex_token()
            } else {
                "new-access".into()
            };
            let reply=SecretBytes::new(serde_json::to_vec(&json!({"access_token":access,"refresh_token":"rotated-refresh","expires_in":3600})).unwrap());
            refreshed_record(service, &old, &reply, 1_000_000)
        })
    }
}
fn oauth() -> Value {
    json!({"type":"oauth","access":"expired-access","refresh":"old-refresh","expires":0,"projectId":"fixture-project","unknown":{"keep":true}})
}
#[tokio::test]
async fn credential_precedence_matches_runtime_file_environment_store_and_fallback() {
    let sources = [
        CredentialSource::Runtime,
        CredentialSource::SecretFile,
        CredentialSource::Environment,
        CredentialSource::Stored,
        CredentialSource::Fallback,
    ];
    for first in 0..sources.len() {
        let backend = Backend::default();
        for (i, source) in sources.iter().enumerate().skip(first) {
            if *source == CredentialSource::Stored {
                backend.set(
                    "provider",
                    json!({"type":"api_key","key":format!("fixture-{i}")}),
                );
            } else {
                backend.source("provider", *source, &format!("fixture-{i}"));
            }
        }
        let coordinator = AuthCoordinator::new(backend, Refresh::default(), Arc::new(FixtureClock));
        let key = coordinator
            .resolve("provider", None, true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.value.expose(), format!("fixture-{first}").as_bytes());
        assert_eq!(key.source, sources[first]);
    }
    let backend = Backend::default();
    backend.source("nvidia", CredentialSource::SecretFile, "parent-key");
    backend.set(
        "nvidia-nim",
        json!({"type":"api_key","key":"stale-child-key"}),
    );
    let coordinator = AuthCoordinator::new(backend, Refresh::default(), Arc::new(FixtureClock));
    assert_eq!(
        coordinator
            .resolve("nvidia-nim", None, false)
            .await
            .unwrap()
            .unwrap()
            .value
            .expose(),
        b"parent-key"
    );
    assert!(
        coordinator
            .resolve("missing", None, false)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn revoked_records_skip_refresh_preserve_revision_and_respect_independent_sources() {
    let backend = Backend::default();
    backend.set("anthropic", json!({"type":REVOKED_CREDENTIAL_TYPE}));
    backend.source("anthropic", CredentialSource::Fallback, "fixture-fallback");
    let refresh = Refresh::default();
    let coordinator =
        AuthCoordinator::new(backend.clone(), refresh.clone(), Arc::new(FixtureClock));
    assert!(
        coordinator
            .resolve("anthropic", Some(OAuthService::Anthropic), false)
            .await
            .unwrap()
            .is_none()
    );
    let fallback = coordinator
        .resolve("anthropic", Some(OAuthService::Anthropic), true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fallback.source, CredentialSource::Fallback);
    assert_eq!(fallback.value.expose(), b"fixture-fallback");
    assert!(fallback.oauth.is_none());
    backend.source(
        "anthropic",
        CredentialSource::Environment,
        "fixture-environment",
    );
    let environment = coordinator
        .resolve("anthropic", Some(OAuthService::Anthropic), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(environment.source, CredentialSource::Environment);
    assert_eq!(environment.value.expose(), b"fixture-environment");
    assert_eq!(refresh.calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.writes.load(Ordering::SeqCst), 0);
    assert_eq!(backend.records.lock().unwrap()["anthropic"].0, 0);
    assert_eq!(
        backend.value("anthropic"),
        json!({"type":REVOKED_CREDENTIAL_TYPE})
    );
}
#[tokio::test]
async fn concurrent_coordinators_share_one_locked_refresh_and_preserve_unknown_fields() {
    let backend = Backend::default();
    backend.set("anthropic", oauth());
    let refresh = Refresh::default();
    let first = Arc::new(AuthCoordinator::new(
        backend.clone(),
        refresh.clone(),
        Arc::new(FixtureClock),
    ));
    let second = Arc::new(AuthCoordinator::new(
        backend.clone(),
        refresh.clone(),
        Arc::new(FixtureClock),
    ));
    let mut tasks = vec![];
    for i in 0..24 {
        let coordinator = if i % 2 == 0 {
            first.clone()
        } else {
            second.clone()
        };
        tasks.push(tokio::spawn(async move {
            coordinator
                .resolve("anthropic", Some(OAuthService::Anthropic), true)
                .await
                .unwrap()
                .unwrap()
                .value
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().expose(), b"new-access");
    }
    assert_eq!(refresh.calls.load(Ordering::SeqCst), 1);
    assert_eq!(backend.writes.load(Ordering::SeqCst), 1);
    assert_eq!(backend.value("anthropic")["unknown"], json!({"keep":true}));
    assert_eq!(backend.value("anthropic")["refresh"], "rotated-refresh");
    assert_eq!(backend.value("anthropic")["expires"], 4_300_000);
}
#[tokio::test]
async fn failed_refresh_corrupt_records_and_cas_conflicts_never_replace_credentials() {
    for fail_refresh in [false, true] {
        let backend = Backend {
            fail_cas: !fail_refresh,
            ..Default::default()
        };
        backend.set("codex", oauth());
        let original = backend.value("codex");
        let coordinator = AuthCoordinator::new(
            backend.clone(),
            Refresh {
                fail: fail_refresh,
                ..Default::default()
            },
            Arc::new(FixtureClock),
        );
        assert!(
            coordinator
                .resolve("codex", Some(OAuthService::Codex), true)
                .await
                .is_err()
        );
        assert_eq!(backend.value("codex"), original);
        assert_eq!(backend.writes.load(Ordering::SeqCst), 0);
    }
    let backend = Backend::default();
    backend
        .records
        .lock()
        .unwrap()
        .insert("corrupt".into(), (0, b"{broken".to_vec()));
    let refresh = Refresh::default();
    let coordinator =
        AuthCoordinator::new(backend.clone(), refresh.clone(), Arc::new(FixtureClock));
    assert!(
        coordinator
            .resolve("corrupt", Some(OAuthService::Anthropic), true)
            .await
            .is_err()
    );
    assert_eq!(refresh.calls.load(Ordering::SeqCst), 0);
    assert_eq!(backend.records.lock().unwrap()["corrupt"].1, b"{broken");
}
#[tokio::test]
async fn placeholder_google_oauth_is_explicit_while_existing_access_tokens_remain_usable() {
    let http = ProviderHttp::new(&["http://127.0.0.1:1".into()], Duration::from_secs(1)).unwrap();
    let refresh = HttpOAuthRefresh::new(http, Arc::new(FixtureClock));
    for service in [OAuthService::GeminiCli, OAuthService::Antigravity] {
        let old = SecretBytes::new(serde_json::to_vec(&oauth()).unwrap());
        assert!(
            matches!(refresh.refresh(service,&old).await,Err(DomainError::Unavailable(message)) if message.contains("placeholder"))
        );
        let backend = Backend::default();
        let mut value = oauth();
        value["expires"] = 2_000_000.into();
        value["access"] = "current-access".into();
        backend.set("google", value);
        let coordinator = AuthCoordinator::new(backend, Refresh::default(), Arc::new(FixtureClock));
        let key = coordinator
            .resolve("google", Some(service), false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(key.value.expose()).unwrap(),
            json!({"token":"current-access","projectId":"fixture-project"})
        );
    }
}
#[test]
fn refresh_response_validation_retains_rotation_project_and_expiry_semantics() {
    for service in [
        OAuthService::Anthropic,
        OAuthService::Codex,
        OAuthService::GeminiCli,
        OAuthService::Antigravity,
        OAuthService::GitHubCopilot,
    ] {
        let reply = if service == OAuthService::GitHubCopilot {
            json!({"token":"new","expires_at":4600})
        } else {
            json!({"access_token":if service==OAuthService::Codex {codex_token()} else {"new".into()},"refresh_token":"rotated","expires_in":3600})
        };
        let bytes = SecretBytes::new(serde_json::to_vec(&reply).unwrap());
        let raw = refreshed_record(service, &oauth(), &bytes, 1_000_000).unwrap();
        let result: Value = serde_json::from_slice(raw.expose()).unwrap();
        assert_eq!(
            result["access"],
            if service == OAuthService::Codex {
                codex_token()
            } else {
                "new".into()
            }
        );
        assert_eq!(result["expires"], 4_300_000);
        assert_eq!(result["projectId"], "fixture-project");
        assert_eq!(
            result["refresh"],
            if service == OAuthService::GitHubCopilot {
                "old-refresh"
            } else {
                "rotated"
            }
        );
        for malformed in [
            json!({}),
            json!({"access_token":"secret-do-not-log","expires_in":"bad"}),
            json!({"access_token":"secret-do-not-log","refresh_token":"private","expires_in":-1}),
        ] {
            let error = refreshed_record(
                service,
                &oauth(),
                &SecretBytes::new(serde_json::to_vec(&malformed).unwrap()),
                1_000_000,
            )
            .err()
            .unwrap();
            assert!(!format!("{error:?}").contains("secret-do-not-log"));
        }
    }
}

#[tokio::test]
async fn durable_vault_refresh_survives_reopen_and_rejects_stale_writes() {
    use ag1_platform::{
        executor::BlockingPool, provider::credential_storage::StorageCredentialBackend,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault");
    let pool = Arc::new(BlockingPool::new(2, 16).unwrap());
    let store = ag1_storage::Store::open(&path).unwrap();
    let backend = StorageCredentialBackend::new(store, pool.clone());
    backend
        .import(
            "anthropic",
            SecretBytes::new(serde_json::to_vec(&oauth()).unwrap()),
        )
        .await
        .unwrap();
    assert!(
        backend
            .import("anthropic", SecretBytes::new(b"{}".to_vec()))
            .await
            .is_err()
    );
    let refresh = Refresh::default();
    let calls = refresh.calls.clone();
    let coordinator = Arc::new(AuthCoordinator::new(
        backend.clone(),
        refresh,
        Arc::new(FixtureClock),
    ));
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..24 {
        let c = coordinator.clone();
        set.spawn(async move {
            let key = c
                .resolve("anthropic", Some(OAuthService::Anthropic), false)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(key.value.expose(), b"new-access");
        });
    }
    while let Some(result) = set.join_next().await {
        result.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let loaded = backend.load("anthropic").await.unwrap().unwrap();
    assert_eq!(loaded.revision, 1);
    let value: Value = serde_json::from_slice(loaded.raw.expose()).unwrap();
    assert_eq!(value["unknown"], json!({"keep":true}));
    assert_eq!(
        backend
            .compare_exchange("anthropic", 0, SecretBytes::new(b"{}".to_vec()))
            .await,
        Err(DomainError::Conflict)
    );
    drop(coordinator);
    drop(backend);
    let reopened = StorageCredentialBackend::new(ag1_storage::Store::open(&path).unwrap(), pool);
    let loaded = reopened.load("anthropic").await.unwrap().unwrap();
    assert_eq!(loaded.revision, 1);
    let value: Value = serde_json::from_slice(loaded.raw.expose()).unwrap();
    assert_eq!(value["refresh"], "rotated-refresh");
    assert_eq!(value["unknown"], json!({"keep":true}));
}
