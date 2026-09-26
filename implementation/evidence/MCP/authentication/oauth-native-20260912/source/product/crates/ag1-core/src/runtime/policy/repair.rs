//! Repeated verifier-failure guidance. Only trusted host observations advance
//! this state, and diagnosis errors leave the prior failure intact.
use super::{
    evidence::likely_test,
    failure::{self, Failure},
};
use crate::DomainError;
use serde::Serialize;

pub struct Controller {
    relevant_paths: Vec<String>,
    expected_changed_paths: Vec<String>,
    repeated_failure_limit: u32,
    last_failure: Option<Failure>,
    same_failure_count: u32,
    mutation_since_failure: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot<'a> {
    pub failed: bool,
    pub mutation_since_failure: bool,
    pub same_failure_count: u32,
    pub repeated_failure_limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<&'a Failure>,
    pub should_stop: bool,
}
#[derive(Serialize)]
#[serde(untagged)]
pub enum Decision {
    Allow {
        allow: bool,
    },
    Reject {
        allow: bool,
        reason: String,
        #[serde(rename = "nextAction")]
        next_action: String,
    },
}
impl Controller {
    pub fn new(
        relevant_paths: Vec<String>,
        expected_changed_paths: Vec<String>,
        repeated_failure_limit: u32,
    ) -> Result<Self, DomainError> {
        if repeated_failure_limit == 0
            || [&relevant_paths, &expected_changed_paths]
                .iter()
                .any(|paths| {
                    paths.len() > 1000 || paths.iter().any(|p| p.len() > 4096 || p.contains('\0'))
                })
        {
            return Err(DomainError::InvalidInput("repair controller bounds".into()));
        }
        Ok(Self {
            relevant_paths,
            expected_changed_paths,
            repeated_failure_limit,
            last_failure: None,
            same_failure_count: 0,
            mutation_since_failure: false,
        })
    }
    pub fn note_mutation(&mut self) {
        if self.last_failure.is_some() {
            self.mutation_since_failure = true;
        }
    }
    pub fn note_success(&mut self) {
        self.last_failure = None;
        self.same_failure_count = 0;
        self.mutation_since_failure = false;
    }
    pub fn note_failure(&mut self, output: &str) -> Result<&Failure, DomainError> {
        let failure = failure::classify(output, &self.relevant_paths)?;
        self.same_failure_count = if self
            .last_failure
            .as_ref()
            .is_some_and(|old| old.signature == failure.signature)
        {
            self.same_failure_count.saturating_add(1)
        } else {
            1
        };
        self.last_failure = Some(failure);
        self.mutation_since_failure = false;
        Ok(self.last_failure.as_ref().unwrap())
    }
    pub fn next_repair_action(&self) -> String {
        let target = self
            .expected_changed_paths
            .first()
            .or_else(|| self.relevant_paths.iter().find(|p| !likely_test(p)));
        target
            .filter(|p| !p.is_empty())
            .map_or_else(|| "final_answer".into(), |p| format!("edit {p}"))
    }
    pub fn evaluate_verifier(&self) -> Decision {
        if let Some(failure) = &self.last_failure
            && !self.mutation_since_failure
        {
            let (reason, next_action) = if self.same_failure_count >= self.repeated_failure_limit {
                (
                    format!(
                        "Controlled runtime: same verifier failure repeated {} times ({}). Stop the loop and summarize the blocker: {}",
                        self.same_failure_count,
                        failure.kind.as_str(),
                        failure.summary
                    ),
                    "final_answer".into(),
                )
            } else {
                (
                    format!(
                        "Controlled runtime: verifier already failed with the current code ({}). Repair before rerunning verifier: {}",
                        failure.kind.as_str(),
                        failure.summary
                    ),
                    self.next_repair_action(),
                )
            };
            return Decision::Reject {
                allow: false,
                reason,
                next_action,
            };
        }
        Decision::Allow { allow: true }
    }
    pub fn guidance(&self) -> Option<String> {
        let failure = self.last_failure.as_ref()?;
        if self.mutation_since_failure {
            return None;
        }
        Some(if self.same_failure_count >= self.repeated_failure_limit {
            format!(
                "final_answer; repeated {} verifier failure: {}",
                failure.kind.as_str(),
                failure.summary
            )
        } else {
            format!(
                "{}; repair {} verifier failure: {}",
                self.next_repair_action(),
                failure.kind.as_str(),
                failure.summary
            )
        })
    }
    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            failed: self.last_failure.is_some(),
            mutation_since_failure: self.mutation_since_failure,
            same_failure_count: self.same_failure_count,
            repeated_failure_limit: self.repeated_failure_limit,
            last_failure: self.last_failure.as_ref(),
            should_stop: self.last_failure.is_some()
                && self.same_failure_count >= self.repeated_failure_limit
                && !self.mutation_since_failure,
        }
    }
}
