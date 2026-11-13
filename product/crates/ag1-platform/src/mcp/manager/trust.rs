//! Durable scope policy, resolved only through the current native connection.
//! Stored rules are data. A live, revocable core witness is issued only after
//! matching the native target, credential generation and exact catalog entry.
use super::*;
use ag1_core::tools::mcp::{
    Identity, Scope as CoreScope, SelectedDefinition,
    trust::{HostRisk, ReviewedPolicy, Rule, TrustMode, WeakReviewedPolicy},
};
use serde_json::json;

const MAX_SCOPE_POLICIES: usize = 128;
const MAX_GRANTS: usize = 64;
const MAX_LIVE_WITNESSES: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityGrant {
    pub identity: Identity,
    pub target_digest: String,
    pub rules: Vec<Rule>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopePolicy {
    pub scope: CoreScope,
    pub revision: u64,
    pub mode: TrustMode,
    pub grants: Vec<CapabilityGrant>,
}
fn digest(value: &impl Serialize) -> Result<String, DomainError> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| invalid("trust encoding"))?)
    ))
}
pub(super) fn target_digest(
    record: &ServerRecord,
    local: Option<&PinnedLocal>,
) -> Result<String, DomainError> {
    // The definition contains the exact public endpoint/launch/provenance. The
    // executable and cwd pins distinguish same-path replacement on reconnect.
    // Dependencies are mutable host-account code, explicitly disclosed at launch.
    digest(
        &json!({"draft":record.draft,"localAccess":record.local_access,"local":local.map(|p|json!({
            "executableSha256":p.executable_hash,"executableIdentity":p.executable_identity,"cwdIdentity":p.cwd_identity
        }))}),
    )
}
fn scope(scope: &CatalogScope) -> CoreScope {
    match scope {
        CatalogScope::GeneralChat => CoreScope::GeneralChat,
        CatalogScope::Workspace(id) => CoreScope::Workspace(id.clone()),
    }
}
fn same_capability(a: &Identity, b: &Identity) -> bool {
    let normalize = |i: &Identity| {
        let mut i = i.clone();
        i.execution_scope = None;
        i.binding.connection_generation = 1;
        i
    };
    normalize(a) == normalize(b)
}
pub(super) fn validate(record: &ServerRecord) -> Result<(), DomainError> {
    if record.trust.len() > MAX_SCOPE_POLICIES
        || serde_json::to_vec(&record.trust)
            .map_err(|_| invalid("trust encoding"))?
            .len()
            > 256 * 1024
    {
        return Err(DomainError::LimitExceeded("MCP scoped trust policy".into()));
    }
    let mut scopes = std::collections::BTreeSet::new();
    for policy in &record.trust {
        if policy.revision == 0
            || policy.revision > record.revision
            || policy.grants.len() > MAX_GRANTS
            || !scopes
                .insert(serde_json::to_string(&policy.scope).map_err(|_| invalid("trust scope"))?)
        {
            return Err(invalid("trust policy identity"));
        }
        validate_scope(&match &policy.scope {
            CoreScope::GeneralChat => CatalogScope::GeneralChat,
            CoreScope::Workspace(id) => CatalogScope::Workspace(id.clone()),
        })?;
        let mut keys = std::collections::BTreeSet::new();
        for grant in &policy.grants {
            grant.identity.validate()?;
            if grant.identity.execution_scope.is_some()
                || grant.identity.binding.server_id != record.id
                || grant.identity.binding.scope != policy.scope
                || grant.target_digest.len() != 64
                || !grant
                    .target_digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || grant.rules.is_empty()
                || grant.rules.len() > 32
                || serde_json::to_vec(&grant.rules)
                    .map_err(|_| invalid("trust encoding"))?
                    .len()
                    > 64 * 1024
                || !keys.insert(grant.identity.wire_name())
            {
                return Err(invalid("trust capability binding"));
            }
            for rule in &grant.rules {
                rule.validate()?;
            }
        }
    }
    Ok(())
}

