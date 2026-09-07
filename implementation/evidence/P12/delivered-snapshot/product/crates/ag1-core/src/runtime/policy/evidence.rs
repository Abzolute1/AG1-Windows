//! Ordered evidence guidance. State comes from acknowledged host observations;
//! the plan itself neither executes a tool nor proves completion.
use super::{Intent, TaskPolicy};
use crate::DomainError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::LazyLock};

static TEST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|/)(test|tests|__tests__)/|[._-](test|spec)\.[cm]?[jt]sx?$").unwrap()
});
pub(super) fn likely_test(path: &str) -> bool {
    TEST.is_match(&path.to_ascii_lowercase())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Read,
    Edit,
    Verify,
    FinalAnswer,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub required: bool,
    pub reason: String,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub read_paths: BTreeSet<String>,
    pub mutated_paths: BTreeSet<String>,
    pub mutation_since_verifier: bool,
    pub verifier_passed_after_mutation: bool,
    /// A terminal verifier observation for verification-only tasks. Separate
    /// from mutation evidence so requesting a check cannot skip the check.
    pub verification_completed: bool,
}
impl Step {
    pub fn format(&self) -> String {
        match (self.action, self.path.as_deref(), self.command.as_deref()) {
            (Action::Read, Some(path), _) if !path.is_empty() => format!("read {path}"),
            (Action::Edit, Some(path), _) if !path.is_empty() => format!("edit {path}"),
            (Action::Verify, _, Some(command)) if !command.is_empty() => format!("bash {command}"),
            _ => "final_answer".into(),
        }
    }
}
fn paths(values: &[String]) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .map(String::as_str)
        .filter(|s| !s.is_empty() && seen.insert(*s))
        .collect()
}
pub fn build(policy: &TaskPolicy) -> Result<Vec<Step>, DomainError> {
    policy.validate()?;
    let targets = if policy.expected_changed_paths.is_empty() {
        // Slice before de-duplicating, as in the pinned planner.
        let candidates: Vec<_> = policy
            .allowed_paths
            .iter()
            .filter(|p| !likely_test(p))
            .take(if policy.intent == Intent::MultiFileChange {
                6
            } else {
                1
            })
            .cloned()
            .collect();
        paths(&candidates)
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    } else {
        paths(&policy.expected_changed_paths)
            .into_iter()
            .map(str::to_owned)
            .collect()
    };
    let tests: Vec<_> = paths(&policy.likely_tests)
        .into_iter()
        .filter(|p| policy.allowed_paths.is_empty() || policy.allowed_paths.iter().any(|a| a == p))
        .collect();
    let mut plan = Vec::new();
    let step = |action, path: Option<&str>, command: Option<&str>, required, reason: &str| Step {
        action,
        path: path.map(str::to_owned),
        command: command.map(str::to_owned),
        required,
        reason: reason.into(),
    };
    if policy.mutation_expected {
        for path in tests {
            plan.push(step(
                Action::Read,
                Some(path),
                None,
                true,
                "read the likely failing or regression test before patching",
            ));
        }
        for path in &targets {
            plan.push(step(
                Action::Read,
                Some(path),
                None,
                true,
                "read the expected mutation target before patching",
            ));
        }
        let planned: BTreeSet<_> = plan.iter().filter_map(|step| step.path.clone()).collect();
        for path in policy
            .allowed_paths
            .iter()
            .filter(|p| !planned.contains(*p))
            .take(if policy.intent == Intent::MultiFileChange {
                2
            } else {
                1
            })
        {
            plan.push(step(
                Action::Read,
                Some(path),
                None,
                false,
                "optional supporting context if required by the already-read evidence",
            ));
        }
        for path in &targets {
            plan.push(step(
                Action::Edit,
                Some(path),
                None,
                true,
                "patch the expected mutation target after required evidence is read",
            ));
        }
    } else if policy.intent == Intent::VerificationOnly
        && policy.verification.as_ref().is_some_and(|v| !v.is_empty())
    {
        plan.push(step(
            Action::Verify,
            None,
            policy.verification.as_deref(),
            true,
            "run the requested verification command",
        ));
    } else if policy.intent != Intent::Research {
        for path in policy
            .allowed_paths
            .iter()
            .take(if policy.intent == Intent::DirectAnswer {
                1
            } else {
                2
            })
        {
            plan.push(step(
                Action::Read,
                Some(path),
                None,
                true,
                "read the smallest relevant local evidence set",
            ));
        }
    }
    if policy.mutation_expected && policy.verification.as_ref().is_some_and(|v| !v.is_empty()) {
        plan.push(step(
            Action::Verify,
            None,
            policy.verification.as_deref(),
            true,
            "verify after mutation",
        ));
    }
    plan.push(step(
        Action::FinalAnswer,
        None,
        None,
        true,
        "summarize the completed work and verification result",
    ));
    let mut seen = BTreeSet::new();
    plan.retain(|step| {
        seen.insert((
            step.action,
            step.path.clone(),
            step.command.clone(),
            step.required,
        ))
    });
    Ok(plan)
}
pub fn next<'a>(policy: &TaskPolicy, plan: &'a [Step], state: &State) -> Option<&'a Step> {
    let final_answer = || plan.iter().find(|s| s.action == Action::FinalAnswer);
    if state.verifier_passed_after_mutation {
        return final_answer();
    }
    for (action, observed) in [
        (Action::Read, &state.read_paths),
        (Action::Edit, &state.mutated_paths),
    ] {
        if let Some(step) = plan.iter().find(|s| {
            s.action == action
                && s.required
                && s.path
                    .as_ref()
                    .is_some_and(|p| !p.is_empty() && !observed.contains(p))
        }) {
            return Some(step);
        }
    }
    // The SDK only checks mutationSinceVerifier here, causing verification-only
    // plans to return final_answer without executing their required verifier.
    let verification_only = policy.intent == Intent::VerificationOnly;
    if (state.mutation_since_verifier || verification_only && !state.verification_completed)
        && let Some(step) = plan.iter().find(|s| {
            s.action == Action::Verify && s.command.as_ref().is_some_and(|v| !v.is_empty())
        })
    {
        return Some(step);
    }
    final_answer()
}
