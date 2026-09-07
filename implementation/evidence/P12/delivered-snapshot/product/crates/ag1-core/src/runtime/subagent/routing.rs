use super::Profile;
use crate::DomainError;
use serde_json::Value;
pub fn validate_catalog(candidates: &[Value]) -> Result<(), DomainError> {
    if candidates.len() > 1024 {
        return Err(DomainError::LimitExceeded(
            "delegation model catalog".into(),
        ));
    }
    crate::task::bounded_json(&serde_json::json!(candidates))
}

/// Candidates are supplied by the host only after credential availability is
/// checked. Model identities in a request cannot introduce a new endpoint.
pub fn select(
    profile: Profile,
    explicit: Option<&str>,
    parent: &Value,
    candidates: &[Value],
) -> Result<Value, DomainError> {
    validate_catalog(candidates)?;
    let identity = |m: &Value| {
        format!(
            "{}/{}",
            m["provider"].as_str().unwrap_or(""),
            m["id"].as_str().unwrap_or("")
        )
    };
    if let Some(id) = explicit {
        if id.len() > 1024
            || id
                .split_once('/')
                .is_none_or(|(p, m)| p.is_empty() || m.is_empty())
        {
            return Err(DomainError::InvalidInput(
                "subagent model requires provider/id".into(),
            ));
        }
        return candidates
            .iter()
            .find(|m| identity(m) == id)
            .cloned()
            .ok_or_else(|| DomainError::Unavailable("subagent model is not configured".into()));
    }
    if parent["provider"].as_str().is_none_or(str::is_empty)
        || parent["id"].as_str().is_none_or(str::is_empty)
    {
        return Err(DomainError::Unavailable("subagent parent model".into()));
    }
    if profile.uses_parent_model() {
        return Ok(parent.clone());
    }
    let cost = |m: &Value| {
        if m["provider"] == "local" {
            return 0.0;
        }
        let field = |name: &str| {
            m["cost"][name]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.)
                .unwrap_or(f64::INFINITY)
        };
        let input = field("input");
        input + field("output") + field("cacheRead").min(input) + field("cacheWrite").min(input)
    };
    let mut seen = std::collections::BTreeSet::new();
    let best = std::iter::once(parent)
        .chain(candidates)
        .filter(|m| seen.insert(identity(m)))
        .filter(|m| {
            m["input"]
                .as_array()
                .is_some_and(|inputs| inputs.iter().any(|v| v == "text"))
                && m["contextWindow"].as_u64().unwrap_or(0) >= 16384
                && m["maxTokens"].as_u64().unwrap_or(0) >= 4096
        })
        .min_by(|a, b| {
            cost(a)
                .total_cmp(&cost(b))
                .then_with(|| (a["reasoning"] == true).cmp(&(b["reasoning"] == true)))
                .then_with(|| identity(a).cmp(&identity(b)))
        });
    Ok(best
        .filter(|best| cost(best) < cost(parent))
        .unwrap_or(parent)
        .clone())
}