/// Exact native review, owned by one manager/session and valid for five minutes.
/// It is not cloneable or deserializable and does not itself permit execution.
pub struct TrustReview {
    owner: Arc<()>,
    pending: Arc<AtomicUsize>,
    record_revision: u64,
    connection: String,
    scope: CatalogScope,
    pub(super) lease: Option<Arc<lease::Token>>,
    expires: Instant,
    grant: CapabilityGrant,
    display: Value,
}
impl TrustReview {
    pub fn display(&self) -> &Value {
        &self.display
    }
}
impl Drop for TrustReview {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Control {
    pub(super) fn revoke_policies(&self) {
        let mut cache = self.policies.lock().unwrap_or_else(|p| p.into_inner());
        for policy in cache.values().filter_map(WeakReviewedPolicy::upgrade) {
            policy.revoke();
        }
        cache.clear();
    }
    pub(super) fn apply_policy(
        &self,
        record: &ServerRecord,
        selected: SelectedDefinition,
    ) -> Result<SelectedDefinition, DomainError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Conflict);
        }
        let Some(policy) = record
            .trust
            .iter()
            .find(|p| p.scope == selected.identity().binding.scope)
        else {
            return Ok(selected);
        };
        let grant = policy.grants.iter().find(|g| {
            g.target_digest == self.target_digest
                && same_capability(&g.identity, selected.identity())
        });
        if grant.is_none() && policy.mode == TrustMode::Restricted {
            return Ok(selected);
        }
        let key = digest(&json!([
            selected.identity(),
            policy.revision,
            self.target_digest
        ]))?;
        let mut cache = self.policies.lock().map_err(|_| DomainError::Conflict)?;
        cache.retain(|_, p| p.upgrade().is_some());
        let witness = if let Some(witness) = cache.get(&key).and_then(WeakReviewedPolicy::upgrade) {
            witness.check(selected.identity())?;
            witness
        } else {
            if cache.len() >= MAX_LIVE_WITNESSES {
                return Err(DomainError::LimitExceeded(
                    "MCP live policy witnesses".into(),
                ));
            }
            let witness = ReviewedPolicy::new(&selected, policy.revision, policy.mode, HostRisk::Unknown,
                "AG1 matched the native target, credential, scope and complete capability metadata to an explicit user-reviewed rule. Server annotations are assertions, not authority; remote behavior and mutable local dependencies are not guaranteed.".into(),
                grant.map(|g| g.rules.clone()).unwrap_or_default())?;
            cache.insert(key, witness.downgrade());
            witness
        };
        selected.with_reviewed_policy(witness)
    }
}
impl Manager {
    pub fn scope_policy(
        &self,
        id: &str,
        target_scope: &CatalogScope,
    ) -> Result<Option<ScopePolicy>, DomainError> {
        validate_scope(target_scope)?;
        let state = self.state()?;
        Ok(state
            .records
            .servers
            .get(id)
            .ok_or(DomainError::NotFound)?
            .trust
            .iter()
            .find(|p| p.scope == scope(target_scope))
            .cloned())
    }
    /// Scope mode grants no calls by itself. Restricting/removing policy retires
    /// only this scope's connections, preserving other scopes and credentials.
    pub fn set_trust_mode(
        &self,
        id: &str,
        expected: u64,
        target_scope: &CatalogScope,
        mode: TrustMode,
    ) -> Result<(), DomainError> {
        self.edit_trust(id, expected, target_scope, |policy| {
            policy.mode = mode;
            Ok(())
        })
    }
    pub fn revoke_trust(
        &self,
        id: &str,
        expected: u64,
        target_scope: &CatalogScope,
        wire_name: &str,
    ) -> Result<(), DomainError> {
        self.edit_trust(id, expected, target_scope, |policy| {
            let before = policy.grants.len();
            policy
                .grants
                .retain(|g| g.identity.wire_name() != wire_name);
            if before == policy.grants.len() {
                return Err(DomainError::NotFound);
            }
            Ok(())
        })
    }
    fn edit_trust(
        &self,
        id: &str,
        expected: u64,
        target_scope: &CatalogScope,
        edit: impl FnOnce(&mut ScopePolicy) -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        validate_scope(target_scope)?;
        let mut state = self.state()?;
        let mut next = state.records.clone();
        let record = next.servers.get_mut(id).ok_or(DomainError::NotFound)?;
        if record.revision != expected {
            return Err(DomainError::Conflict);
        }
        record.config_revision = record.configuration_revision();
        record.revision = expected.checked_add(1).ok_or(DomainError::Conflict)?;
        let target = scope(target_scope);
        if !record.trust.iter().any(|p| p.scope == target) {
            record.trust.push(ScopePolicy {
                scope: target.clone(),
                revision: record.revision,
                mode: TrustMode::Restricted,
                grants: Vec::new(),
            });
        }
        let policy = record
            .trust
            .iter_mut()
            .find(|p| p.scope == target)
            .ok_or(DomainError::Conflict)?;
        policy.revision = record.revision;
        edit(policy)?;
        validate(record)?;
        self.commit_trust(&mut state, next, id, target_scope)
    }
    fn commit_trust(
        &self,
        state: &mut State,
        next: Records,
        id: &str,
        target_scope: &CatalogScope,
    ) -> Result<(), DomainError> {
        // Validate storage bounds before retiring healthy work. On actual commit
        // failure the existing manager commit path poisons/retires the owner.
        let bytes = serde_json::to_vec(&next).map_err(|_| invalid("trust encoding"))?;
        decode_json(&bytes, Limits::default())?;
        for worker in state.live.values() {
            let status = worker
                .control
                .view
                .lock()
                .map_err(|_| DomainError::Conflict)?
                .status
                .clone();
            if status.server_id == id && &status.scope == target_scope {
                worker.control.cancel();
            }
        }
        self.commit(state, next)
    }
    pub fn review_trust(
        &self,
        connection: &str,
        expected: u64,
        kind: ag1_core::tools::mcp::Kind,
        key: &str,
        rules: Vec<Rule>,
    ) -> Result<TrustReview, DomainError> {
        self.review_trust_for(connection, expected, kind, key, rules, None)
    }
    pub(super) fn review_trust_for(
        &self,
        connection: &str,
        expected: u64,
        kind: ag1_core::tools::mcp::Kind,
        key: &str,
        rules: Vec<Rule>,
        token: Option<&Arc<lease::Token>>,
    ) -> Result<TrustReview, DomainError> {
        let state = self.state()?;
        let worker = state.live.get(connection).ok_or(DomainError::NotFound)?;
        if !lease::same(worker.control.lease.as_ref(), token) {
            return Err(DomainError::PermissionDenied);
        }
        let view = worker
            .control
            .view
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        let record = state
            .records
            .servers
            .get(&view.status.server_id)
            .ok_or(DomainError::NotFound)?;
        if record.revision != expected
            || worker.control.cancelled.load(Ordering::Acquire)
            || view.status.health != ConnectionHealth::Ready
        {
            return Err(DomainError::Conflict);
        }
        worker.control.credential_check()?;
        let catalog = view.catalog.as_ref().ok_or(DomainError::Conflict)?;
        if catalog.binding.config_revision != record.configuration_revision() {
            return Err(DomainError::Conflict);
        }
        let definition = catalog.definition(kind, key)?;
        let grant = CapabilityGrant {
            identity: definition.identity().clone(),
            target_digest: worker.control.target_digest.clone(),
            rules,
        };
        let mut prospective = record.clone();
        prospective.trust = vec![ScopePolicy {
            scope: scope(&view.status.scope),
            revision: expected,
            mode: TrustMode::Restricted,
            grants: vec![grant.clone()],
        }];
        validate(&prospective)?;
        let display = json!({"serverId":record.id,"serverName":record.draft.name,"configuration":record.draft.configuration,
            "scope":scope(&view.status.scope),"identity":grant.identity,"identityJson":serde_json::to_string_pretty(&grant.identity).map_err(|_|invalid("trust encoding"))?,"targetDigest":grant.target_digest,"rules":grant.rules,"rulesJson":serde_json::to_string_pretty(&grant.rules).map_err(|_|invalid("trust encoding"))?,
            "metadata":catalog.rows(kind).into_iter().find(|r|r[kind.key_field()]==key).ok_or(DomainError::NotFound)?,
            "hostClassification":HostRisk::for_call(definition.identity(), &json!({}))?,"basis":"AG1 verifies exact native target and schema bindings. Reviewed effects are user policy; server annotations do not prove behavior. Consequential operation/argument signals still require exact confirmation.",
            "localAccess":matches!(record.draft.configuration,Configuration::Local{..}).then_some(record.local_access.description())});
        self.reviews
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_REVIEWS).then_some(n + 1)
            })
            .map_err(|_| DomainError::LimitExceeded("MCP pending reviews".into()))?;
        Ok(TrustReview {
            owner: self.owner.clone(),
            pending: self.reviews.clone(),
            record_revision: expected,
            connection: connection.into(),
            scope: view.status.scope.clone(),
            lease: token.cloned(),
            expires: Instant::now() + Duration::from_secs(300),
            grant,
            display,
        })
    }
    pub(super) fn append_trust_review(
        &self,
        mut review: TrustReview,
    ) -> Result<TrustReview, DomainError> {
        if !Arc::ptr_eq(&review.owner, &self.owner) || Instant::now() >= review.expires {
            return Err(DomainError::PermissionDenied);
        }
        let state = self.state()?;
        let record = state
            .records
            .servers
            .get(&review.grant.identity.binding.server_id)
            .ok_or(DomainError::NotFound)?;
        if record.revision != review.record_revision {
            return Err(DomainError::Conflict);
        }
        let prior = record
            .trust
            .iter()
            .find(|p| p.scope == scope(&review.scope))
            .and_then(|p| {
                p.grants.iter().find(|g| {
                    g.target_digest == review.grant.target_digest
                        && same_capability(&g.identity, &review.grant.identity)
                })
            });
        if let Some(prior) = prior {
            let mut rules = prior.rules.clone();
            rules.append(&mut review.grant.rules);
            review.grant.rules = rules;
        }
        let mut prospective = record.clone();
        prospective.trust = vec![ScopePolicy {
            scope: scope(&review.scope),
            revision: record.revision,
            mode: TrustMode::Restricted,
            grants: vec![review.grant.clone()],
        }];
        validate(&prospective)?;
        review.display["rules"] = json!(review.grant.rules);
        review.display["rulesJson"] = json!(
            serde_json::to_string_pretty(&review.grant.rules)
                .map_err(|_| invalid("trust encoding"))?
        );
        review.display["existingRulesRetained"] = json!(prior.map_or(0, |g| g.rules.len()));
        Ok(review)
    }
    pub fn save_trust(&self, review: TrustReview) -> Result<(), DomainError> {
        if !Arc::ptr_eq(&review.owner, &self.owner)
            || Instant::now() >= review.expires
            || review
                .lease
                .as_ref()
                .is_some_and(|l| l.closed.load(Ordering::Acquire))
        {
            return Err(DomainError::PermissionDenied);
        }
        let mut state = self.state()?;
        let worker = state
            .live
            .get(&review.connection)
            .ok_or(DomainError::Conflict)?;
        let view = worker
            .control
            .view
            .lock()
            .map_err(|_| DomainError::Conflict)?;
        if view.status.health != ConnectionHealth::Ready
            || worker.control.cancelled.load(Ordering::Acquire)
            || worker.control.target_digest != review.grant.target_digest
            || view.status.scope != review.scope
            || !lease::same(worker.control.lease.as_ref(), review.lease.as_ref())
        {
            return Err(DomainError::Conflict);
        }
        worker.control.credential_check()?;
        let catalog = view.catalog.as_ref().ok_or(DomainError::Conflict)?;
        let current = catalog.definition(
            review.grant.identity.kind,
            &review.grant.identity.original_name,
        )?;
        if current.identity() != &review.grant.identity {
            return Err(DomainError::Conflict);
        }
        let id = review.grant.identity.binding.server_id.clone();
        let mut next = state.records.clone();
        let record = next.servers.get_mut(&id).ok_or(DomainError::NotFound)?;
        if record.revision != review.record_revision
            || record.configuration_revision() != catalog.binding.config_revision
        {
            return Err(DomainError::Conflict);
        }
        record.config_revision = record.configuration_revision();
        record.revision = record
            .revision
            .checked_add(1)
            .ok_or(DomainError::Conflict)?;
        let target = scope(&review.scope);
        if !record.trust.iter().any(|p| p.scope == target) {
            record.trust.push(ScopePolicy {
                scope: target.clone(),
                revision: record.revision,
                mode: TrustMode::Restricted,
                grants: Vec::new(),
            });
        }
        let policy = record
            .trust
            .iter_mut()
            .find(|p| p.scope == target)
            .ok_or(DomainError::Conflict)?;
        policy.revision = record.revision;
        policy
            .grants
            .retain(|g| g.identity.wire_name() != review.grant.identity.wire_name());
        policy.grants.push(review.grant.clone());
        validate(record)?;
        drop(view);
        self.commit_trust(&mut state, next, &id, &review.scope)
    }
}
