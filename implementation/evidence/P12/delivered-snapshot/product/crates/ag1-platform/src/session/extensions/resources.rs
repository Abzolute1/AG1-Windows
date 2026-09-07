use super::*;
use ag1_core::{runtime::prompt::PromptConfiguration, tools::registry::Registry};
use std::path::Path;

impl NativeExtensions {
    /// Attach the already loaded native resource owner during service
    /// preparation. This does not execute callbacks or authorize extensions.
    pub fn with_resources(
        self,
        loader: crate::resources::ResourceLoader,
    ) -> Result<Self, DomainError> {
        if loader.snapshot().is_none() {
            return Err(DomainError::Unavailable(
                "resources have not been loaded".into(),
            ));
        }
        *self.resources.lock().map_err(|_| DomainError::Conflict)? = Some(loader);
        Ok(self)
    }

    pub(in crate::session) fn discover_resources(
        &self,
        scope: &Scope,
        snapshot: &Value,
        registry: Registry,
        timestamp: &str,
        control: &ScanControl<'_>,
    ) -> Result<Option<PromptConfiguration>, DomainError> {
        if !self.handles("resources_discover") {
            return Ok(None);
        }
        let mut host = self.host.lock().map_err(|_| DomainError::Conflict)?;
        let mut resources = self.resources.lock().map_err(|_| DomainError::Conflict)?;
        let loader = resources
            .as_mut()
            .ok_or_else(|| DomainError::Unavailable("native extension resource owner".into()))?;
        let cwd = Path::new(
            snapshot["session"]["cwd"]
                .as_str()
                .ok_or(DomainError::Conflict)?,
        );
        if loader.workspace_root() != cwd {
            return Err(DomainError::PermissionDenied);
        }
        host.advance_scope(scope)?;
        host.verify_workspace(cwd)?;
        let result = host
            .call_with_snapshot(
                scope,
                "event",
                &json!({"event":{"type":"resources_discover","cwd":cwd,"reason":"startup"}}),
                snapshot,
                control,
                &mut DenyBridge,
            )?
            .ok_or(DomainError::Conflict)?;
        self.check_catalog(&mut host, scope, control)?;
        let paths = crate::resources::extension_resource_paths(&result, &host)?;
        if ["skillPaths", "promptPaths", "themePaths"]
            .iter()
            .all(|key| result[*key].as_array().is_none_or(Vec::is_empty))
        {
            return Ok(None);
        }
        host.verify_workspace(cwd)?;
        loader.extend_resources(loader.generation(), paths, control)?;
        super::super::prompt::configuration(
            cwd,
            timestamp,
            loader.snapshot().ok_or(DomainError::Conflict)?,
            registry,
        )
        .map(Some)
    }
}
