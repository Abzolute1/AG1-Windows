//! OAuth management uses the same native review, definition CAS and session
//! owner as private entry. Network/browser work never holds the manager lock.
use super::*;
use crate::{
    mcp::{
        auth::{Binding, Pending, Target, Vault},
        oauth::{
            Authorization, Network,
            login::{Browser, ClientRegistration},
            token::Tokens,
        },
    },
    provider::http::CancelToken,
};
use std::{future::Future, sync::Weak};

/// App-owned native consent/browser coordinator. Server or renderer data cannot
/// implement this port or supply authorization codes through it.
pub trait OAuthEntry: Send + Sync {
    fn enter(
        &self,
        session: &SessionLease,
        review: OAuthReview,
        refresh: bool,
        control: &crate::resource_fs::ScanControl<'_>,
    ) -> Result<(), DomainError>;
}

pub struct OAuthReview(pub(super) PrivateReview);
impl OAuthReview {
    pub fn request(&self) -> &PrivateRequest {
        self.0.request()
    }
    pub(super) fn bind_session(&mut self, token: Arc<lease::Token>) {
        self.0.bind_session(token);
    }
}
/// Native one-use attempt. Its private reservation is never a renderer DTO.
pub struct OAuthAttempt {
    manager: Weak<Manager>,
    review: PrivateReview,
    record: ServerRecord,
    binding: Binding,
    vault: Arc<Vault>,
    pending: Option<Pending>,
    login: Option<(Authorization, ClientRegistration)>,
}
impl Drop for OAuthAttempt {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            let _ = self.vault.abort_pending(pending);
        }
    }
}
impl OAuthAttempt {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn revision(&self) -> u64 {
        self.record.revision
    }
    pub(super) fn session(&self) -> Option<&Arc<lease::Token>> {
        self.review.lease.as_ref()
    }
    pub fn check(&self) -> Result<(), DomainError> {
        let manager = self
            .manager
            .upgrade()
            .ok_or(DomainError::PermissionDenied)?;
        manager.check_oauth_attempt(self)?;
        if let Some(pending) = &self.pending {
            self.vault.check_pending(pending)?;
        }
        Ok(())
    }
    async fn watch<T>(
        &self,
        future: impl Future<Output = Result<T, DomainError>>,
    ) -> Result<T, DomainError> {
        self.check()?;
        tokio::pin!(future);
        let mut ticker = tokio::time::interval(Duration::from_millis(25));
        loop {
            tokio::select! {biased;
                _=ticker.tick()=>self.check()?,
                result=&mut future=>{self.check()?;return result;}
            }
        }
    }
    /// Native UI supplies the browser port only after showing scoped consent.
    /// Returns the still-bound attempt alongside private tokens for final CAS.
    pub async fn perform(
        mut self,
        network: &Network,
        browser: Option<&dyn Browser>,
        cancel: &mut CancelToken,
    ) -> Result<(Self, Tokens), DomainError> {
        self.check()?;
        let tokens = if let Some((auth, registration)) = self.login.take() {
            let browser = browser.ok_or_else(|| invalid("OAuth browser port missing"))?;
            let code = self
                .watch(async {
                    let mut login = network
                        .begin_login(auth, registration, self.binding.clone(), cancel)
                        .await?;
                    self.check()?;
                    login.open(browser, cancel)?;
                    login.wait(cancel).await
                })
                .await?;
            let pending = self.pending.take().ok_or(DomainError::Conflict)?;
            self.watch(network.exchange(code, self.vault.clone(), pending, cancel))
                .await?
        } else {
            let pending = self.pending.take().ok_or(DomainError::Conflict)?;
            self.watch(network.refresh(self.vault.clone(), pending, cancel))
                .await?
        };
        Ok((self, tokens))
    }
}
impl Manager {
    pub fn review_oauth(&self, id: &str, revision: u64) -> Result<OAuthReview, DomainError> {
        let review = self.review_private(id, revision)?;
        if !matches!(
            &review.record.draft.configuration,
            Configuration::Http {
                loopback_development: false,
                ..
            }
        ) {
            return Err(DomainError::Unavailable(
                "MCP OAuth requires an HTTPS remote server".into(),
            ));
        }
        Ok(OAuthReview(review))
    }
    fn check_oauth_review(&self, review: &PrivateReview) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&self.owner, &review.owner)
            || Instant::now() >= review.expires
            || review
                .lease
                .as_ref()
                .is_some_and(|l| l.closed.load(Ordering::Acquire))
        {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    fn check_oauth_attempt(&self, attempt: &OAuthAttempt) -> Result<(), DomainError> {
        self.check_oauth_review(&attempt.review)?;
        if !Arc::ptr_eq(self.vault()?, &attempt.vault) {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.state()?;
        let current = state
            .records
            .servers
            .get(&attempt.record.id)
            .ok_or(DomainError::NotFound)?;
        if current.revision != attempt.record.revision {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    /// Call only after native metadata/issuer/client/scope consent. This retires
    /// existing server admissions and reserves credentials, without activation.
    pub fn begin_oauth(
        self: &Arc<Self>,
        review: OAuthReview,
        auth: Authorization,
        registration: ClientRegistration,
    ) -> Result<OAuthAttempt, DomainError> {
        let review = review.0;
        self.check_oauth_review(&review)?;
        auth.check_code_flow()?;
        if registration.issuer() != auth.view().issuer
            || !auth.view().token_auth_methods.iter().any(|m| m == "none")
        {
            return Err(DomainError::Unavailable(
                "MCP OAuth public client registration does not match the selected issuer".into(),
            ));
        }
        let Configuration::Http {
            url,
            loopback_development: false,
        } = &review.record.draft.configuration
        else {
            return Err(DomainError::PermissionDenied);
        };
        if url != &auth.resource().endpoint {
            return Err(DomainError::PermissionDenied);
        }
        let mut state = self.state()?;
        self.check_oauth_review(&review)?;
        let current = state
            .records
            .servers
            .get(&review.record.id)
            .ok_or(DomainError::NotFound)?;
        if current.revision != review.record.revision {
            return Err(DomainError::Conflict);
        }
        let mut next = current.clone();
        next.revision = next.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        next.config_revision = next.revision;
        next.draft.auth_profile = Some(review.request.profile_id.clone());
        let mut binding = credentials::credential_binding(&next, None)?;
        binding.target = Target::OAuth {
            endpoint: url.clone(),
            resource: auth.resource().resource.clone(),
            issuer: auth.view().issuer.clone(),
            token_endpoint: auth.view().token_endpoint.clone(),
            client_id: registration.client_id().into(),
        };
        binding.validate()?;
        self.cancel_server(&state, &next.id)?;
        let mut records = state.records.clone();
        records.servers.insert(next.id.clone(), next.clone());
        self.commit(&mut state, records)?;
        let vault = self.vault()?.clone();
        let pending = vault.begin_login(binding.clone())?;
        Ok(OAuthAttempt {
            manager: Arc::downgrade(self),
            review,
            record: next,
            binding,
            vault,
            pending: Some(pending),
            login: Some((auth, registration)),
        })
    }
    /// Refresh is explicit and shares the vault reservation across sessions.
    /// Existing connections are retired; completion does not reconnect or replay.
    pub fn begin_oauth_refresh(
        self: &Arc<Self>,
        review: OAuthReview,
    ) -> Result<OAuthAttempt, DomainError> {
        let review = review.0;
        self.check_oauth_review(&review)?;
        let mut state = self.state()?;
        self.check_oauth_review(&review)?;
        let current = state
            .records
            .servers
            .get(&review.record.id)
            .ok_or(DomainError::NotFound)?;
        if current.revision != review.record.revision {
            return Err(DomainError::Conflict);
        }
        let public = self.bound_credential(current, None)?;
        if !matches!(&public.binding.target, Target::OAuth { .. }) {
            return Err(DomainError::PermissionDenied);
        }
        let vault = self.vault()?.clone();
        let pending = vault.begin_refresh(&public.binding, public.generation)?;
        let mut next = current.clone();
        next.revision = next.revision.checked_add(1).ok_or(DomainError::Conflict)?;
        next.config_revision = current.configuration_revision();
        self.cancel_server(&state, &next.id)?;
        let mut records = state.records.clone();
        records.servers.insert(next.id.clone(), next.clone());
        self.commit(&mut state, records)?;
        Ok(OAuthAttempt {
            manager: Arc::downgrade(self),
            review,
            record: next,
            binding: public.binding,
            vault,
            pending: Some(pending),
            login: None,
        })
    }
    pub fn finish_oauth(&self, attempt: OAuthAttempt, tokens: Tokens) -> Result<(), DomainError> {
        self.check_oauth_review(&attempt.review)?;
        if !tokens.belongs_to(self.vault()?) || tokens.binding() != &attempt.binding {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.state()?;
        self.check_oauth_review(&attempt.review)?;
        let current = state
            .records
            .servers
            .get(&attempt.record.id)
            .ok_or(DomainError::NotFound)?;
        if current.revision != attempt.record.revision {
            return Err(DomainError::Conflict);
        }
        if self.bound_credential(current, None)?.binding != attempt.binding {
            return Err(DomainError::Conflict);
        }
        self.cancel_server(&state, &current.id)?;
        tokens.commit(credentials::now()?)?;
        Ok(())
    }
}
