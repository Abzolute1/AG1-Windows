//! An explicitly requested, versioned tuning proposal. Discovery, migration,
//! model selection and startup never call this module. It performs no writes.
use crate::{DomainError, OperationId};
pub struct ExplicitTuningRequest {
    pub version: u32,
    pub operation: OperationId,
    pub profile_sha256: String,
    pub current_n: u32,
    pub headroom_mib: f64,
    pub estimated_mib_per_layer: Option<f64>,
}
#[derive(Debug, PartialEq)]
pub struct TuningProposal {
    pub version: u32,
    pub operation: OperationId,
    pub expected_profile_sha256: String,
    pub previous_n: u32,
    pub proposed_n: u32,
}
pub fn propose(request: ExplicitTuningRequest) -> Result<TuningProposal, DomainError> {
    let estimate = request.estimated_mib_per_layer.unwrap_or(550.0);
    if request.version != 1
        || request.current_n > 60
        || !request.headroom_mib.is_finite()
        || !estimate.is_finite()
        || estimate <= 0.0
        || request.profile_sha256.len() != 64
        || !request
            .profile_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(DomainError::InvalidInput("explicit tuning request".into()));
    }
    // JS Math.round rounds halfway toward +infinity, including negative headroom.
    let delta = (request.headroom_mib / estimate + 0.5).floor();
    let proposed = (f64::from(request.current_n) - delta).clamp(0.0, 60.0) as u32;
    Ok(TuningProposal {
        version: 1,
        operation: request.operation,
        expected_profile_sha256: request.profile_sha256,
        previous_n: request.current_n,
        proposed_n: proposed,
    })
}
