//! Request-local completion evidence from acknowledged native tool receipts.
//! This supplements the compatibility transcript assessment; it is not yet the
//! full workspace/patch lineage or restart reconciliation service.
use ag1_core::{
    DomainError,
    runtime::completion::{self, Status, Verification},
    tools::executor::{Effect, ExecutionArtifacts, Outcome, Status as ToolStatus},
};
use ag1_storage::context::ContextStore;
use base64::Engine;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub(super) type Shared = Arc<Mutex<Evidence>>;
pub(super) struct Evidence {
    canonical: Option<String>,
    pub mutation_seen: bool,
    checks: BTreeMap<String, Status>,
}
impl Evidence {
    pub fn new(canonical: Option<String>) -> Shared {
        Arc::new(Mutex::new(Self {
            canonical,
            mutation_seen: false,
            checks: BTreeMap::new(),
        }))
    }
    pub fn verified(&self) -> bool {
        self.mutation_seen
            && self.checks.values().any(|s| *s == Status::Passed)
            && !self
                .checks
                .values()
                .any(|s| matches!(s, Status::Failed | Status::NoTests | Status::Inconclusive))
    }
    fn recognizes(&self, command: &str) -> bool {
        self.canonical
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty() && v.trim() == command.trim())
            || direct_test(command)
    }
}
/// Conservative fallback when no exact host/repository verifier matches. Shell
/// composition needs a canonical command: quoted text cannot grant execution.
fn direct_test(command: &str) -> bool {
    if command.chars().any(|c| ";&|\n\r`$<>\\".contains(c)) {
        return false;
    }
    if command
        .split_ascii_whitespace()
        .any(|word| matches!(word, "--help" | "--version" | "-h" | "--no-run" | "--list"))
    {
        return false;
    }
    let words: Vec<_> = command.split_ascii_whitespace().take(4).collect();
    match words.as_slice() {
        ["npm" | "pnpm" | "yarn" | "bun", "test", ..]
        | ["npm" | "pnpm" | "yarn" | "bun", "run", "test", ..]
        | ["node", "--test", ..]
        | [
            "cargo" | "go" | "dotnet" | "mvn" | "gradle" | "./mvnw" | "./gradlew" | "defects4j",
            "test",
            ..,
        ]
        | ["pytest" | "py.test", ..] => true,
        [python, "-m", "pytest", ..] => python
            .strip_prefix("python")
            .is_some_and(|s| s.bytes().all(|b| b.is_ascii_digit() || b == b'.')),
        _ => false,
    }
}
pub(super) fn observe(
    shared: &Shared,
    artifacts: &ContextStore<'_>,
    outcome: &Outcome,
) -> Result<(), DomainError> {
    if outcome.record.status != ToolStatus::Completed || outcome.replayed {
        return Ok(());
    }
    let mut evidence = shared.lock().map_err(|_| DomainError::Conflict)?;
    let mutation = ag1_core::runtime::policy::orchestrator::mutation_tool(&outcome.record.tool)
        && outcome.record.effect == Effect::Committed;
    if mutation {
        evidence.mutation_seen = true;
        evidence.checks.clear();
        return Ok(());
    }
    let args = artifacts.read_json(&outcome.record.arguments)?;
    if outcome.record.tool == "bash"
        && let Some(command) = args["command"].as_str()
        && evidence.recognizes(command)
    {
        if !evidence.checks.contains_key(command) && evidence.checks.len() >= 256 {
            return Err(DomainError::LimitExceeded(
                "native completion checks".into(),
            ));
        }
        let native = artifacts.read_json(
            outcome
                .record
                .native_result
                .as_ref()
                .ok_or(DomainError::DurabilityUnknown)?,
        )?;
        let (verification, _) = shell_verification(artifacts, &native, command)?;
        if evidence.mutation_seen {
            evidence.checks.insert(command.into(), verification.status);
        }
    } else if outcome.record.effect == Effect::Unknown {
        evidence.checks.clear();
    }
    Ok(())
}
pub(super) fn shell_verification(
    artifacts: &ContextStore<'_>,
    native: &Value,
    command: &str,
) -> Result<(Verification, String), DomainError> {
    let original: ag1_core::context::ContextArtifact =
        serde_json::from_value(native["details"]["rawOutputArtifact"].clone())
            .map_err(|_| DomainError::DurabilityUnknown)?;
    if original.bytes > 6 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("native verifier output".into()));
    }
    let raw = artifacts.read_json(&original)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw["data"].as_str().ok_or(DomainError::DurabilityUnknown)?)
        .map_err(|_| DomainError::DurabilityUnknown)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded("native verifier output".into()));
    }
    if raw["encoding"] != "base64" || raw["bytes"].as_u64() != Some(bytes.len() as u64) {
        return Err(DomainError::DurabilityUnknown);
    }
    let output = ag1_core::tools::shell::sanitize(&bytes)?;
    let success = raw["complete"] == true
        && native["details"]["fullOutputRetained"] == true
        && native["details"]["termination"] == "exit"
        && native["isError"] != true;
    let mut verification = completion::classify(command, &output, !success)?;
    if reports_zero_tests(&output) {
        verification.status = Status::NoTests;
    }
    Ok((verification, output))
}
fn reports_zero_tests(output: &str) -> bool {
    output
        .lines()
        .filter_map(|line| {
            line.strip_prefix("# tests ")
                .or_else(|| line.strip_prefix("ℹ tests "))
        })
        .filter_map(|n| n.trim().parse::<u64>().ok())
        .next_back()
        == Some(0)
}
