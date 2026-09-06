//! P09 resource captures feed the P08 base prompt through the session owner.
use super::{Listener, NativeSession};
use ag1_core::{
    DomainError,
    context::prompt::{ContextFile, PromptInput, PromptSkill},
    runtime::prompt::PromptConfiguration,
    tools::registry::Registry,
};

impl NativeSession {
    /// The host supplies a fully captured P09 resource snapshot and the registry
    /// whose concrete adapters it owns. Resource discovery never grants execution.
    pub async fn install_resources(
        &self,
        command: &str,
        timestamp: &str,
        resources: &crate::resources::Snapshot,
        registry: Registry,
        listener: &dyn Listener,
    ) -> Result<(), DomainError> {
        let _activity = self.enter()?;
        let prompt = configuration(self.root.root(), timestamp, resources, registry)?;
        let action = self
            .state()?
            .install_prompt(command, command, timestamp, prompt)?;
        self.commit(action, listener).await
    }
}

pub(super) fn configuration(
    cwd: &std::path::Path,
    timestamp: &str,
    resources: &crate::resources::Snapshot,
    registry: Registry,
) -> Result<PromptConfiguration, DomainError> {
    let date = timestamp
        .get(..10)
        .ok_or_else(|| DomainError::InvalidInput("resource date".into()))?;
    let cwd = cwd
        .to_str()
        .ok_or_else(|| DomainError::InvalidInput("workspace path".into()))?;
    let input = PromptInput {
        date: date.into(),
        cwd: cwd.into(),
        custom_prompt: resources.system.clone(),
        append_system_prompt: (!resources.append_system.is_empty())
            .then(|| resources.append_system.join("\n\n")),
        context_files: resources
            .agents_files
            .iter()
            .map(|file| ContextFile {
                path: file.path.clone(),
                content: file.content.clone(),
            })
            .collect(),
        skills: resources
            .catalog
            .skills
            .iter()
            .map(|skill| PromptSkill {
                name: skill.name.clone(),
                description: skill.description.clone(),
                file_path: skill.file_path.clone(),
                disable_model_invocation: skill.disable_model_invocation,
            })
            .collect(),
        ..Default::default()
    };
    PromptConfiguration::new(input, registry, resources.fingerprint.clone())
}
