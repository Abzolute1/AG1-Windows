use super::*;
use crate::{
    resource_fs::{ResourceFs, ScanControl},
    resources::{Paths, ResourceLoader},
};
impl Host {
    pub(super) fn resources(&self) -> Result<crate::resources::Snapshot> {
        load(
            &self.working_workspace()?,
            &self.state,
            &self.config.system_prompt,
            self.config.prompt_append.as_deref(),
            self.config.packages.as_ref(),
        )
    }
    pub(super) fn resource_action(&self, text: &str) -> Result<w::Command> {
        match actions::parse(text) {
            Ok(command) => Ok(command),
            Err(DomainError::Unavailable(_)) => {
                let resources = self.resources()?;
                if text.starts_with("/skill:") {
                    let prompt = crate::session::NativeSession::client_prompt(
                        self.working_workspace()?.as_ref(),
                        &timestamp(),
                        &resources,
                        ag1_core::tools::registry::Registry::builtins()?,
                    )?;
                    return Ok(w::Command::Prompt {
                        input: w::PromptInput::Inline {
                            text: prompt.expand_skill(text)?,
                        },
                    });
                }
                let (name, arguments) = text
                    .trim_start_matches('/')
                    .split_once(' ')
                    .unwrap_or((text.trim_start_matches('/'), ""));
                if let Some(extension) = self
                    .registry
                    .selected()?
                    .services
                    .tools()
                    .client_extensions()
                    && extension.catalog()["commands"]
                        .as_array()
                        .is_some_and(|commands| {
                            commands.iter().any(|c| c["invocationName"] == name)
                        })
                {
                    return Ok(w::Command::Runtime {
                        action: "extension_command".into(),
                        args: json!({"name":name,"arguments":arguments}),
                    });
                }
                let expanded =
                    ag1_core::resources::prompts::expand(text, &resources.catalog.prompts)?;
                if expanded == text {
                    return Err(DomainError::Unavailable(
                        "Unknown slash action or template".into(),
                    ));
                }
                Ok(w::Command::Prompt {
                    input: w::PromptInput::Inline { text: expanded },
                })
            }
            Err(error) => Err(error),
        }
    }
}

pub(super) fn load(
    workspace: &Arc<Workspace>,
    state: &Path,
    system: &str,
    append: Option<&str>,
    packages: Option<&resource_actions::Packages>,
) -> Result<crate::resources::Snapshot> {
    let home = state.join("home");
    let agent = home.join(".ag1/agent");
    fs::create_dir_all(&agent).map_err(crate::io_error)?;
    let fs = ResourceFs::new(vec![
        workspace.clone(),
        WorkspaceService::new()?.register(&home)?,
    ])?;
    let mut installed = vec![];
    if let Some(options) = packages {
        let service = resource_actions::package_service(workspace.clone(), state, options)?;
        let cancel = AtomicBool::new(false);
        for (root, path, scope) in [
            (
                workspace.clone(),
                ".ag1/settings.json",
                ag1_core::resources::package_lifecycle::Scope::Project,
            ),
            (
                WorkspaceService::new()?.register(&home)?,
                ".ag1/agent/settings.json",
                ag1_core::resources::package_lifecycle::Scope::User,
            ),
        ] {
            let document = match root.snapshot(path, 1024 * 1024) {
                Ok(s) => serde_json::from_slice::<Value>(&s.bytes).map_err(json_error)?,
                Err(DomainError::NotFound) => json!({}),
                Err(e) => return Err(e),
            };
            if let Some(entries) = document["packages"].as_array() {
                if entries.len() > 256 {
                    return Err(DomainError::LimitExceeded("configured packages".into()));
                }
                for entry in entries {
                    let source = entry
                        .as_str()
                        .or_else(|| entry["source"].as_str())
                        .ok_or_else(|| DomainError::InvalidInput("package source".into()))?;
                    if matches!(
                        ag1_core::resources::packages::parse(source)?,
                        ag1_core::resources::packages::Source::Local { .. }
                    ) {
                        continue;
                    }
                    if let Some(record) = service.installation_record(
                        source,
                        scope,
                        ScanControl {
                            cancel: &cancel,
                            deadline: std::time::Instant::now() + Duration::from_secs(10),
                        },
                    )? {
                        installed.push(record);
                    }
                }
            }
        }
    }
    let loader = ResourceLoader::new(
        fs,
        crate::resources::Config {
            cwd: workspace.root().into(),
            agent_dir: agent,
            home,
            additional: Paths::default(),
            disabled: vec![],
            installed,
            system: (!system.is_empty()).then(|| system.to_owned()),
            append_system: None,
        },
    )?;
    let mut snapshot = loader.build(&ScanControl {
        cancel: &AtomicBool::new(false),
        deadline: std::time::Instant::now() + Duration::from_secs(10),
    })?;
    // Host conversation context augments discovered APPEND_SYSTEM.md files;
    // passing it as the loader override would silently hide those instructions.
    if let Some(append) = append.filter(|s| !s.is_empty()) {
        if append.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("host prompt appendix".into()));
        }
        snapshot.fingerprint = ag1_storage::digest(
            &serde_json::to_vec(&("host-prompt-append-v1", &snapshot.fingerprint, append))
                .map_err(json_error)?,
        );
        snapshot.append_system.push(append.to_owned());
    }
    Ok(snapshot)
}
