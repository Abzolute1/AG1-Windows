//! Controlled-mode stop decisions from trusted host observations. These flags
//! guide a controller; they are not a substitute for the completion ledger.
use super::Intent;
use serde::{Deserialize, Serialize};

const VERIFIED: &str = "Controlled runtime: verifier already passed after the latest mutation. Stop using tools and provide the final summary.";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    pub intent: Intent,
    #[serde(default)]
    pub verification_passed_after_mutation: bool,
    pub stop_after_verifier_pass: Option<bool>,
    #[serde(default)]
    pub sufficient_evidence: bool,
    #[serde(default)]
    pub source_count: u32,
    #[serde(default)]
    pub budget_exhausted: bool,
    #[serde(default)]
    pub repeated_failure_count: u32,
    pub repeated_failure_limit: Option<u32>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub should_stop: bool,
    pub final_answer_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<&'static str>,
    pub next_action: &'static str,
}
fn decision(stop: Option<(&'static str, &'static str)>) -> Decision {
    Decision {
        should_stop: stop.is_some(),
        final_answer_only: stop.is_some(),
        reason: stop.map(|v| v.0),
        label: stop.map(|v| v.1),
        next_action: if stop.is_some() {
            "final_answer"
        } else {
            "continue"
        },
    }
}
pub fn decide(input: &Input) -> Decision {
    decision(
        if input.verification_passed_after_mutation && input.stop_after_verifier_pass != Some(false)
        {
            Some((VERIFIED, "verified_after_mutation"))
        } else if input.intent == Intent::DirectAnswer && input.sufficient_evidence {
            Some((
                "Controlled runtime: direct-answer task has sufficient evidence. Provide the final answer.",
                "direct_answer_sufficient",
            ))
        } else if input.intent == Intent::Research && input.source_count >= 3 {
            Some((
                "Controlled runtime: research task has enough sourced evidence. Provide the final answer with citations.",
                "research_sources_sufficient",
            ))
        } else if input.budget_exhausted {
            Some((
                "Controlled runtime: budget is exhausted. Provide the best current result and remaining risk.",
                "budget_exhausted",
            ))
        } else if input.repeated_failure_count >= input.repeated_failure_limit.unwrap_or(3) {
            Some((
                "Controlled runtime: repeated failure limit reached. Summarize the blocker and smallest next repair.",
                "repeated_failure_limit",
            ))
        } else {
            None
        },
    )
}
pub struct Controller {
    stop_after_verifier_pass: bool,
    mutation_since_verifier: bool,
    verifier_passed_after_mutation: bool,
    verifier_failed_after_mutation: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub mutation_since_verifier: bool,
    pub verifier_passed_after_mutation: bool,
    pub verifier_failed_after_mutation: bool,
    pub final_answer_only: bool,
}
impl Default for Controller {
    fn default() -> Self {
        Self::new(true)
    }
}
impl Controller {
    pub fn new(stop_after_verifier_pass: bool) -> Self {
        Self {
            stop_after_verifier_pass,
            mutation_since_verifier: false,
            verifier_passed_after_mutation: false,
            verifier_failed_after_mutation: false,
        }
    }
    pub fn note_mutation(&mut self) {
        self.mutation_since_verifier = true;
        self.verifier_passed_after_mutation = false;
        self.verifier_failed_after_mutation = false;
    }
    pub fn note_verification(&mut self, matched_verifier: bool, success: bool) {
        if !matched_verifier || !self.mutation_since_verifier {
            return;
        }
        if success {
            self.verifier_passed_after_mutation = true;
            self.verifier_failed_after_mutation = false;
            self.mutation_since_verifier = false;
        } else {
            self.verifier_failed_after_mutation = true;
        }
    }
    pub fn evaluate(&self) -> Decision {
        decision(
            (self.verifier_passed_after_mutation && self.stop_after_verifier_pass)
                .then_some((VERIFIED, "verified_after_mutation")),
        )
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            mutation_since_verifier: self.mutation_since_verifier,
            verifier_passed_after_mutation: self.verifier_passed_after_mutation,
            verifier_failed_after_mutation: self.verifier_failed_after_mutation,
            final_answer_only: self.verifier_passed_after_mutation && self.stop_after_verifier_pass,
        }
    }
}
