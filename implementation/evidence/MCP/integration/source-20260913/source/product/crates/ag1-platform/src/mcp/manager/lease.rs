//! Native session connection ownership, separate from persistent availability.
use super::*;
use ag1_core::tools::{mcp::Kind, registry::Registry};

pub(super) struct Token {
    pub(super) closed: AtomicBool,
    pub(super) execution_scope: Option<ag1_core::tools::mcp::ExecutionScope>,
}
pub(super) fn same(a: Option<&Arc<Token>>, b: Option<&Arc<Token>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}
pub struct SessionLease {
    manager: Arc<Manager>,
    scope: CatalogScope,
    token: Arc<Token>,
    closing: Mutex<()>,
}
impl SessionLease {
    pub fn new(manager: Arc<Manager>, scope: CatalogScope) -> Result<Self, DomainError> {
        Self::with_owner(manager, scope, None)
    }
    pub(crate) fn for_session(
        manager: Arc<Manager>,
        scope: CatalogScope,
        owner: &ag1_core::tools::registry::Scope,
    ) -> Result<Self, DomainError> {
        Self::with_owner(
            manager,
            scope,
            Some(ag1_core::tools::mcp::ExecutionScope::from_native(owner)?),
        )
    }
    fn with_owner(
        manager: Arc<Manager>,
        scope: CatalogScope,
        execution_scope: Option<ag1_core::tools::mcp::ExecutionScope>,
    ) -> Result<Self, DomainError> {
        validate_scope(&scope)?;
        Ok(Self {
            manager,
            scope,
            closing: Mutex::new(()),
            token: Arc::new(Token {
                closed: AtomicBool::new(false),
                execution_scope,
            }),
        })
    }
    pub fn manager(&self) -> &Arc<Manager> {
        &self.manager
    }
    pub fn scope(&self) -> &CatalogScope {
        &self.scope
    }
    pub fn check(&self) -> Result<(), DomainError> {
        if self.token.closed.load(Ordering::Acquire) {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    pub fn review(
        &self,
        server: &str,
        inspection_only: bool,
    ) -> Result<ConnectionReview, DomainError> {
        self.check()?;
        let mut review = self
            .manager
            .review(server, self.scope.clone(), inspection_only)?;
        review.lease = Some(self.token.clone());
        Ok(review)
    }
    pub fn review_trust(
        &self,
        connection: &str,
        revision: u64,
        kind: Kind,
        key: &str,
        rules: Vec<ag1_core::tools::mcp::trust::Rule>,
    ) -> Result<super::TrustReview, DomainError> {
        self.check()?;
        self.manager
            .review_trust_for(connection, revision, kind, key, rules, Some(&self.token))
    }
    pub fn review_trust_append(
        &self,
        connection: &str,
        revision: u64,
        kind: Kind,
        key: &str,
        rules: Vec<ag1_core::tools::mcp::trust::Rule>,
    ) -> Result<super::TrustReview, DomainError> {
        let review = self.review_trust(connection, revision, kind, key, rules)?;
        self.manager.append_trust_review(review)
    }
    pub fn save_trust(&self, review: super::TrustReview) -> Result<(), DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        if !same(review.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        self.manager.save_trust(review)
    }
    pub fn set_trust_mode(
        &self,
        id: &str,
        revision: u64,
        mode: ag1_core::tools::mcp::trust::TrustMode,
    ) -> Result<(), DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        self.manager.set_trust_mode(id, revision, &self.scope, mode)
    }
    pub fn revoke_trust(
        &self,
        id: &str,
        revision: u64,
        wire_name: &str,
    ) -> Result<(), DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        self.manager
            .revoke_trust(id, revision, &self.scope, wire_name)
    }
    pub fn review_private(
        &self,
        server: &str,
        revision: u64,
    ) -> Result<super::PrivateReview, DomainError> {
        self.check()?;
        let mut review = self.manager.review_private(server, revision)?;
        review.bind_session(self.token.clone());
        Ok(review)
    }
    pub fn enter_private(
        &self,
        review: super::PrivateReview,
        entry: &dyn super::PrivateEntry,
    ) -> Result<(), DomainError> {
        self.check()?;
        if !same(review.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        let material = entry.enter(
            review.request(),
            &crate::resource_fs::ScanControl {
                cancel: &self.token.closed,
                deadline: review.expires,
            },
        )?;
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        self.manager.save_private(review, material)
    }
    #[cfg(target_os = "linux")]
    pub fn review_oauth(
        &self,
        server: &str,
        revision: u64,
    ) -> Result<super::OAuthReview, DomainError> {
        self.check()?;
        let mut review = self.manager.review_oauth(server, revision)?;
        review.bind_session(self.token.clone());
        Ok(review)
    }
    #[cfg(target_os = "linux")]
    pub fn enter_oauth(
        &self,
        review: super::OAuthReview,
        refresh: bool,
        entry: &dyn super::OAuthEntry,
    ) -> Result<(), DomainError> {
        self.check()?;
        if !same(review.0.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        let deadline = review.0.expires;
        entry.enter(
            self,
            review,
            refresh,
            &crate::resource_fs::ScanControl {
                cancel: &self.token.closed,
                deadline,
            },
        )?;
        self.check()
    }
    #[cfg(target_os = "linux")]
    pub fn begin_oauth(
        &self,
        review: super::OAuthReview,
        auth: crate::mcp::oauth::Authorization,
        registration: crate::mcp::oauth::login::ClientRegistration,
    ) -> Result<super::OAuthAttempt, DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        if !same(review.0.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        self.manager.begin_oauth(review, auth, registration)
    }
    #[cfg(target_os = "linux")]
    pub fn begin_oauth_refresh(
        &self,
        review: super::OAuthReview,
    ) -> Result<super::OAuthAttempt, DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        if !same(review.0.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        self.manager.begin_oauth_refresh(review)
    }
    #[cfg(target_os = "linux")]
    pub fn finish_oauth(
        &self,
        attempt: super::OAuthAttempt,
        tokens: crate::mcp::oauth::token::Tokens,
    ) -> Result<(), DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.check()?;
        if !same(attempt.session(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        self.manager.finish_oauth(attempt, tokens)
    }
    pub fn connect(
        &self,
        review: ConnectionReview,
        consent: ConnectionConsent,
    ) -> Result<String, DomainError> {
        self.check()?;
        if !same(review.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        self.manager.connect(review, consent)
    }
    pub fn connections(&self) -> Result<Vec<ConnectionStatus>, DomainError> {
        let state = self.manager.state()?;
        state
            .live
            .values()
            .filter(|w| same(w.control.lease.as_ref(), Some(&self.token)))
            .map(|w| {
                w.control
                    .view
                    .lock()
                    .map(|v| v.status.clone())
                    .map_err(|_| DomainError::Conflict)
            })
            .collect()
    }
    fn owns(&self, connection: &str) -> Result<(), DomainError> {
        self.check()?;
        let state = self.manager.state()?;
        let worker = state.live.get(connection).ok_or(DomainError::NotFound)?;
        if !same(worker.control.lease.as_ref(), Some(&self.token)) {
            return Err(DomainError::PermissionDenied);
        }
        Ok(())
    }
    pub fn inspect(&self, connection: &str) -> Result<Arc<ConnectedCatalog>, DomainError> {
        self.owns(connection)?;
        self.manager.inspect_catalog(connection, &self.scope)
    }
    pub(crate) fn definition(
        &self,
        connection: &str,
        kind: Kind,
        key: &str,
    ) -> Result<ag1_core::tools::mcp::SelectedDefinition, DomainError> {
        let definition = self
            .inspect(connection)?
            .definition(kind, key)?
            .for_execution_scope(self.token.execution_scope.as_ref())?;
        let state = self.manager.state()?;
        let worker = state.live.get(connection).ok_or(DomainError::Conflict)?;
        let view = worker
            .control
            .view
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if !same(worker.control.lease.as_ref(), Some(&self.token))
            || !view.catalog.as_ref().is_some_and(|c| {
                c.matches(definition.identity(), self.token.execution_scope.as_ref())
            })
        {
            return Err(DomainError::Conflict);
        }
        let record = state
            .records
            .servers
            .get(&view.status.server_id)
            .ok_or(DomainError::Conflict)?;
        worker.control.apply_policy(record, definition)
    }
    pub fn disconnect(&self, connection: &str) -> Result<(), DomainError> {
        self.owns(connection)?;
        self.manager.disconnect(connection)?;
        self.manager.wait_disconnected(connection)
    }
    pub fn search(
        &self,
        kind: Kind,
        query: &str,
        limit: usize,
    ) -> Result<CapabilityMatches, DomainError> {
        self.check()?;
        self.manager
            .search_capabilities_for(&self.scope, kind, query, limit, Some(&self.token))
    }
    pub fn select(
        &self,
        registry: &mut Registry,
        selected: &[SelectedCapability],
    ) -> Result<Selection, DomainError> {
        self.check()?;
        self.manager.select_capabilities_for(
            registry,
            self.scope.clone(),
            selected,
            Some(&self.token),
        )
    }
    /// Called on the native blocking worker during session retirement. It never
    /// cancels another session's connection, even for the same server and scope.
    pub fn close(&self) -> Result<(), DomainError> {
        let _closing = self.closing.lock().map_err(|_| DomainError::Conflict)?;
        self.token.closed.store(true, Ordering::Release);
        // Take custody under the manager lock so another session's reaper
        // cannot remove a worker between cancellation and its required join.
        let workers = {
            let mut state = self
                .manager
                .state
                .lock()
                .map_err(|_| DomainError::Conflict)?;
            let ids: Vec<_> = state
                .live
                .iter()
                .filter(|(_, w)| same(w.control.lease.as_ref(), Some(&self.token)))
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| state.live.remove(&id))
                .inspect(|worker| worker.control.cancel())
                .collect::<Vec<_>>()
        };
        let mut error = None;
        for mut worker in workers {
            if let Err(e) = worker.join() {
                error.get_or_insert(e);
            }
            let status = worker.control.view.lock().map(|view| view.status.clone());
            match status {
                Ok(status) => match self.manager.state.lock() {
                    Ok(mut state) => remember(&mut state, status),
                    Err(_) => {
                        error.get_or_insert(DomainError::Conflict);
                    }
                },
                Err(_) => {
                    error.get_or_insert(DomainError::Conflict);
                }
            }
        }
        if let Some(error) = error {
            return Err(error);
        }
        Ok(())
    }
}
impl Drop for SessionLease {
    fn drop(&mut self) {
        self.token.closed.store(true, Ordering::Release);
        // Explicit retirement drains; drop also revokes abandoned preparations.
        if let Ok(connections) = self.connections() {
            for status in connections {
                let _ = self.manager.disconnect(&status.id);
            }
        }
    }
}
