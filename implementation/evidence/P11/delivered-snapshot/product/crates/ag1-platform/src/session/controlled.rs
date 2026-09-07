//! Native controlled-mode observations use acknowledged P09 outcomes and original
//! artifacts. Model-facing projections and command substrings never prove a pass.
use ag1_core::{
    DomainError,
    runtime::{
        completion,
        policy::{
            TaskPolicy,
            orchestrator::{Controller, Observation, Verification},
        },
    },
    tools::executor::{Effect, ExecutionArtifacts, Status},
};
use ag1_storage::context::ContextStore;
use serde_json::Value;
use std::sync::{Arc, Mutex};
pub(super) type Shared = Arc<Mutex<Controller>>;
pub(super) fn new(policy: TaskPolicy) -> Result<Shared, DomainError> {
    Ok(Arc::new(Mutex::new(Controller::new(policy)?)))
}
pub(super) fn serialize(model: &Value) -> bool {
    ag1_core::model::infer_capabilities(model["contextWindow"].as_u64(), false, false, false)
        .serialize_tools()
}
pub(super) fn path(args: &Value) -> Option<&str> {
    ["path", "filePath", "file_path"]
        .iter()
        .find_map(|key| args.get(key).filter(|v| !v.is_null()))
        .and_then(Value::as_str)
}
pub(super) fn observe(
    controller: &Shared,
    files: &crate::file_tools::FileService,
    artifacts: &ContextStore<'_>,
    outcome: &ag1_core::tools::executor::Outcome,
) -> Result<(), DomainError> {
    if outcome.record.status != Status::Completed || outcome.replayed {
        return Ok(());
    }
    let args = artifacts.read_json(&outcome.record.arguments)?;
    let path = path(&args).map(|p| files.relative_path(p)).transpose()?;
    let native = artifacts.read_json(
        outcome
            .record
            .native_result
            .as_ref()
            .ok_or(DomainError::DurabilityUnknown)?,
    )?;
    let mut controller = controller.lock().map_err(|_| DomainError::Conflict)?;
    let command = args["command"].as_str();
    let verifier =
        outcome.record.tool == "bash" && command.is_some_and(|c| controller.verifier_matches(c));
    if !verifier && outcome.record.effect == Effect::Unknown {
        controller.invalidate_verification();
    }
    let mut verification = None;
    let mut output = String::new();
    if verifier {
        let (assessment, full_output) =
            super::completion_evidence::shell_verification(artifacts, &native, command.unwrap())?;
        output = full_output;
        let success = assessment.status == completion::Status::Passed;
        verification = Some(Verification {
            success,
            output: &output,
        });
    }
    let result = controller.observe(Observation {
        name: &outcome.record.tool,
        path: path.as_deref(),
        is_error: native["isError"] == true,
        mutation_committed: outcome.record.effect == Effect::Committed
            && ag1_core::runtime::policy::orchestrator::mutation_tool(&outcome.record.tool),
        verification,
    });
    drop(output);
    result
}
