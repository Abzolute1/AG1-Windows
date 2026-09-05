use ag1_core::{DomainError, NetworkPort, NetworkRequest, PortFuture, SecretBytes, SecretPort};
use std::collections::{BTreeMap, BTreeSet};

/// Fixture credentials are explicitly injected; no credential files or ambient environment lookup.
#[derive(Default)]
pub struct FakeSecretPort {
    values: BTreeMap<(String, String), Vec<u8>>,
}
impl FakeSecretPort {
    pub fn insert(
        &mut self,
        namespace: String,
        key: String,
        bytes: Vec<u8>,
    ) -> Result<(), DomainError> {
        if self.values.len() >= 128 || bytes.len() > 8192 {
            return Err(DomainError::LimitExceeded("fixture secrets".into()));
        }
        self.values.insert((namespace, key), bytes);
        Ok(())
    }
}
impl SecretPort for FakeSecretPort {
    fn resolve<'a>(&'a self, namespace: &'a str, key: &'a str) -> PortFuture<'a, SecretBytes> {
        Box::pin(async move {
            self.values
                .get(&(namespace.into(), key.into()))
                .cloned()
                .map(SecretBytes::new)
                .ok_or(DomainError::NotFound)
        })
    }
}
/// Explicit origin allowlist and response bounds. This does not perform HTTP or DNS.
#[derive(Default)]
pub struct FakeNetworkPort {
    origins: BTreeSet<String>,
    responses: BTreeMap<(String, String), Vec<u8>>,
}
impl FakeNetworkPort {
    pub fn allow_origin(&mut self, origin: String) -> Result<(), DomainError> {
        if !origin.starts_with("https://")
            || origin[8..].contains(['/', '@', '?', '#'])
            || self.origins.len() >= 64
        {
            return Err(DomainError::InvalidInput(
                "exact HTTPS origin required".into(),
            ));
        }
        self.origins.insert(origin);
        Ok(())
    }
    pub fn respond(
        &mut self,
        origin: String,
        path: String,
        bytes: Vec<u8>,
    ) -> Result<(), DomainError> {
        if self.responses.len() >= 128 || bytes.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("fixture responses".into()));
        }
        self.responses.insert((origin, path), bytes);
        Ok(())
    }
}
impl NetworkPort for FakeNetworkPort {
    fn fetch<'a>(&'a self, request: NetworkRequest) -> PortFuture<'a, Vec<u8>> {
        Box::pin(async move {
            if !self.origins.contains(&request.origin)
                || !request.path.starts_with('/')
                || request.path.starts_with("//")
                || request.path.contains(['\r', '\n'])
            {
                return Err(DomainError::PermissionDenied);
            }
            if request.max_bytes > 1024 * 1024 {
                return Err(DomainError::LimitExceeded("network body".into()));
            }
            let bytes = self
                .responses
                .get(&(request.origin, request.path))
                .ok_or(DomainError::NotFound)?;
            if bytes.len() > request.max_bytes {
                return Err(DomainError::LimitExceeded("network body".into()));
            }
            Ok(bytes.clone())
        })
    }
}

/// Cross-platform lifecycle fixture: exit and pipe EOF are independent signals.
#[derive(Default, Debug)]
pub struct DrainState {
    pub exit_seen: bool,
    pub eof_seen: bool,
    pub elapsed_ms: u64,
}
impl DrainState {
    pub fn tick(&mut self, ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(ms);
    }
    pub fn finished(&self) -> bool {
        self.exit_seen && self.eof_seen
    }
    pub fn needs_tree_cancel(&self) -> bool {
        self.exit_seen && !self.eof_seen && self.elapsed_ms >= 200
    }
}
