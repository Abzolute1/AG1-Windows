//! Controlled tool decisions. Paths have already been normalized by the host;
//! capability opens and P09 approval still govern every actual operation.
use super::{
    Mode, TaskPolicy,
    evidence::{self, Action, State},
    repair, stop,
};
use crate::DomainError;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub allow: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
}
impl Decision {
    fn allow() -> Self {
        Self {
            allow: true,
            reason: None,
            next_action: None,
        }
    }
    fn block(reason: String, next_action: Option<String>) -> Self {
        Self {
            allow: false,
            reason: Some(reason),
            next_action,
        }
    }
    pub fn rejection(&self) -> Option<String> {
        if self.allow {
            return None;
        }
        Some(match &self.next_action {
            Some(next) => format!(
                "{}\nNext allowed action: {next}",
                self.reason
                    .as_deref()
                    .unwrap_or("Controlled execution blocked")
            ),
            None => self.reason.clone().unwrap_or_default(),
        })
    }
}
/// Host-observed, durably retained verifier result. Matching command text alone
/// cannot construct this from an untrusted model-facing tool projection.
pub struct Verification<'a> {
    pub success: bool,
    pub output: &'a str,
}
pub struct Observation<'a> {
    pub name: &'a str,
    pub path: Option<&'a str>,
    pub is_error: bool,
    pub mutation_committed: bool,
    pub verification: Option<Verification<'a>>,
}
pub struct Controller {
    policy: TaskPolicy,
    plan: Vec<evidence::Step>,
    state: State,
    reads: u32,
    searches: u32,
    patches: u32,
    mutated_since_verifier: std::collections::BTreeSet<String>,
    stop: stop::Controller,
    repair: repair::Controller,
}
pub fn mutation_tool(name: &str) -> bool {
    matches!(
        name,
        "edit" | "write" | "append" | "editApply" | "notebook_edit" | "NotebookEdit"
    )
}
impl Controller {
    pub fn new(policy: TaskPolicy) -> Result<Self, DomainError> {
        let plan = evidence::build(&policy)?;
        let repair = repair::Controller::new(
            policy.allowed_paths.clone(),
            policy.expected_changed_paths.clone(),
            2,
        )?;
        let stop = stop::Controller::new(policy.stop_after_verifier_pass);
        Ok(Self {
            policy,
            plan,
            repair,
            stop,
            state: State::default(),
            reads: 0,
            searches: 0,
            patches: 0,
            mutated_since_verifier: Default::default(),
        })
    }
    pub fn policy(&self) -> &TaskPolicy {
        &self.policy
    }
    pub fn stopped(&self) -> bool {
        self.stop.evaluate().should_stop || self.repair.snapshot().should_stop
    }
    pub fn verified_after_mutation(&self) -> bool {
        self.state.verifier_passed_after_mutation && !self.repair.snapshot().failed
    }
    /// Invalidate a prior check after an effect whose changed files are unknown,
    /// without inventing a source mutation or an expected changed path.
    pub fn invalidate_verification(&mut self) {
        self.state.verifier_passed_after_mutation = false;
        self.state.verification_completed = false;
        self.stop = stop::Controller::new(self.policy.stop_after_verifier_pass);
        if self.patches > 0 {
            self.stop.note_mutation();
            self.state.mutation_since_verifier = true;
        }
    }
    pub fn verifier_matches(&self, command: &str) -> bool {
        // Preserve whitespace inside quoted shell paths. The source substring
        // heuristic also matches `echo "npm test"`; it is not execution proof.
        self.policy
            .verification
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty() && command.trim() == v.trim())
    }
    pub fn next_action(&self) -> Option<String> {
        if self.repair.snapshot().should_stop {
            return Some("final_answer".into());
        }
        if self.repair.snapshot().failed && !self.repair.snapshot().mutation_since_failure {
            return Some(self.repair.next_repair_action());
        }
        evidence::next(&self.policy, &self.plan, &self.state).map(evidence::Step::format)
    }
    pub fn guidance(&self) -> Option<String> {
        if self.policy.mode != Mode::StrictBenchmark {
            return None;
        }
        self.repair.guidance().or_else(|| self.next_action())
    }
    pub fn evaluate(
        &self,
        name: &str,
        args: &Value,
        path: Option<&str>,
    ) -> Result<Decision, DomainError> {
        if name.len() > 128 || path.is_some_and(|p| p.len() > 4096 || p.contains('\0')) {
            return Err(DomainError::LimitExceeded("controlled tool input".into()));
        }
        let p = &self.policy;
        if p.mode == Mode::Flexible {
            return Ok(Decision::allow());
        }
        let stop = self.stop.evaluate();
        if stop.should_stop {
            return Ok(Decision::block(
                stop.reason.unwrap().into(),
                Some("final_answer".into()),
            ));
        }
        let repair = self.repair.snapshot();
        if repair.should_stop {
            return Ok(Decision::block(
                format!(
                    "Controlled runtime: repeated verifier failure limit reached. Summarize the blocker and smallest next repair: {}",
                    repair.last_failure.unwrap().summary
                ),
                Some("final_answer".into()),
            ));
        }
        let strict = p.mode == Mode::StrictBenchmark;
        let block = |reason| Ok(Decision::block(reason, self.next_action()));
        if strict && !p.allowed_paths.is_empty() {
            if name == "ls" {
                return block(format!(
                    "Controlled runtime: broad directory listing is blocked because relevant files are already known: {}.",
                    p.allowed_paths.join(", ")
                ));
            }
            if name == "find"
                && matches!(
                    args["pattern"].as_str().map(str::trim),
                    None | Some("" | "*" | "**/*" | "." | "./")
                )
            {
                return block(format!(
                    "Controlled runtime: broad find is blocked because relevant files are already known: {}.",
                    p.allowed_paths.join(", ")
                ));
            }
        }
        if strict
            && matches!(name, "grep" | "find")
            && self.patches == 0
            && self.searches >= p.max_searches_before_patch
        {
            return block(format!(
                "Controlled runtime: {name} exceeds the pre-patch search budget ({}).",
                p.max_searches_before_patch
            ));
        }
        if name == "read"
            && let Some(path) = path
        {
            let required = self.plan.iter().any(|s| {
                s.action == Action::Read
                    && s.required
                    && s.path.as_deref() == Some(path)
                    && !self.state.read_paths.contains(path)
            });
            if !required
                && strict
                && self.patches == 0
                && !p.expected_changed_paths.iter().any(|p| p == path)
            {
                if p.expected_changed_paths.len() == 1
                    && self.state.read_paths.len() >= 2
                    && self.state.read_paths.contains(&p.expected_changed_paths[0])
                {
                    return block(format!(
                        "Controlled runtime: the expected target and supporting context were already read. Edit {} before reading more files.",
                        p.expected_changed_paths.join(", ")
                    ));
                }
                if self.reads >= p.max_reads_before_patch {
                    return block(format!(
                        "Controlled runtime: read budget before patch is exhausted ({}).",
                        p.max_reads_before_patch
                    ));
                }
            }
        }
        if mutation_tool(name) {
            if !p.mutation_expected {
                return Ok(Decision::block(
                    format!(
                        "Controlled runtime: {name} is blocked because this task was classified as non-mutating ({}).",
                        serde_json::to_value(p.intent).unwrap().as_str().unwrap()
                    ),
                    None,
                ));
            }
            if strict && let Some(path) = path {
                if self.state.mutation_since_verifier
                    && !self.stop.snapshot().verifier_failed_after_mutation
                    && self.mutated_since_verifier.contains(path)
                {
                    return Ok(Decision::block(
                        format!(
                            "Controlled runtime: {path} was already edited. Run the verifier before making another edit to the same file."
                        ),
                        Some(
                            p.verification
                                .as_ref()
                                .map_or_else(|| "final_answer".into(), |v| format!("bash {v}")),
                        ),
                    ));
                }
                let allowed = if p.expected_changed_paths.is_empty() {
                    &p.allowed_paths
                } else {
                    &p.expected_changed_paths
                };
                if !allowed.is_empty() && !allowed.iter().any(|p| p == path) {
                    return Ok(Decision::block(
                        format!(
                            "Controlled runtime: {name} target {path} is outside the expected mutation set: {}.",
                            allowed.join(", ")
                        ),
                        Some(format!("edit {}", allowed[0])),
                    ));
                }
            }
        }
        if name == "bash" {
            let Some(command) = args["command"].as_str().filter(|s| !s.is_empty()) else {
                return Ok(Decision::block("Controlled runtime: bash requires a concrete command. Use the verifier command or a final answer.".into(),Some(p.verification.as_ref().map_or_else(||"final_answer".into(),|v|format!("bash {v}")))));
            };
            if command.len() > 65536 {
                return Err(DomainError::LimitExceeded(
                    "controlled shell command".into(),
                ));
            }
            if self.verifier_matches(command) {
                if let repair::Decision::Reject {
                    reason,
                    next_action,
                    ..
                } = self.repair.evaluate_verifier()
                {
                    return Ok(Decision::block(reason, Some(next_action)));
                }
                let missing: Vec<_> = p
                    .expected_changed_paths
                    .iter()
                    .filter(|p| !self.state.mutated_paths.contains(*p))
                    .collect();
                if strict && p.mutation_expected && !missing.is_empty() {
                    return Ok(Decision::block(
                        format!(
                            "Controlled runtime: verifier is blocked until expected mutation paths are edited: {}.",
                            missing
                                .iter()
                                .map(|p| p.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        Some(format!("edit {}", missing[0])),
                    ));
                }
            } else if strict && !p.allowed_paths.is_empty() && discovery(command) {
                return block(format!(
                    "Controlled runtime: shell discovery is blocked because relevant files are already known: {}.",
                    p.allowed_paths.join(", ")
                ));
            }
        }
        Ok(Decision::allow())
    }
    pub fn observe(&mut self, observation: Observation<'_>) -> Result<(), DomainError> {
        if let Some(path) = observation.path
            && (path.len() > 4096
                || path.contains('\0')
                || self.state.read_paths.len() >= 1000 && !self.state.read_paths.contains(path)
                || self.state.mutated_paths.len() >= 1000
                    && !self.state.mutated_paths.contains(path))
        {
            return Err(DomainError::LimitExceeded(
                "controlled observed paths".into(),
            ));
        }
        if let Some(verification) = observation.verification {
            // Diagnose before changing any state so failure limits cannot erase
            // a previous failure or fabricate a successful transition.
            if !verification.success {
                self.repair.note_failure(verification.output)?;
            } else {
                self.repair.note_success();
            }
            self.state.verification_completed = true;
            self.stop.note_verification(true, verification.success);
            let stop = self.stop.snapshot();
            self.state.mutation_since_verifier = stop.mutation_since_verifier;
            self.state.verifier_passed_after_mutation = stop.verifier_passed_after_mutation;
            if verification.success {
                self.mutated_since_verifier.clear();
            }
            return Ok(());
        }
        if observation.mutation_committed {
            self.patches = self.patches.saturating_add(1);
            self.stop.note_mutation();
            self.repair.note_mutation();
            self.state.mutation_since_verifier = true;
            self.state.verifier_passed_after_mutation = false;
            self.state.verification_completed = false;
            if let Some(path) = observation.path {
                self.state.mutated_paths.insert(path.into());
                self.mutated_since_verifier.insert(path.into());
            }
        } else if !observation.is_error {
            if observation.name == "read"
                && let Some(path) = observation.path
            {
                self.state.read_paths.insert(path.into());
                if self.patches == 0 {
                    self.reads = self.reads.saturating_add(1);
                }
            }
            if matches!(observation.name, "grep" | "find") && self.patches == 0 {
                self.searches = self.searches.saturating_add(1);
            }
        }
        Ok(())
    }
}
fn discovery(command: &str) -> bool {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        super::pattern(
            r"(^|[;&|]\s*)(ls\b|find\b|fd\b|rg\s+--files\b|pwd\b|git\s+status(\s|$)|git\s+branch(\s|$))",
        )
    });
    PATTERN.is_match(
        &command
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase(),
    )
}
