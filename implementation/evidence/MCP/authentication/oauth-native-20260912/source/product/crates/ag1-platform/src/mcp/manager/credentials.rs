//! Native private-entry review and manager/vault coordination. UI values contain
//! public target fields only; secret material arrives through the native port.
use super::*;
use crate::mcp::auth::{Binding, CredentialLease, Grant, Material, PublicRecord, Target, Vault};
use crate::resource_fs::ScanControl;

pub trait PrivateEntry: Send + Sync {
    fn enter(
        &self,
        request: &PrivateRequest,
        control: &ScanControl<'_>,
    ) -> Result<Material, DomainError>;
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivateRequest {
    pub server_id: String,
    pub server_name: String,
    pub profile_id: String,
    pub configuration: Configuration,
    pub executable_sha256: Option<String>,
}
/// Native review only. No Clone/Deserialize; the entry port gets a public copy.
pub struct PrivateReview {
    pending: Arc<AtomicUsize>,
    pub(super) owner: Arc<()>,
    pub(super) record: ServerRecord,
    local: Option<PinnedLocal>,
    pub(super) expires: Instant,
    pub(super) lease: Option<Arc<lease::Token>>,
    pub(super) request: PrivateRequest,
}
impl Drop for PrivateReview {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }
}
impl PrivateReview {
    pub fn request(&self) -> &PrivateRequest {
        &self.request
    }
    pub(super) fn bind_session(&mut self, token: Arc<lease::Token>) {
        self.lease = Some(token);
    }
}
pub(super) fn now() -> Result<u64, DomainError> {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| DomainError::Unavailable("MCP credential clock".into()))?
            .as_millis(),
    )
    .map_err(|_| DomainError::Unavailable("MCP credential clock".into()))
}
impl Manager {
    /// Compose only at native application startup, before sharing the manager.
    /// The vault must own a separate private account Store.
    pub fn with_vault(mut self, vault: Arc<Vault>) -> Result<Self, DomainError> {
        if self.vault.is_some() || !self.state()?.live.is_empty() {
            return Err(DomainError::Conflict);
        }
        self.vault = Some(vault);
        Ok(self)
    }
    pub(super) fn vault(&self) -> Result<&Arc<Vault>, DomainError> {
        self.vault.as_ref().ok_or_else(|| {
            DomainError::Unavailable("MCP private credential storage is unavailable".into())
        })
    }
    pub fn authentication(&self, id: &str) -> Result<Option<PublicRecord>, DomainError> {
        let state = self.state()?;
        let r = state.records.servers.get(id).ok_or(DomainError::NotFound)?;
        let Some(profile) = &r.draft.auth_profile else {
            return Ok(None);
        };
        let Some(vault) = &self.vault else {
            return Ok(None);
        };
        let http_binding = matches!(r.draft.configuration, Configuration::Http { .. })
            .then(|| credential_binding(r, None))
            .transpose()?;
        Ok(vault.records(now()?)?.into_iter().find(|p| {
            p.binding.server_id == id
                && &p.binding.profile_id == profile
                && p.binding.configuration_revision == r.configuration_revision()
                && http_binding
                    .as_ref()
                    .is_none_or(|b| binding_matches(b, &r.draft.configuration, &p.binding))
        }))
    }
    pub fn review_private(&self, id: &str, revision: u64) -> Result<PrivateReview, DomainError> {
        self.vault()?;
        let record = self
            .state()?
            .records
            .servers
            .get(id)
            .cloned()
            .ok_or(DomainError::NotFound)?;
        if record.revision != revision {
            return Err(DomainError::Conflict);
        }
        let local = match &record.draft.configuration {
            Configuration::Local {
                executable,
                cwd,
                secret_environment,
                ..
            } => {
                if secret_environment.is_empty() {
                    return Err(invalid(
                        "configure the exact private environment names first",
                    ));
                }
                Some(PinnedLocal::new(executable, cwd)?)
            }
            Configuration::Http { .. } => None,
        };
        let profile_id = record.draft.auth_profile.clone().unwrap_or(new_id()?);
        let request = PrivateRequest {
            server_id: record.id.clone(),
            server_name: record.draft.name.clone(),
            profile_id,
            configuration: record.draft.configuration.clone(),
            executable_sha256: local.as_ref().map(|p| p.executable_hash.clone()),
        };
        let mut prospective = record.clone();
        prospective.revision = prospective
            .revision
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        prospective.config_revision = prospective.revision;
        prospective.draft.auth_profile = Some(request.profile_id.clone());
        credential_binding(&prospective, local.as_ref())?;
        self.reviews
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_REVIEWS).then_some(n + 1)
            })
            .map_err(|_| DomainError::LimitExceeded("MCP pending reviews".into()))?;
        Ok(PrivateReview {
            pending: self.reviews.clone(),
            owner: self.owner.clone(),
            record,
            local,
            expires: Instant::now() + Duration::from_secs(600),
            lease: None,
            request,
        })
    }
    /// The only material argument is native/private. Nothing here starts a peer,
    /// makes an HTTP request, changes availability or approves a model call.
    pub fn save_private(
        &self,
        review: PrivateReview,
        material: Material,
    ) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&self.owner, &review.owner)
            || Instant::now() >= review.expires
            || review
                .lease
                .as_ref()
                .is_some_and(|l| l.closed.load(Ordering::Acquire))
        {
            return Err(DomainError::PermissionDenied);
        }
        if let Configuration::Local {
            executable, cwd, ..
        } = &review.record.draft.configuration
        {
            review
                .local
                .as_ref()
                .ok_or(DomainError::Conflict)?
                .revalidate(executable, cwd)?;
        }
        let mut state = self.state()?;
        let current = state
            .records
            .servers
            .get(&review.record.id)
            .ok_or(DomainError::NotFound)?;
        if current.revision != review.record.revision
            || review
                .lease
                .as_ref()
                .is_some_and(|l| l.closed.load(Ordering::Acquire))
        {
            return Err(DomainError::Conflict);
        }
        let mut next = current.clone();
        next.revision = next.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        next.config_revision = next.revision;
        next.draft.auth_profile = Some(review.request.profile_id.clone());
        let binding = credential_binding(&next, review.local.as_ref())?;
        material.validate(&binding.target)?;
        local_private_budget(&next.draft.configuration, &material)?;
        self.cancel_server(&state, &next.id)?;
        // Public config commits first. A later private failure leaves a missing/
        // revoked credential profile, never an unauthenticated fallback. On an
        // uncertain public commit the manager is poisoned before secret storage.
        let mut records = state.records.clone();
        records.servers.insert(next.id.clone(), next);
        self.commit(&mut state, records)?;
        let vault = self.vault()?;
        let pending = vault.begin_login(binding)?;
        vault.commit(pending, material, Grant::default(), now()?)?;
        Ok(())
    }
    pub fn logout(&self, id: &str, revision: u64) -> Result<(), DomainError> {
        let mut state = self.state()?;
        let current = state.records.servers.get(id).ok_or(DomainError::NotFound)?;
        if current.revision != revision {
            return Err(DomainError::Conflict);
        }
        let mut next = current.clone();
        next.config_revision = current.configuration_revision();
        next.revision = next.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        self.cancel_server(&state, id)?;
        self.revoke_credentials(id)?;
        // Also fence native private-entry dialogs that have not reached the
        // vault yet. Their old management revision cannot log in after logout.
        let mut records = state.records.clone();
        records.servers.insert(id.into(), next);
        self.commit(&mut state, records)
    }
    pub(super) fn cancel_server(&self, state: &State, id: &str) -> Result<(), DomainError> {
        for worker in state.live.values() {
            let matches = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .status
                .server_id
                == id;
            if matches {
                worker.control.cancel();
            }
        }
        Ok(())
    }
    pub(super) fn revoke_credentials(&self, id: &str) -> Result<(), DomainError> {
        if let Some(vault) = &self.vault {
            for public in vault
                .records(now()?)?
                .into_iter()
                .filter(|r| r.binding.server_id == id)
            {
                vault.logout(&public.binding)?;
            }
        }
        Ok(())
    }
    pub(super) fn credential(
        &self,
        record: &ServerRecord,
        local: Option<&PinnedLocal>,
    ) -> Result<Option<Arc<CredentialLease>>, DomainError> {
        if record.draft.auth_profile.is_none() {
            if matches!(&record.draft.configuration,Configuration::Local {secret_environment,..} if !secret_environment.is_empty())
            {
                return Err(DomainError::Unavailable(
                    "Enter the configured MCP private values before connecting".into(),
                ));
            }
            return Ok(None);
        }
        let vault = self.vault()?;
        let public = self.bound_credential(record, local)?;
        Ok(Some(Arc::new(vault.lease(
            &public.binding,
            public.generation,
            now()?,
        )?)))
    }
    pub(super) fn bound_credential(
        &self,
        record: &ServerRecord,
        local: Option<&PinnedLocal>,
    ) -> Result<PublicRecord, DomainError> {
        let binding = credential_binding(record, local)?;
        self.vault()?
            .records(now()?)?
            .into_iter()
            .find(|p| binding_matches(&binding, &record.draft.configuration, &p.binding))
            .ok_or_else(|| {
                DomainError::Unavailable(
                    "MCP reauthentication required for this configuration".into(),
                )
            })
    }
}
fn binding_matches(base: &Binding, configuration: &Configuration, actual: &Binding) -> bool {
    let mut expected = base.clone();
    if let (
        Configuration::Http {
            url,
            loopback_development: false,
        },
        Target::OAuth { endpoint, .. },
    ) = (configuration, &actual.target)
    {
        if endpoint != url {
            return false;
        }
        // Only the trusted native vault supplies issuer/client/token destination.
        expected.target = actual.target.clone();
    }
    actual == &expected
}
pub(super) fn credential_binding(
    record: &ServerRecord,
    local: Option<&PinnedLocal>,
) -> Result<Binding, DomainError> {
    let target = match &record.draft.configuration {
        Configuration::Http {
            url,
            loopback_development,
        } => Target::ManualBearer {
            endpoint: url.clone(),
            loopback_development: *loopback_development,
        },
        Configuration::Local {
            secret_environment, ..
        } => Target::Local {
            executable_sha256: local.ok_or(DomainError::Conflict)?.executable_hash.clone(),
            environment_names: secret_environment.clone(),
        },
    };
    let local_identity = local.map(|p| (p.executable_identity, p.cwd_identity));
    let raw=serde_json::to_vec(&serde_json::json!({"configuration":record.draft.configuration,"localIdentity":local_identity})).map_err(|_|invalid("private configuration identity"))?;
    let binding = Binding {
        server_id: record.id.clone(),
        profile_id: record
            .draft
            .auth_profile
            .clone()
            .ok_or(DomainError::PermissionDenied)?,
        configuration_revision: record.configuration_revision(),
        configuration_sha256: format!("{:x}", Sha256::digest(&raw)),
        target,
    };
    binding.validate()?;
    Ok(binding)
}

