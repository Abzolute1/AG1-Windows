//! MCP-only private credential coordinator. Native pending operations and leases
//! are never deserialized from renderer, model, server or persisted metadata.
//! Network/OAuth work runs outside the vault lock; commit is generation-fenced.
use super::{http::Endpoint, invalid};
use ag1_core::{DomainError, SecretBytes};
use ag1_storage::mcp_private::PrivateStore;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

mod codec;
pub const MAX_PROFILES: usize = 128;
pub const MAX_SECRET_BYTES: usize = 16 * 1024;
const PENDING_TIMEOUT: Duration = Duration::from_secs(600);
pub(crate) const PRIVATE_ECHO: &str =
    "MCP response withheld because it contains a supplied private value";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Target {
    ManualBearer {
        endpoint: String,
        loopback_development: bool,
    },
    OAuth {
        endpoint: String,
        resource: String,
        issuer: String,
        token_endpoint: String,
        client_id: String,
    },
    Local {
        executable_sha256: String,
        environment_names: BTreeSet<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub server_id: String,
    pub profile_id: String,
    pub configuration_revision: u64,
    /// Includes all reviewed configuration fields, not just endpoint origin.
    pub configuration_sha256: String,
    pub target: Target,
}
fn uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
}
fn hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn env_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
impl Binding {
    pub fn validate(&self) -> Result<(), DomainError> {
        if !uuid(&self.server_id)
            || !uuid(&self.profile_id)
            || self.configuration_revision == 0
            || !hash(&self.configuration_sha256)
        {
            return Err(invalid("credential binding"));
        }
        match &self.target {
            Target::ManualBearer {
                endpoint,
                loopback_development,
            } => {
                if *loopback_development {
                    Endpoint::loopback_development(endpoint)?;
                } else {
                    Endpoint::public_https(endpoint)?;
                }
            }
            Target::OAuth {
                endpoint,
                resource,
                issuer,
                token_endpoint,
                client_id,
            } => {
                Endpoint::public_https(endpoint)?;
                super::oauth::url(resource, false)?;
                super::oauth::url(issuer, false)?;
                super::oauth::url(token_endpoint, true)?;
                if client_id.is_empty()
                    || client_id.len() > 4096
                    || client_id.chars().any(char::is_control)
                {
                    return Err(invalid("OAuth public client ID"));
                }
            }
            Target::Local {
                executable_sha256,
                environment_names,
            } => {
                if !hash(executable_sha256)
                    || environment_names.is_empty()
                    || environment_names.len() > 32
                    || environment_names.iter().any(|s| !env_name(s))
                {
                    return Err(invalid("private environment binding"));
                }
            }
        }
        Ok(())
    }
    fn key(&self) -> String {
        format!("{}:{}", self.server_id, self.profile_id)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Grant {
    /// Only a validated authorization flow may supply verified account identity.
    /// Manual tokens and unverified server labels leave this absent.
    pub account: Option<String>,
    pub scopes: BTreeSet<String>,
    pub expires_at_ms: Option<u64>,
}
impl Grant {
    fn validate(&self, target: &Target) -> Result<(), DomainError> {
        if self
            .account
            .as_ref()
            .is_some_and(|s| !super::definitions::label(s, 256))
            || self.scopes.len() > 64
            || self.scopes.iter().any(|s| {
                s.is_empty()
                    || s.len() > 256
                    || !s.bytes().all(|b| {
                        b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b)
                    })
            })
        {
            return Err(invalid("credential grant"));
        }
        if !matches!(target, Target::OAuth { .. })
            && (self.account.is_some() || !self.scopes.is_empty())
        {
            return Err(invalid("manual credential account/scope assertion"));
        }
        if matches!(target, Target::Local { .. }) && self.expires_at_ms.is_some() {
            return Err(invalid("local credential expiry"));
        }
        Ok(())
    }
}

/// No Debug/Clone/Serialize; private payloads are copied only for native custody.
pub enum Material {
    Bearer {
        access: SecretBytes,
        refresh: Option<SecretBytes>,
    },
    Environment(BTreeMap<String, SecretBytes>),
}
impl Material {
    pub(crate) fn reject_echo(&self, value: &serde_json::Value) -> Result<(), DomainError> {
        fn contains(value: &serde_json::Value, needle: &str) -> bool {
            match value {
                serde_json::Value::String(s) => s.contains(needle),
                serde_json::Value::Array(values) => values.iter().any(|v| contains(v, needle)),
                serde_json::Value::Object(values) => values
                    .iter()
                    .any(|(k, v)| k.contains(needle) || contains(v, needle)),
                other => other.to_string().contains(needle),
            }
        }
        let matches = |secret: &SecretBytes| {
            std::str::from_utf8(secret.expose()).is_ok_and(|s| !s.is_empty() && contains(value, s))
        };
        let echoed = match self {
            Self::Bearer { access, refresh } => {
                matches(access) || refresh.as_ref().is_some_and(matches)
            }
            Self::Environment(values) => values.values().any(matches),
        };
        if echoed {
            Err(DomainError::Unavailable(PRIVATE_ECHO.into()))
        } else {
            Ok(())
        }
    }
    pub(crate) fn validate(&self, target: &Target) -> Result<(), DomainError> {
        match (self, target) {
            (
                Self::Bearer { access, refresh },
                Target::ManualBearer { .. } | Target::OAuth { .. },
            ) => {
                let bytes = access.expose();
                let end = bytes.iter().position(|b| *b == b'=').unwrap_or(bytes.len());
                if end == 0
                    || bytes.len() > MAX_SECRET_BYTES
                    || !bytes[..end]
                        .iter()
                        .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(b))
                    || !bytes[end..].iter().all(|b| *b == b'=')
                {
                    return Err(invalid("bearer credential format"));
                }
                if refresh.as_ref().is_some_and(|s| {
                    s.expose().is_empty()
                        || s.expose().len() > MAX_SECRET_BYTES
                        || std::str::from_utf8(s.expose()).is_err()
                }) || (matches!(target, Target::ManualBearer { .. }) && refresh.is_some())
                {
                    return Err(invalid("refresh credential"));
                }
            }
            (
                Self::Environment(values),
                Target::Local {
                    environment_names, ..
                },
            ) => {
                if values.keys().collect::<BTreeSet<_>>() != environment_names.iter().collect()
                    || values.values().any(|s| {
                        s.expose().is_empty()
                            || s.expose().len() > 4096
                            || s.expose().contains(&0)
                            || std::str::from_utf8(s.expose()).is_err()
                    })
                {
                    return Err(invalid("private environment values"));
                }
            }
            _ => return Err(DomainError::PermissionDenied),
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Authenticating,
    Connected,
    Expired,
    Revoked,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicRecord {
    pub binding: Binding,
    pub generation: u64,
    pub status: Status,
    pub grant: Grant,
}
struct Record {
    public: PublicRecord,
    material: Option<Material>,
    refresh: Option<(Weak<()>, Instant)>,
}
struct State {
    revision: u64,
    records: BTreeMap<String, Record>,
    poisoned: bool,
}
pub struct Vault {
    store: PrivateStore,
    state: Mutex<State>,
    owner: Arc<()>,
}
enum PendingKind {
    Login {
        reservation: Arc<()>,
    },
    Refresh {
        reservation: Arc<()>,
        token: SecretBytes,
    },
}
/// One native login/refresh attempt. Its token cannot be supplied in JSON.
pub struct Pending {
    owner: Arc<()>,
    binding: Binding,
    generation: u64,
    expires: Instant,
    kind: PendingKind,
    grant: Grant,
}
impl Pending {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn refresh_token(&self) -> Option<&SecretBytes> {
        match &self.kind {
            PendingKind::Refresh { token, .. } => Some(token),
            _ => None,
        }
    }
    pub(crate) fn grant(&self) -> &Grant {
        &self.grant
    }
}
/// A retained reference contains no secret. Each use checks the live vault.
pub struct CredentialLease {
    vault: Arc<Vault>,
    binding: Binding,
    generation: u64,
}
impl CredentialLease {
    pub(crate) fn reject_echo(&self, value: &serde_json::Value) -> Result<(), DomainError> {
        self.with_material(super::http::credential_now()?, |material| {
            material.reject_echo(value)
        })
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn check(&self, now_ms: u64) -> Result<(), DomainError> {
        self.with_material(now_ms, |_| Ok(()))
    }
    /// Native transport admission only. The short callback runs under the same
    /// lock as logout/commit. It must not await I/O or reenter this vault.
    pub fn with_material<T>(
        &self,
        now_ms: u64,
        use_secret: impl FnOnce(&Material) -> Result<T, DomainError>,
    ) -> Result<T, DomainError> {
        let state = self.vault.lock()?;
        let r = self.vault.current(&state, &self.binding, self.generation)?;
        if r.public.status != Status::Connected
            || r.public.grant.expires_at_ms.is_some_and(|t| now_ms >= t)
        {
            return Err(DomainError::PermissionDenied);
        }
        use_secret(r.material.as_ref().ok_or(DomainError::PermissionDenied)?)
    }
}
impl Vault {
    pub fn open(store: PrivateStore) -> Result<Arc<Self>, DomainError> {
        let saved = store.load().map_err(DomainError::from)?;
        let (revision, records) = match saved {
            Some(s) => (s.revision, codec::decode(s.bytes.expose())?),
            None => (0, BTreeMap::new()),
        };
        let vault = Arc::new(Self {
            store,
            state: Mutex::new(State {
                revision,
                records,
                poisoned: false,
            }),
            owner: Arc::new(()),
        });
        // A restarted process cannot resume an old browser callback. The durable
        // pending generation remains a tombstone; no automatic login/refresh.
        {
            let mut state = vault.lock()?;
            let mut changed = false;
            for r in state.records.values_mut() {
                if r.public.status == Status::Authenticating {
                    r.public.status = Status::Revoked;
                    changed = true;
                }
            }
            if changed {
                vault.persist(&mut state)?;
            }
        }
        Ok(vault)
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, DomainError> {
        let s = self.state.lock().map_err(|_| DomainError::Conflict)?;
        if s.poisoned {
            return Err(DomainError::Unavailable(
                "MCP private state requires reopen after storage failure".into(),
            ));
        }
        Ok(s)
    }
    fn persist(&self, state: &mut State) -> Result<(), DomainError> {
        let result = codec::encode(&state.records).and_then(|raw| {
            self.store
                .commit(state.revision, &raw)
                .map_err(DomainError::from)
        });
        match result {
            Ok(revision) => {
                state.revision = revision;
                Ok(())
            }
            Err(_) => {
                state.poisoned = true;
                for r in state.records.values_mut() {
                    r.material = None;
                    r.refresh = None;
                }
                Err(DomainError::Unavailable(
                    "MCP private state commit failed; reopen required".into(),
                ))
            }
        }
    }
    fn replace(&self, state: &mut State, key: String, record: Record) -> Result<(), DomainError> {
        // Deterministic quota/validation errors leave live and durable state
        // unchanged. Only an uncertain storage failure poisons admissions.
        let raw = codec::replacing(&state.records, &key, &record)?;
        match self.store.commit(state.revision, &raw) {
            Ok(revision) => {
                state.revision = revision;
                state.records.insert(key, record);
                Ok(())
            }
            Err(_) => {
                state.poisoned = true;
                for r in state.records.values_mut() {
                    r.material = None;
                    r.refresh = None;
                }
                Err(DomainError::Unavailable(
                    "MCP private state commit failed; reopen required".into(),
                ))
            }
        }
    }
    fn current<'a>(
        &self,
        state: &'a State,
        binding: &Binding,
        generation: u64,
    ) -> Result<&'a Record, DomainError> {
        let r = state
            .records
            .get(&binding.key())
            .ok_or(DomainError::NotFound)?;
        if r.public.binding != *binding || r.public.generation != generation {
            return Err(DomainError::Conflict);
        }
        Ok(r)
    }
    pub fn records(&self, now_ms: u64) -> Result<Vec<PublicRecord>, DomainError> {
        Ok(self
            .lock()?
            .records
            .values()
            .map(|r| {
                let mut p = r.public.clone();
                if p.status == Status::Connected
                    && p.grant.expires_at_ms.is_some_and(|t| now_ms >= t)
                {
                    p.status = Status::Expired;
                }
                if p.status == Status::Authenticating
                    && r.refresh.as_ref().is_none_or(|(weak, expires)| {
                        weak.strong_count() == 0 || Instant::now() >= *expires
                    })
                {
                    p.status = Status::Revoked;
                }
                p
            })
            .collect())
    }
    /// Begins only after native authentication/private-entry consent. Activation
    /// and tool approval remain separate. Retires previous credential admissions.
    pub fn begin_login(&self, binding: Binding) -> Result<Pending, DomainError> {
        binding.validate()?;
        let mut state = self.lock()?;
        let key = binding.key();
        if !state.records.contains_key(&key) && state.records.len() >= MAX_PROFILES {
            return Err(DomainError::LimitExceeded("MCP credential profiles".into()));
        }
        let generation = state
            .records
            .get(&key)
            .map_or(0, |r| r.public.generation)
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let reservation = Arc::new(());
        let expires = Instant::now() + PENDING_TIMEOUT;
        self.replace(
            &mut state,
            key,
            Record {
                public: PublicRecord {
                    binding: binding.clone(),
                    generation,
                    status: Status::Authenticating,
                    grant: Grant::default(),
                },
                material: None,
                refresh: Some((Arc::downgrade(&reservation), expires)),
            },
        )?;
        Ok(Pending {
            owner: self.owner.clone(),
            binding,
            generation,
            expires,
            kind: PendingKind::Login { reservation },
            grant: Grant::default(),
        })
    }
    pub fn begin_refresh(
        &self,
        binding: &Binding,
        generation: u64,
    ) -> Result<Pending, DomainError> {
        let mut state = self.lock()?;
        let r = self.current(&state, binding, generation)?;
        if r.public.status != Status::Connected || !matches!(binding.target, Target::OAuth { .. }) {
            return Err(DomainError::PermissionDenied);
        }
        if r.refresh
            .as_ref()
            .is_some_and(|(weak, expires)| weak.strong_count() > 0 && Instant::now() < *expires)
        {
            return Err(DomainError::Conflict);
        }
        let Some(Material::Bearer {
            refresh: Some(token),
            ..
        }) = &r.material
        else {
            return Err(DomainError::Unavailable(
                "MCP reauthentication required".into(),
            ));
        };
        let token = SecretBytes::new(token.expose().to_vec());
        let grant = r.public.grant.clone();
        let reservation = Arc::new(());
        let expires = Instant::now() + PENDING_TIMEOUT;
        state
            .records
            .get_mut(&binding.key())
            .ok_or(DomainError::NotFound)?
            .refresh = Some((Arc::downgrade(&reservation), expires));
        Ok(Pending {
            owner: self.owner.clone(),
            binding: binding.clone(),
            generation,
            expires,
            kind: PendingKind::Refresh { reservation, token },
            grant,
        })
    }
    /// Revalidate the native reservation after asynchronous work and before send.
    pub(crate) fn check_pending(&self, pending: &Pending) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&self.owner, &pending.owner) || Instant::now() >= pending.expires {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.lock()?;
        let r = self.current(&state, &pending.binding, pending.generation)?;
        let reservation = match &pending.kind {
            PendingKind::Login { reservation } if r.public.status == Status::Authenticating => {
                reservation
            }
            PendingKind::Refresh { reservation, .. } if r.public.status == Status::Connected => {
                reservation
            }
            _ => return Err(DomainError::Conflict),
        };
        if !r
            .refresh
            .as_ref()
            .and_then(|(weak, _)| weak.upgrade())
            .is_some_and(|active| Arc::ptr_eq(&active, reservation))
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    /// Retire only this reservation after a failed/abandoned private exchange.
    /// A newer login or logout always wins over this late cleanup.
    pub(crate) fn abort_pending(&self, pending: &Pending) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&self.owner, &pending.owner) {
            return Err(DomainError::PermissionDenied);
        }
        let mut state = self.lock()?;
        let r = self.current(&state, &pending.binding, pending.generation)?;
        let reservation = match &pending.kind {
            PendingKind::Login { reservation } | PendingKind::Refresh { reservation, .. } => {
                reservation
            }
        };
        if !r
            .refresh
            .as_ref()
            .and_then(|(weak, _)| weak.upgrade())
            .is_some_and(|active| Arc::ptr_eq(&active, reservation))
        {
            return Err(DomainError::Conflict);
        }
        let mut public = r.public.clone();
        public.generation = public
            .generation
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        public.status = Status::Revoked;
        public.grant = Grant::default();
        self.replace(
            &mut state,
            pending.binding.key(),
            Record {
                public,
                material: None,
                refresh: None,
            },
        )
    }
    pub fn commit(
        &self,
        pending: Pending,
        material: Material,
        grant: Grant,
        now_ms: u64,
    ) -> Result<u64, DomainError> {
        self.commit_reserved(&pending, material, grant, now_ms)
    }
    pub(crate) fn commit_reserved(
        &self,
        pending: &Pending,
        material: Material,
        grant: Grant,
        now_ms: u64,
    ) -> Result<u64, DomainError> {
        if !Arc::ptr_eq(&self.owner, &pending.owner) || Instant::now() >= pending.expires {
            return Err(DomainError::PermissionDenied);
        }
        material.validate(&pending.binding.target)?;
        grant.validate(&pending.binding.target)?;
        material
            .reject_echo(&serde_json::to_value(&grant).map_err(|_| invalid("credential grant"))?)?;
        if grant.expires_at_ms.is_some_and(|t| t <= now_ms) {
            return Err(invalid("expired credential response"));
        }
        let mut state = self.lock()?;
        let r = self.current(&state, &pending.binding, pending.generation)?;
        match &pending.kind {
            PendingKind::Login { reservation }
                if r.public.status == Status::Authenticating
                    && r.refresh
                        .as_ref()
                        .and_then(|(weak, _)| weak.upgrade())
                        .is_some_and(|active| Arc::ptr_eq(&active, reservation)) => {}
            PendingKind::Refresh { reservation, .. }
                if r.public.status == Status::Connected
                    && r.refresh
                        .as_ref()
                        .and_then(|(weak, _)| weak.upgrade())
                        .is_some_and(|active| Arc::ptr_eq(&active, reservation)) =>
            {
                // A refresh cannot silently switch account, widen scope or lose
                // grants. Changed consent must take the explicit login path.
                if grant.account != r.public.grant.account || grant.scopes != r.public.grant.scopes
                {
                    return Err(DomainError::PermissionDenied);
                }
            }
            _ => return Err(DomainError::Conflict),
        }
        let generation = pending
            .generation
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        self.replace(
            &mut state,
            pending.binding.key(),
            Record {
                public: PublicRecord {
                    binding: pending.binding.clone(),
                    generation,
                    status: Status::Connected,
                    grant,
                },
                material: Some(material),
                refresh: None,
            },
        )?;
        Ok(generation)
    }
    pub fn lease(
        self: &Arc<Self>,
        binding: &Binding,
        generation: u64,
        now_ms: u64,
    ) -> Result<CredentialLease, DomainError> {
        let lease = CredentialLease {
            vault: self.clone(),
            binding: binding.clone(),
            generation,
        };
        lease.check(now_ms)?;
        Ok(lease)
    }
    /// Local removal only. Upstream revocation requires its own confirmed result.
    /// Existing records retain a tombstone, so a late operation cannot win.
    pub fn logout(&self, binding: &Binding) -> Result<(), DomainError> {
        binding.validate()?;
        let mut state = self.lock()?;
        let key = binding.key();
        let Some(r) = state.records.get(&key) else {
            return Ok(());
        };
        if r.public.binding != *binding {
            return Err(DomainError::Conflict);
        }
        let mut public = r.public.clone();
        public.generation = public
            .generation
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        public.status = Status::Revoked;
        public.grant = Grant::default();
        self.replace(
            &mut state,
            key,
            Record {
                public,
                material: None,
                refresh: None,
            },
        )
    }
}
