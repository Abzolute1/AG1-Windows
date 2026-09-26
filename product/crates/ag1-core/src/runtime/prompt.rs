//! Captured resource instructions and tool metadata for a session's base prompt.
use crate::{DomainError, context::prompt::PromptInput, tools::registry::Registry};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct PromptConfiguration {
    input: PromptInput,
    registry: Registry,
    fingerprint: String,
    skills: std::sync::Arc<Vec<(crate::resources::skills::Skill, String)>>,
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
            skills: Default::default(),
        })
    }
    /// Historical workspace observation captured by the native session owner.
    pub fn workspace_snapshot(&self) -> Option<&str> {
        self.input.workspace_snapshot.as_deref()
    }
    pub fn with_workspace_snapshot(self, snapshot: Option<String>) -> Result<Self, DomainError> {
        let mut input = self.input;
        input.workspace_snapshot = snapshot;
        Self::new(input, self.registry, self.fingerprint)?.with_skills((*self.skills).clone())
    }
    /// Only host-captured skill bytes are admitted; never read model-supplied paths.
    pub fn with_skills(mut self, skills: Vec<(crate::resources::skills::Skill, String)>) -> Result<Self, DomainError> {
        if skills.len() > 4096 || skills.iter().map(|(_, body)| body.len()).sum::<usize>() > 32 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("captured skills".into()));
        }
        self.skills = std::sync::Arc::new(skills);
        Ok(self)
    }
    pub fn expand_skill(&self, text: &str) -> Result<String, DomainError> {
        if text.trim_start().starts_with("<skill ") { return Ok(text.into()); }
        // Large valid user messages retain the pre-existing admission contract.
        // Automatic matching is bounded; explicit selections still report limits.
        if !text.trim_start().starts_with("/skill:") && (self.skills.is_empty() || text.len() > 1024 * 1024) { return Ok(text.into()); }
        let (selected, request) = if let Some(rest) = text.trim_start().strip_prefix("/skill:") {
            let split = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let name = &rest[..split];
            let selected = self.skills.iter().position(|(skill, _)| skill.name == name)
                .ok_or_else(|| DomainError::Unavailable(format!("Skill '{name}' is not loaded; refresh resources or start a new chat")))?;
            (Some(selected), rest[split..].trim_start())
        } else {
            let catalog: Vec<_> = self.skills.iter().map(|(skill, _)| skill.clone()).collect();
            let matched = crate::resources::skills::match_prompt(text, &catalog)?;
            (matched.and_then(|skill| self.skills.iter().position(|(s, _)| s.file_path == skill.file_path)), text)
        };
        let Some(index) = selected else { return Ok(text.into()); };
        let (skill, content) = &self.skills[index];
        let body = crate::resources::frontmatter::parse(content)?.body;
        fn attribute(s: &str) -> String { s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;") }
        let expanded = format!("<skill name=\"{}\" location=\"{}\">\nSkill instructions are already loaded. Resolve relative references against {}.\n{}\n</skill>\n\n{}", attribute(&skill.name), attribute(&skill.file_path), skill.base_dir, body, request);
        if expanded.len() > 1024 * 1024 { return Err(DomainError::LimitExceeded("expanded skill prompt (1 MiB)".into())); }
        Ok(expanded)
    }
    /// Overlay only native-validated MCP identities; all other captured tool
    /// metadata and resource instructions retain their original authority.
    pub fn with_mcp_selection(&self, selected: &[crate::tools::mcp::SelectedDefinition]) -> Result<Self, DomainError> {
        let mut next = self.clone();
        next.registry.replace_mcp_selection(selected)?;
        Ok(next)
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub(super) fn build(&self, model: &Value, tools: &[Value]) -> Result<String, DomainError> {
        let mut input = self.input.clone();
        input.model_provider = model["provider"].as_str().map(str::to_owned);
        input.context_window = model["contextWindow"].as_u64();
        let mut registry = self.registry.clone();
        let native = registry.native_tools();
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
            let legacy_matches = definition.metadata["parameters"] == tool["parameters"]
                && definition.metadata["description"] == tool["description"];
            // Admit only the two exact host-derived presentations: saved SDK
            // metadata and current native descriptions. Never arbitrary schema edits.
            let native_matches = native.iter().any(|expected| expected["name"] == name
                && expected["parameters"] == tool["parameters"]
                && expected["description"] == tool["description"]);
            if !legacy_matches && !native_matches {
                return Err(DomainError::Conflict);
            }
            names.push(name.to_owned());
        }
        let available: BTreeSet<_> = names.iter().cloned().collect();
        registry.activate(&names, &available)?;
        registry.build_native_prompt(&input, model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn captured_skill_activation_is_explicit_bounded_and_idempotent() {
        let body = "---\nname: svg-art\ndescription: Draw SVGs\ntriggers: [pelican]\n---\nPatch the existing SVG. UNIQUE_SKILL_BODY";
        let mut skill = crate::resources::skills::load("/work/skills/svg-art/SKILL.md", body, None).skill.unwrap();
        let mut prompt = PromptConfiguration::new(PromptInput::default(), Registry::builtins().unwrap(), "a".repeat(64)).unwrap().with_skills(vec![(skill.clone(),body.into())]).unwrap().with_workspace_snapshot(Some("snapshot".into())).unwrap();
        let expanded = prompt.expand_skill("Draw a pelican").unwrap();
        assert!(expanded.contains("UNIQUE_SKILL_BODY"));
        assert!(expanded.contains("/work/skills/svg-art"));
        assert_eq!(prompt.expand_skill(&expanded).unwrap(), expanded);
        assert_eq!(crate::task::canonical_task(&expanded).unwrap(), "Draw a pelican");
        assert_eq!(prompt.expand_skill("Hello").unwrap(), "Hello");
        let large = "a".repeat(1024 * 1024 + 1);
        assert_eq!(prompt.expand_skill(&large).unwrap(), large);
        skill.disable_model_invocation = true;
        prompt = prompt.with_skills(vec![(skill,body.into())]).unwrap();
        assert_eq!(prompt.expand_skill("Draw a pelican").unwrap(), "Draw a pelican");
        assert!(prompt.expand_skill("/skill:svg-art\nDraw a bird").unwrap().contains("UNIQUE_SKILL_BODY"));
        assert!(prompt.expand_skill("/skill:unknown test").is_err());
    }
    #[test]
    fn native_and_saved_metadata_are_exact_and_forged_definitions_still_fail() {
        let registry = Registry::builtins().unwrap();
        let legacy = registry.public_tools().into_iter().find(|t| t["name"] == "read").unwrap();
        let native = registry.native_tools().into_iter().find(|t| t["name"] == "read").unwrap();
        let input = PromptInput { date: "2026-09-10".into(), cwd: "/work/fixture".into(), ..Default::default() };
        let prompt = PromptConfiguration::new(input, registry, "a".repeat(64)).unwrap();
        let model = json!({"id":"fixture","provider":"fixture"});
        prompt.build(&model, &[legacy]).unwrap();
        prompt.build(&model, &[native.clone()]).unwrap();
        let mut forged = native.clone(); forged["description"] = json!("Arbitrary replacement");
        assert!(matches!(prompt.build(&model, &[forged]), Err(DomainError::Conflict)));
        let mut forged = native; forged["parameters"]["required"] = json!([]);
        assert!(matches!(prompt.build(&model, &[forged]), Err(DomainError::Conflict)));
    }
}
