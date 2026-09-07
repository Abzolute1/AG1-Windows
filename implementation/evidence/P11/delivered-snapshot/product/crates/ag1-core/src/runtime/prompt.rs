//! Captured resource instructions and tool metadata for a session's base prompt.
use crate::{DomainError, context::prompt::PromptInput, tools::registry::Registry};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct PromptConfiguration {
    input: PromptInput,
    registry: Registry,
    fingerprint: String,
}
impl PromptConfiguration {
    pub fn new(
        input: PromptInput,
        registry: Registry,
        fingerprint: String,
    ) -> Result<Self, DomainError> {
        crate::task::bounded_json(
            &serde_json::to_value(&input)
                .map_err(|_| DomainError::InvalidInput("prompt resources".into()))?,
        )?;
        if fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(DomainError::InvalidInput("resource fingerprint".into()));
        }
        Ok(Self {
            input,
            registry,
            fingerprint,
        })
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub(super) fn build(&self, model: &Value, tools: &[Value]) -> Result<String, DomainError> {
        let mut input = self.input.clone();
        input.model_provider = model["provider"].as_str().map(str::to_owned);
        input.context_window = model["contextWindow"].as_u64();
        let mut registry = self.registry.clone();
        let mut names = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = tool["name"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("prompt tool name".into()))?;
            let definition = registry
                .definitions()
                .find(|definition| definition.metadata["name"] == name)
                .ok_or_else(|| {
                    DomainError::InvalidInput("prompt tool definition missing".into())
                })?;
            if definition.metadata["parameters"] != tool["parameters"]
                || definition.metadata["description"] != tool["description"]
            {
                return Err(DomainError::Conflict);
            }
            names.push(name.to_owned());
        }
        let available: BTreeSet<_> = names.iter().cloned().collect();
        registry.activate(&names, &available)?;
        registry.build_prompt(&input)
    }
}
