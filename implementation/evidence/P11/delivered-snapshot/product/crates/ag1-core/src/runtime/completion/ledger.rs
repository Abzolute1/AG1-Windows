//! Immutable candidate lineage. Capacity failure retains all unresolved evidence
//! and changes no owner state; durable/pageable native capture is a separate port.
use super::{Status, Verification, classify};
use crate::DomainError;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub sequence: u64,
    pub patch_sha256: String,
    pub command: String,
    #[serde(flatten)]
    pub verification: Verification,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedFailure<'a> {
    pub command_key: &'a str,
    pub failed_checks: Vec<&'a str>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerAssessment<'a> {
    pub passed: bool,
    pub reasons: Vec<String>,
    pub patch_sha256: &'a str,
    pub passing_verification_count: usize,
    pub unresolved_failures: Vec<UnresolvedFailure<'a>>,
    /// Borrow existing immutable observations instead of cloning full history.
    pub observations: &'a [Observation],
}
#[derive(Default)]
pub struct Ledger {
    active: String,
    parents: BTreeMap<String, Option<String>>,
    observations: Vec<Observation>,
    bytes: usize,
    sequence: u64,
}
impl Ledger {
    fn validate_candidate(&self, patch: &str) -> Result<(), DomainError> {
        if patch.len() > 128 || patch.chars().any(char::is_control) {
            return Err(DomainError::InvalidInput("candidate identity".into()));
        }
        if !patch.is_empty() && !self.parents.contains_key(patch) && self.parents.len() >= 4096 {
            return Err(DomainError::LimitExceeded(
                "verification candidate history".into(),
            ));
        }
        Ok(())
    }
    pub fn note_candidate(&mut self, patch: &str) -> Result<(), DomainError> {
        self.validate_candidate(patch)?;
        if !patch.is_empty() && patch != self.active {
            self.parents
                .entry(patch.into())
                .or_insert_with(|| (!self.active.is_empty()).then(|| self.active.clone()));
            self.active = patch.into();
        }
        Ok(())
    }
    /// The native capture owner supplies patch identity and baseline state.
    /// Words such as `git stash` cannot establish a clean baseline by themselves.
    pub fn note_verification(
        &mut self,
        patch: &str,
        command: &str,
        output: &str,
        is_error: bool,
        baseline_probe: Option<bool>,
    ) -> Result<&Observation, DomainError> {
        self.validate_candidate(patch)?;
        if self.observations.len() >= 4096 {
            return Err(DomainError::LimitExceeded(
                "verification observations".into(),
            ));
        }
        let mut verification = classify(command, output, is_error)?;
        verification.baseline_probe =
            baseline_probe.unwrap_or(verification.baseline_probe) && patch.is_empty();
        let sequence = self.sequence.checked_add(1).ok_or(DomainError::Conflict)?;
        let observation = Observation {
            sequence,
            patch_sha256: patch.into(),
            command: command.into(),
            verification,
        };
        let size = serde_json::to_vec(&observation)
            .map_err(|_| DomainError::Conflict)?
            .len();
        let bytes = self
            .bytes
            .checked_add(size)
            .filter(|n| *n <= 16 * 1024 * 1024)
            .ok_or_else(|| DomainError::LimitExceeded("verification working bytes".into()))?;
        if !observation.verification.baseline_probe {
            self.note_candidate(patch)?;
        }
        self.bytes = bytes;
        self.sequence = sequence;
        self.observations.push(observation);
        Ok(self.observations.last().unwrap())
    }
    pub fn assess<'a>(&'a self, patch: &'a str) -> LedgerAssessment<'a> {
        let mut lineage = BTreeSet::new();
        let mut cursor = Some(patch);
        while let Some(patch) = cursor.filter(|p| !p.is_empty()) {
            if !lineage.insert(patch) {
                break;
            }
            cursor = self.parents.get(patch).and_then(Option::as_deref);
        }
        // Vec replacement preserves first-insertion order like SDK Map.set.
        let mut latest: Vec<&Observation> = Vec::new();
        let mut current: Vec<&Observation> = Vec::new();
        let replace = |items: &mut Vec<&'a Observation>, observation: &'a Observation| {
            if let Some(index) = items
                .iter()
                .position(|o| o.verification.command_key == observation.verification.command_key)
            {
                items[index] = observation;
            } else {
                items.push(observation);
            }
        };
        for observation in &self.observations {
            if !observation.verification.baseline_probe
                && lineage.contains(observation.patch_sha256.as_str())
            {
                replace(&mut latest, observation);
                if observation.patch_sha256 == patch {
                    replace(&mut current, observation);
                }
            }
        }
        let current_passes = current
            .into_iter()
            .filter(|o| o.verification.status == Status::Passed)
            .collect::<Vec<_>>();
        let passing_checks = current_passes
            .iter()
            .flat_map(|o| o.verification.passed_checks.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let baseline_failures = self
            .observations
            .iter()
            .filter(|o| o.verification.baseline_probe && o.verification.status == Status::Failed)
            .flat_map(|o| o.verification.failed_checks.iter().map(String::as_str))
            .collect::<BTreeSet<_>>();
        let unresolved_failures = latest
            .into_iter()
            .filter(|o| o.verification.status == Status::Failed)
            .filter_map(|o| {
                let failed_checks: Vec<_> = o
                    .verification
                    .failed_checks
                    .iter()
                    .map(String::as_str)
                    .filter(|check| {
                        !baseline_failures.contains(check) && !passing_checks.contains(check)
                    })
                    .collect();
                (!failed_checks.is_empty() || o.verification.failed_checks.is_empty()).then_some(
                    UnresolvedFailure {
                        command_key: &o.verification.command_key,
                        failed_checks,
                    },
                )
            })
            .collect::<Vec<_>>();
        let mut reasons = Vec::new();
        if patch.is_empty() {
            reasons.push("no source patch is active".into());
        }
        if current_passes.is_empty() {
            reasons.push("the exact current patch revision has no successful test command".into());
        }
        if !unresolved_failures.is_empty() {
            reasons.push(format!("{} inherited verification failure(s) were not rerun successfully or proven on baseline", unresolved_failures.len()));
        }
        LedgerAssessment {
            passed: reasons.is_empty(),
            reasons,
            patch_sha256: patch,
            passing_verification_count: current_passes.len(),
            unresolved_failures,
            observations: &self.observations,
        }
    }
}
