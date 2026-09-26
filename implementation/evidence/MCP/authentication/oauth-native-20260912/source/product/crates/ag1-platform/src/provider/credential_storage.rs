//! Durable credential adapter over the P05 single-writer storage port. Construct
//! one backend per vault, then clone it for coordinators; clones share refresh locks.
use super::auth::*;
use crate::executor::BlockingPool;
use ag1_core::{DomainError, PortFuture, SecretBytes, storage::StoragePort};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub struct StorageCredentialBackend<S: StoragePort + 'static> {
    inner: Arc<Inner<S>>,
}
struct Inner<S: StoragePort + 'static> {
    storage: Arc<S>,
    pool: Arc<BlockingPool>,
    lock: Arc<tokio::sync::Mutex<()>>,
    sources: Mutex<BTreeMap<(String, u8), SecretBytes>>,
}
impl<S: StoragePort + 'static> Clone for StorageCredentialBackend<S> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}
fn source_id(source: CredentialSource) -> u8 {
    match source {
        CredentialSource::Runtime => 0,
        CredentialSource::SecretFile => 1,
        CredentialSource::Environment => 2,
        CredentialSource::Stored => 3,
        CredentialSource::Fallback => 4,
    }
}
fn parse(bytes: &[u8]) -> Result<Value, DomainError> {
    if bytes.len() > 1024 * 1024 {
        return Err(DomainError::LimitExceeded("credential record".into()));
    }
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|_| DomainError::InvalidInput("corrupt credential record".into()))?;
    if !v.is_object() {
        return Err(DomainError::InvalidInput("credential record object".into()));
    }
    Ok(v)
}
impl<S: StoragePort + 'static> StorageCredentialBackend<S> {
    /// Takes ownership of the storage adapter. Its single-writer lease must span the
    /// backend lifetime; external writers are not allowed during OAuth refresh.
    pub fn new(storage: S, pool: Arc<BlockingPool>) -> Self {
        Self {
            inner: Arc::new(Inner {
                storage: Arc::new(storage),
                pool,
                lock: Arc::new(tokio::sync::Mutex::new(())),
                sources: Mutex::new(BTreeMap::new()),
            }),
        }
    }
    /// Explicit host config resolution. No ambient variable/file/command is guessed.
    pub fn set_resolved(
        &self,
        provider: &str,
        source: CredentialSource,
        value: SecretBytes,
    ) -> Result<(), DomainError> {
        if provider.is_empty() || provider.len() > 128 || value.expose().len() > 64 * 1024 {
            return Err(DomainError::InvalidInput("resolved credential".into()));
        }
        let mut sources = self
            .inner
            .sources
            .lock()
            .map_err(|_| DomainError::Unavailable("credential source lock".into()))?;
        if sources.len() >= 640 && !sources.contains_key(&(provider.into(), source_id(source))) {
            return Err(DomainError::LimitExceeded("credential sources".into()));
        }
        sources.insert((provider.into(), source_id(source)), value);
        Ok(())
    }
    /// Explicit import into the new private vault only. Original credential files
    /// are read-only inputs and are never overwritten by this adapter.
    pub async fn import(&self, provider: &str, raw: SecretBytes) -> Result<(), DomainError> {
        let _guard = self.inner.lock.lock().await;
        self.create(provider, raw).await
    }
}
impl<S: StoragePort + 'static> CredentialBackend for StorageCredentialBackend<S> {
    fn create<'a>(&'a self, provider: &'a str, raw: SecretBytes) -> PortFuture<'a, ()> {
        let store = self.inner.storage.clone();
        let provider = provider.to_owned();
        self.inner.pool.submit(move || {
            let record = parse(raw.expose())?;
            let body = serde_json::to_vec(&json!({"revision":0,"credential":record}))
                .map_err(|_| DomainError::InvalidInput("credential creation".into()))?;
            store.commit_metadata("provider-credential", &provider, None, &body)?;
            Ok(())
        })
    }
    fn acquire<'a>(&'a self, _provider: &'a str) -> PortFuture<'a, Box<dyn CredentialLease + 'a>> {
        Box::pin(async move {
            Ok(Box::new(self.inner.lock.clone().lock_owned().await) as Box<dyn CredentialLease>)
        })
    }
    fn resolve<'a>(
        &'a self,
        provider: &'a str,
        source: CredentialSource,
    ) -> PortFuture<'a, Option<SecretBytes>> {
        Box::pin(async move {
            {
                let sources = self
                    .inner
                    .sources
                    .lock()
                    .map_err(|_| DomainError::Unavailable("credential source lock".into()))?;
                if let Some(v) = sources.get(&(provider.into(), source_id(source))) {
                    return Ok(Some(SecretBytes::new(v.expose().to_vec())));
                }
            }
            if source != CredentialSource::Stored {
                return Ok(None);
            }
            let Some(record) = self.load(provider).await? else {
                return Ok(None);
            };
            let value = parse(record.raw.expose())?;
            if value["type"] != "api_key" {
                return Ok(None);
            }
            let Some(key) = value["key"].as_str() else {
                return Err(DomainError::InvalidInput("stored API key".into()));
            };
            if key.starts_with('!') {
                return Err(DomainError::Unavailable(
                    "credential command requires the host policy broker".into(),
                ));
            }
            if key.is_empty() {
                return Ok(None);
            }
            Ok(Some(SecretBytes::new(key.as_bytes().to_vec())))
        })
    }
    fn load<'a>(&'a self, provider: &'a str) -> PortFuture<'a, Option<CredentialRecord>> {
        let store = self.inner.storage.clone();
        let provider = provider.to_owned();
        self.inner.pool.submit(move || {
            let Some(doc) = store.load_metadata("provider-credential", &provider)? else {
                return Ok(None);
            };
            if doc.diagnostic.is_some() {
                return Err(DomainError::InvalidInput(
                    "corrupt credential metadata".into(),
                ));
            }
            let value = parse(&doc.bytes)?;
            let revision = value["revision"]
                .as_u64()
                .ok_or_else(|| DomainError::InvalidInput("credential revision".into()))?;
            let credential = &value["credential"];
            if !credential.is_object() {
                return Err(DomainError::InvalidInput("credential object".into()));
            }
            Ok(Some(CredentialRecord {
                revision,
                raw: SecretBytes::new(
                    serde_json::to_vec(credential)
                        .map_err(|_| DomainError::InvalidInput("credential JSON".into()))?,
                ),
            }))
        })
    }
    fn compare_exchange<'a>(
        &'a self,
        provider: &'a str,
        revision: u64,
        raw: SecretBytes,
    ) -> PortFuture<'a, ()> {
        let store = self.inner.storage.clone();
        let provider = provider.to_owned();
        self.inner.pool.submit(move || {
            let Some(doc) = store.load_metadata("provider-credential", &provider)? else {
                return Err(DomainError::NotFound);
            };
            let old = parse(&doc.bytes)?;
            if old["revision"].as_u64() != Some(revision) {
                return Err(DomainError::Conflict);
            }
            let revision = revision
                .checked_add(1)
                .ok_or_else(|| DomainError::LimitExceeded("credential revision".into()))?;
            let record = parse(raw.expose())?;
            let body = serde_json::to_vec(&json!({"revision":revision,"credential":record}))
                .map_err(|_| DomainError::InvalidInput("credential record".into()))?;
            store.commit_metadata("provider-credential", &provider, Some(&doc.sha256), &body)?;
            Ok(())
        })
    }
}