fn local_private_budget(
    configuration: &Configuration,
    material: &Material,
) -> Result<(), DomainError> {
    let Configuration::Local {
        arguments,
        public_environment,
        ..
    } = configuration
    else {
        return Ok(());
    };
    let Material::Environment(values) = material else {
        return Err(DomainError::PermissionDenied);
    };
    #[derive(Serialize)]
    struct Spec<'a> {
        executable: &'a str,
        argv: &'a [String],
        cwd: &'a str,
        env: BTreeMap<&'a str, &'a str>,
        timeout_ms: u64,
        grace_ms: u64,
        resident: bool,
    }
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            if self.0 > 64 * 1024 {
                return Err(std::io::Error::other("process spec quota"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut env: BTreeMap<&str, &str> = public_environment
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for (name, value) in values {
        env.insert(
            name,
            std::str::from_utf8(value.expose())
                .map_err(|_| invalid("private environment encoding"))?,
        );
    }
    // Conservative room for both actual /proc/pid/fd paths. Count JSON escaping
    // without allocating another credential-bearing serialized buffer.
    let descriptor = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
    let spec = Spec {
        executable: descriptor,
        argv: arguments,
        cwd: descriptor,
        env,
        timeout_ms: 600_000,
        grace_ms: 100,
        resident: false,
    };
    serde_json::to_writer(Count(0),&spec).map_err(|_|DomainError::LimitExceeded("MCP local configuration and private values exceed the 64 KiB process specification limit".into()))
}
