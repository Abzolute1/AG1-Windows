//! P09 resource captures feed the P08 base prompt through the session owner.
use super::{Listener, NativeSession};
use ag1_core::{
    DomainError,
    context::prompt::{ContextFile, PromptInput, PromptSkill},
    runtime::prompt::PromptConfiguration,
    tools::registry::Registry,
};

impl NativeSession {
    pub(crate) fn client_prompt(
        workspace: &crate::workspace::Workspace,
        timestamp: &str,
        resources: &crate::resources::Snapshot,
        registry: Registry,
    ) -> Result<PromptConfiguration, DomainError> {
        configuration(workspace.root(), timestamp, resources, registry)?
            .with_workspace_snapshot(workspace_snapshot(workspace)?)
    }
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
        let prompt = configuration(self.root.root(), timestamp, resources, registry)?
            .with_workspace_snapshot(self.workspace_snapshot.clone())?;
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
    PromptConfiguration::new(input, registry, resources.fingerprint.clone())?
        .with_skills(resources.skill_bodies())
}

/// Best-effort, bounded grounding. Never follows links or treats this historical
/// listing as current source evidence. Root identity remains mandatory.
pub(super) fn workspace_snapshot(
    workspace: &crate::workspace::Workspace,
) -> Result<Option<String>, DomainError> {
    workspace.verify_root()?;
    let entries = workspace.directory_entries("");
    workspace.verify_root()?;
    let Ok(mut entries) = entries else {
        return Ok(None);
    };
    entries.retain(|entry| entry.name != ".git");
    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    let mut lines = Vec::new();
    for entry in entries.iter().take(100) {
        // Control characters must not turn a filename into several prompt lines.
        let name = if entry.name.chars().any(char::is_control) {
            serde_json::to_string(&entry.name).map_err(|_| DomainError::Conflict)?
        } else {
            entry.name.clone()
        };
        lines.push(format!("{name}{}", if entry.directory { "/" } else { "" }));
    }
    if entries.len() > lines.len() {
        lines.push(format!("… +{} more", entries.len() - lines.len()));
    }
    Ok((!lines.is_empty()).then(|| lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::WorkspaceService;

    #[test]
    fn workspace_listing_is_bounded_sorted_and_does_not_follow_links() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("line\nbreak"), "").unwrap();
        std::os::unix::fs::symlink("src", dir.path().join("link")).unwrap();
        for i in 0..105 {
            std::fs::write(dir.path().join(format!("file-{i:03}")), "").unwrap();
        }
        let workspace = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        let snapshot = workspace_snapshot(&workspace).unwrap().unwrap();
        let lines: Vec<_> = snapshot.lines().collect();
        assert_eq!(lines.len(), 101);
        assert_eq!(lines[0], "src/");
        assert_eq!(lines[1], "file-000");
        assert_eq!(lines[100], "… +8 more");
        assert!(!snapshot.contains(".git"));
        for i in 0..105 {
            std::fs::remove_file(dir.path().join(format!("file-{i:03}"))).unwrap();
        }
        assert_eq!(
            workspace_snapshot(&workspace).unwrap().unwrap(),
            "src/\n\"line\\nbreak\"\nlink"
        );
    }

    #[test]
    fn workspace_listing_omits_unrepresentable_names_but_rejects_replaced_root() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
        assert_eq!(workspace_snapshot(&workspace).unwrap(), None);
        std::fs::write(root.join(std::ffi::OsString::from_vec(vec![0xff])), "").unwrap();
        assert_eq!(workspace_snapshot(&workspace).unwrap(), None);
        std::fs::rename(&root, dir.path().join("old")).unwrap();
        std::fs::create_dir(&root).unwrap();
        assert!(matches!(
            workspace_snapshot(&workspace),
            Err(DomainError::Conflict)
        ));
    }
}
