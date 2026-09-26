//! One-use approval capabilities bound to prepared arguments, session and source revisions.
use super::{
    policy::{self, Policy},
    registry::{Registry, Scope, ValidatedCall},
};
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub id: String,
    pub operation: String,
    pub argument_hash: String,
    pub workspace: String,
    pub session: String,
    pub epoch: u64,
    pub revision: u64,
    pub registry_revision: u64,
    pub policy_revision: u64,
    pub target_revision: String,
    pub expires_ms: u64,
}
#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    pub binding: Binding,
    pub display: Value,
}
/// Current host-owned values, independent of the client's echoed approval binding.
#[derive(Clone, Copy)]
pub struct ExecutionFence<'a> {
    pub scope: &'a Scope,
    pub policy_revision: u64,
    pub target_revision: &'a str,
    pub now: u64,
}
pub struct PreflightContext<'a> {
    pub scope: &'a Scope,
    pub policy: &'a Policy,
    pub policy_revision: u64,
    pub target_revision: &'a str,
    pub now: u64,
    pub ttl: u64,
    pub untrusted_extensions: bool,
}
/// No Clone/Deserialize: execution consumes the authorization once.
pub struct AuthorizedCall {
    call: ValidatedCall,
    scope: Scope,
    registry_revision: u64,
    policy_revision: u64,
    target_revision: String,
    expires_ms: Option<u64>,
}
impl AuthorizedCall {
    pub fn call(&self) -> &ValidatedCall {
        &self.call
    }
    pub fn target_revision(&self) -> &str {
        &self.target_revision
    }
    pub fn check(&self, registry: &Registry, fence: ExecutionFence<'_>) -> Result<(), DomainError> {
        let ExecutionFence {
            scope,
            policy_revision,
            target_revision,
            now,
        } = fence;
        registry.validate_current(&self.call)?;
        if let Some(identity) = self.call.mcp_identity() {
            identity.check_scope(scope)?;
            if identity.target_revision() != target_revision { return Err(DomainError::Conflict); }
        }
        if self.scope != *scope
            || self.registry_revision != registry.revision()
            || self.policy_revision != policy_revision
            || self.target_revision != target_revision
            || self.expires_ms.is_some_and(|t| now >= t)
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub fn consume(
        self,
        registry: &Registry,
        fence: ExecutionFence<'_>,
    ) -> Result<ValidatedCall, DomainError> {
        self.check(registry, fence)?;
        Ok(self.call)
    }
}
struct Pending {
    call: ValidatedCall,
    scope: Scope,
    request: ApprovalRequest,
}
pub enum Preflight {
    Ready(AuthorizedCall),
    Approval(ApprovalRequest),
    Rejected(String),
}
pub struct Authorizer {
    pending: BTreeMap<String, Pending>,
    serial: u64,
    incarnation: String,
}
impl Authorizer {
    /// The parent allocates a fresh incarnation ID on host restart; a client cannot choose it.
    pub fn new(incarnation: &str) -> Result<Self, DomainError> {
        crate::OperationId::new(incarnation)?;
        Ok(Self {
            pending: BTreeMap::new(),
            serial: 0,
            incarnation: incarnation.into(),
        })
    }

