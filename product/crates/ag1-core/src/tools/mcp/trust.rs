//! Reviewed MCP policy is native authority, separate from peer annotations.
//! A witness is deliberately not deserializable. Its native owner must revoke
//! it on durable policy/configuration/account changes before retiring a lease.
use super::{Identity, SelectedDefinition, invalid, label};
use crate::{DomainError, tools::policy::Mode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustMode {
    #[default]
    Restricted,
    DisableWrites,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    Read,
    Write,
    Unknown,
    Consequential,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Ask,
    Allow,
    Deny,
}
/// Top-level scalar equality only: no coercion, regex, path interpretation or
/// implicit wildcard. AllArguments must itself be deliberately reviewed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "fields",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum Arguments {
    AllArguments,
    Equals(BTreeMap<String, Value>),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub arguments: Arguments,
    pub effect: Effect,
    pub permission: Permission,
}
impl Rule {
    pub fn validate(&self) -> Result<(), DomainError> {
        if let Arguments::Equals(fields) = &self.arguments
            && (fields.is_empty()
                || fields.len() > 16
                || fields.iter().any(|(key, value)| {
                    !label(key)
                        || value.is_array()
                        || value.is_object()
                        || value.as_str().is_some_and(|s| s.len() > 4096)
                }))
        {
            return Err(invalid());
        }
        if serde_json::to_vec(self).map_err(|_| invalid())?.len() > 16 * 1024 {
            return Err(DomainError::LimitExceeded("MCP trust rule bytes".into()));
        }
        Ok(())
    }
    fn matches(&self, args: &Value) -> bool {
        match &self.arguments {
            Arguments::AllArguments => args.is_object(),
            Arguments::Equals(fields) => fields
                .iter()
                .all(|(key, value)| args.get(key) == Some(value)),
        }
    }
}
/// Supplied by native host classification, never deserialized from a server or
/// user policy DTO. Unknown is not evidence of safety. Consequential is a floor
/// that a user-reviewed read/write rule cannot lower.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HostRisk {
    Unknown,
    Consequential,
}
impl HostRisk {
    pub fn for_call(identity: &Identity, args: &Value) -> Result<Self, DomainError> {
        identity.validate()?;
        crate::tools::schema::bounded(args, 256 * 1024, 32)?;
        Ok(if consequential_signal(identity, args) {
            Self::Consequential
        } else {
            Self::Unknown
        })
    }
}
/// A conservative native escalation heuristic, never a proof that an operation
/// is a read. Inspect the actual operation name and arguments, not server hints.
/// Explicit reviewed rules are still required for low-consequence admission.
pub(crate) fn consequential_signal(identity: &Identity, args: &Value) -> bool {
    fn signal(text: &str) -> bool {
        let mut words = String::with_capacity(text.len());
        let mut previous_lower = false;
        for c in text.chars() {
            if c.is_ascii_uppercase() && previous_lower {
                words.push(' ');
            }
            if c.is_ascii_alphanumeric() {
                words.push(c.to_ascii_lowercase());
            } else {
                words.push(' ');
            }
            previous_lower = c.is_ascii_lowercase();
        }
        words.split_whitespace().any(|word| {
            matches!(
                word,
                "delete"
                    | "destroy"
                    | "drop"
                    | "truncate"
                    | "erase"
                    | "purge"
                    | "wipe"
                    | "remove"
                    | "overwrite"
                    | "reset"
                    | "revoke"
                    | "terminate"
                    | "kill"
                    | "admin"
                    | "administrator"
                    | "sudo"
                    | "privilege"
                    | "privileges"
                    | "permission"
                    | "permissions"
                    | "grant"
                    | "credential"
                    | "credentials"
                    | "password"
                    | "secret"
                    | "secrets"
                    | "token"
                    | "tokens"
                    | "apikey"
                    | "payment"
                    | "pay"
                    | "purchase"
                    | "buy"
                    | "charge"
                    | "refund"
                    | "billing"
                    | "transfer"
                    | "deploy"
                    | "publish"
                    | "send"
                    | "invite"
            )
        })
    }
    fn value_signal(value: &Value) -> bool {
        match value {
            Value::String(text) => signal(text),
            Value::Array(rows) => rows.iter().any(value_signal),
            Value::Object(fields) => fields
                .iter()
                .any(|(key, value)| signal(key) || value_signal(value)),
            _ => false,
        }
    }
    signal(&identity.original_name) || value_signal(args)
}
#[derive(Debug)]
struct Review {
    identity: Identity,
    revision: u64,
    mode: TrustMode,
    host_risk: HostRisk,
    basis: String,
    rules: Vec<Rule>,
    revoked: AtomicBool,
}
#[derive(Clone, Debug)]
pub struct ReviewedPolicy(Arc<Review>);
/// Native cache handle; it does not extend a registry's grant lifetime.
#[derive(Clone, Debug)]
pub struct WeakReviewedPolicy(Weak<Review>);
impl WeakReviewedPolicy {
    pub fn upgrade(&self) -> Option<ReviewedPolicy> {
        self.0.upgrade().map(ReviewedPolicy)
    }
}
impl PartialEq for ReviewedPolicy {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl ReviewedPolicy {
    pub fn downgrade(&self) -> WeakReviewedPolicy {
        WeakReviewedPolicy(Arc::downgrade(&self.0))
    }
    /// Native review/admission only. Persisted rule data must first be checked
    /// against the current effective target and the user's exact review.
    pub fn new(
        selected: &SelectedDefinition,
        revision: u64,
        mode: TrustMode,
        host_risk: HostRisk,
        basis: String,
        rules: Vec<Rule>,
    ) -> Result<Self, DomainError> {
        selected.identity.validate()?;
        if revision == 0 || basis.is_empty() || basis.len() > 4096 || rules.len() > 32 {
            return Err(invalid());
        }
        for rule in &rules {
            rule.validate()?;
        }
        if serde_json::to_vec(&rules).map_err(|_| invalid())?.len() > 64 * 1024 {
            return Err(DomainError::LimitExceeded("MCP trust policy bytes".into()));
        }
        let mut identity = selected.identity.clone();
        // A scoped grant may be published for several conversations in the same
        // canonical scope. Execution ownership is independently checked by core.
        identity.execution_scope = None;
        Ok(Self(Arc::new(Review {
            identity,
            revision,
            mode,
            host_risk,
            basis,
            rules,
            revoked: AtomicBool::new(false),
        })))
    }
    pub fn revoke(&self) {
        self.0.revoked.store(true, Ordering::Release);
    }
    pub fn revision(&self) -> u64 {
        self.0.revision
    }
    pub fn check(&self, identity: &Identity) -> Result<(), DomainError> {
        let mut target = identity.clone();
        target.execution_scope = None;
        if self.0.revoked.load(Ordering::Acquire) || target != self.0.identity {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
    pub(crate) fn evaluate(
        &self,
        identity: &Identity,
        args: &Value,
        mode: Mode,
    ) -> Result<Assessment, DomainError> {
        self.check(identity)?;
        let matching: Vec<_> = self.0.rules.iter().filter(|r| r.matches(args)).collect();
        let argument_signal = consequential_signal(identity, args);
        let host_risk = if argument_signal {
            HostRisk::Consequential
        } else {
            self.0.host_risk
        };
        let effect = if host_risk == HostRisk::Consequential
            || matching.iter().any(|r| r.effect == Effect::Consequential)
        {
            Effect::Consequential
        } else if matching.is_empty() || matching.iter().any(|r| r.effect == Effect::Unknown) {
            Effect::Unknown
        } else if matching.iter().any(|r| r.effect == Effect::Write) {
            Effect::Write
        } else {
            Effect::Read
        };
        let permission = if matching.iter().any(|r| r.permission == Permission::Deny)
            || (self.0.mode == TrustMode::DisableWrites
                || matches!(mode, Mode::Planning | Mode::Headless))
                && effect != Effect::Read
        {
            Permission::Deny
        } else if matches!(effect, Effect::Unknown | Effect::Consequential)
            || matching.iter().any(|r| r.permission == Permission::Ask)
        {
            Permission::Ask
        } else {
            Permission::Allow
        };
        Ok(Assessment {
            effect,
            permission,
            host_risk,
            basis: if argument_signal {
                "AG1 detected a potentially consequential operation name or argument; exact confirmation is required regardless of reviewed grants. This conservative heuristic is not a guarantee of server behavior.".into()
            } else {
                self.0.basis.clone()
            },
            revision: self.0.revision,
        })
    }
}
pub(crate) struct Assessment {
    pub effect: Effect,
    pub permission: Permission,
    pub host_risk: HostRisk,
    pub basis: String,
    pub revision: u64,
}
impl SelectedDefinition {
    pub fn reviewed_policy(&self) -> Option<&ReviewedPolicy> {
        self.reviewed_policy.as_ref()
    }
    pub fn with_reviewed_policy(mut self, policy: ReviewedPolicy) -> Result<Self, DomainError> {
        policy.check(&self.identity)?;
        self.reviewed_policy = Some(policy);
        Ok(self)
    }
}
