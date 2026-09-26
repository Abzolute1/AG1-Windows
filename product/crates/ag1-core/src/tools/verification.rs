//! A named release obligation, never proof that a claim is true.
use crate::DomainError;
use serde_json::Value;

pub fn scope(args: &Value) -> Result<Option<&str>, DomainError> {
    args.get("verificationScope").map(|value| {
        let scope = value.as_str().filter(|s| !s.is_empty() && s.len() <= 128 && s.trim() == *s && !s.chars().any(char::is_control))
            .ok_or_else(|| DomainError::InvalidInput("verificationScope must be a nonempty label of at most 128 bytes, without control characters or surrounding whitespace".into()))?;
        Ok(scope)
    }).transpose()
}