    pub fn pending(&self) -> Vec<ApprovalRequest> {
        self.pending.values().map(|p| p.request.clone()).collect()
    }
    pub fn cancel(&mut self, operation: &str) {
        self.pending.remove(operation);
    }
    pub fn expire(&mut self, now: u64) {
        self.pending
            .retain(|_, p| now < p.request.binding.expires_ms);
    }
    pub fn preflight(
        &mut self,
        call: ValidatedCall,
        context: PreflightContext<'_>,
    ) -> Result<Preflight, DomainError> {
        self.preflight_in_folders(call,context,&[])
    }
    /// Folder grants are host-selected and must share the scope's lifetime fence.
    pub fn preflight_in_folders(&mut self, call: ValidatedCall, context: PreflightContext<'_>, folders: &[String]) -> Result<Preflight, DomainError> {
        let PreflightContext {
            scope,
            policy,
            policy_revision,
            target_revision,
            now,
            ttl,
            untrusted_extensions,
        } = context;
        if target_revision.len() > 512 || ttl == 0 || ttl > 300000 {
            return Err(DomainError::InvalidInput(
                "approval lifetime or target revision".into(),
            ));
        }
        if let Some(identity) = call.mcp_identity() {
            identity.check_scope(scope)?;
            if identity.target_revision() != target_revision { return Err(DomainError::Conflict); }
            if let Some(review) = call.mcp_reviewed_policy() { review.check(identity)?; }
        }
        if let Some(old) = self.pending.get(call.operation()) {
            if old.call.args_hash() != call.args_hash()
                || old.call.name() != call.name()
                || old.call.call_id() != call.call_id()
                || old.scope != *scope
                || old.call.registry_revision != call.registry_revision
                || old.call.mcp_reviewed_policy() != call.mcp_reviewed_policy()
                || old.request.binding.target_revision != target_revision
                || old.request.binding.policy_revision != policy_revision
                || now >= old.request.binding.expires_ms
            {
                return Err(DomainError::Conflict);
            }
            return Ok(Preflight::Approval(old.request.clone()));
        }
        let decision = if call.origin() == super::registry::Origin::Mcp {
            super::mcp::decision(&call, policy)?
        } else if call.origin() == super::registry::Origin::TrustedHost && call.name() == super::mcp::discovery::NAME {
            // This native facade reads cached metadata and changes only the
            // bounded request selection. It cannot connect or execute MCP calls.
            policy::Decision { allow: true, reason: None, approval_request: None }
        } else {
            policy::evaluate_in_folders(call.name(), call.args(), policy, untrusted_extensions, folders)?
        };
        if !decision.allow {
            return Ok(Preflight::Rejected(
                decision
                    .reason
                    .unwrap_or_else(|| "Tool execution was blocked".into()),
            ));
        }
        if let Some(display) = decision.approval_request {
            if self.pending.len() >= 256 {
                return Err(DomainError::QueueFull);
            }
            self.serial = self.serial.checked_add(1).ok_or(DomainError::Conflict)?;
            let binding = Binding {
                id: format!("approval-{}-{}", self.incarnation, self.serial),
                operation: call.operation().into(),
                argument_hash: call.args_hash().into(),
                workspace: scope.workspace.clone(),
                session: scope.session.clone(),
                epoch: scope.epoch,
                revision: scope.revision,
                registry_revision: call.registry_revision,
                policy_revision,
                target_revision: target_revision.into(),
                expires_ms: now.checked_add(ttl).ok_or(DomainError::Conflict)?,
            };
            let request = ApprovalRequest { binding, display };
            self.pending.insert(
                call.operation().into(),
                Pending {
                    call,
                    scope: scope.clone(),
                    request: request.clone(),
                },
            );
            Ok(Preflight::Approval(request))
        } else {
            Ok(Preflight::Ready(AuthorizedCall {
                registry_revision: call.registry_revision,
                call,
                scope: scope.clone(),
                policy_revision,
                target_revision: target_revision.into(),
                expires_ms: None,
            }))
        }
    }
    pub fn answer(
        &mut self,
        binding: &Binding,
        allow: bool,
        registry: &Registry,
        fence: ExecutionFence<'_>,
    ) -> Result<Option<AuthorizedCall>, DomainError> {
        let ExecutionFence {
            scope,
            policy_revision,
            target_revision,
            now,
        } = fence;
        let pending = self
            .pending
            .get(&binding.operation)
            .ok_or(DomainError::PermissionDenied)?;
        if &pending.request.binding != binding
            || pending.scope != *scope
            || binding.policy_revision != policy_revision
            || binding.target_revision != target_revision
            || now >= binding.expires_ms
        {
            return Err(DomainError::Conflict);
        }
        registry.validate_current(&pending.call)?;
        let pending = self.pending.remove(&binding.operation).unwrap();
        if !allow {
            return Ok(None);
        }
        Ok(Some(AuthorizedCall {
            registry_revision: binding.registry_revision,
            call: pending.call,
            scope: pending.scope,
            policy_revision,
            target_revision: target_revision.into(),
            expires_ms: Some(binding.expires_ms),
        }))
    }

    /// Review expiry is checked and consumed before issuing a separately bounded
    /// execution capability. All identity, revision and replay checks still apply.
    pub fn answer_for_execution(
        &mut self,
        binding: &Binding,
        allow: bool,
        registry: &Registry,
        fence: ExecutionFence<'_>,
        execution_ms: u64,
    ) -> Result<Option<AuthorizedCall>, DomainError> {
        if execution_ms == 0 || execution_ms > 86_430_000 {
            return Err(DomainError::InvalidInput("execution lifetime".into()));
        }
        let expires = fence.now.checked_add(execution_ms).ok_or(DomainError::Conflict)?;
        let mut authorization = self.answer(binding, allow, registry, fence)?;
        if let Some(authorization) = &mut authorization {
            authorization.expires_ms = Some(expires);
        }
        Ok(authorization)
    }
}
