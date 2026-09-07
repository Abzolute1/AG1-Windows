//! Optional admitted extension callbacks. The mutex owns one already trusted
//! process; callbacks borrow the current native journal, never a second writer.
use super::tool_journal::ToolJournal;
use crate::{
    extension_host::{DenyBridge, ExtensionHost},
    resource_fs::ScanControl,
};
use ag1_core::{DomainError, tools::registry::Scope};
use serde_json::{Value, json};
use std::sync::Mutex;
mod resources;
pub(super) mod snapshot;

pub struct NativeExtensions {
    host: Mutex<ExtensionHost>,
    catalog: Value,
    resources: Mutex<Option<crate::resources::ResourceLoader>>,
}
impl NativeExtensions {
    /// Install this admitted host's complete static tool catalog atomically.
    /// Selection remains a separate native configuration/approval operation.
    pub fn install_tools(
        &self,
        registry: &mut ag1_core::tools::registry::Registry,
    ) -> Result<(), DomainError> {
        registry.register_extensions(
            self.catalog["tools"]
                .as_array()
                .ok_or(DomainError::Conflict)?,
        )
    }
    pub fn tool_names(&self) -> Vec<String> {
        self.catalog["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect()
    }
    pub(super) fn target_revision(&self) -> String {
        format!(
            "extension-{}",
            ag1_storage::digest(self.catalog.to_string().as_bytes())
        )
    }
    pub(super) fn prepare_tool(
        &self,
        journal: &ToolJournal,
        registry: &ag1_core::tools::registry::Registry,
        scope: &Scope,
        call: &ag1_core::tools::registry::ValidatedCall,
        control: &ScanControl<'_>,
    ) -> Result<Value, DomainError> {
        if !self.catalog["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|t| t["name"] == call.name() && t["hasPrepareArguments"] == true)
        {
            return Ok(call.args().clone());
        }
        self.invoke_tool(journal, registry, scope, call, "tool.prepare", control)?
            .ok_or_else(|| DomainError::InvalidInput("extension prepared arguments".into()))
    }
    pub(super) fn invoke_tool(
        &self,
        journal: &ToolJournal,
        registry: &ag1_core::tools::registry::Registry,
        scope: &Scope,
        call: &ag1_core::tools::registry::ValidatedCall,
        method: &str,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        if call.origin() != ag1_core::tools::registry::Origin::Extension
            || !self.tool_names().iter().any(|name| name == call.name())
        {
            return Err(DomainError::PermissionDenied);
        }
        let branch = journal.snapshot(scope)?;
        let stored = journal.stored_snapshot(scope)?;
        let (configuration, _) = journal.model(scope)?;
        let mut snapshot = snapshot::configuration(
            &configuration,
            snapshot::session(&stored, &branch)?,
            false,
            journal.pending_messages()?,
        );
        snapshot["allTools"] = json!(registry.public_tools());
        let mut host = self.host.lock().map_err(|_| DomainError::Conflict)?;
        journal.snapshot(scope)?;
        host.advance_scope(scope)?;
        self.check_catalog(&mut host, scope, control)?;
        let result = host.call_with_snapshot(
            scope,
            method,
            &json!({"name":call.name(),"callId":call.call_id(),"arguments":call.args()}),
            &snapshot,
            control,
            &mut DenyBridge,
        )?;
        self.check_catalog(&mut host, scope, control)?;
        Ok(result)
    }
    pub fn bind(mut host: ExtensionHost, control: &ScanControl<'_>) -> Result<Self, DomainError> {
        let scope = host.scope().clone();
        let catalog = host
            .call(
                &scope,
                "bind",
                &json!({"snapshot":{}}),
                control,
                &mut DenyBridge,
            )?
            .ok_or_else(|| DomainError::InvalidInput("extension catalog missing".into()))?;
        Ok(Self {
            host: Mutex::new(host),
            catalog,
            resources: Mutex::new(None),
        })
    }
    pub fn catalog(&self) -> &Value {
        &self.catalog
    }
    pub fn handles(&self, event: &str) -> bool {
        self.catalog["handlers"]
            .as_array()
            .is_some_and(|handlers| handlers.iter().any(|name| name == event))
    }
    pub fn stop(&self) -> Result<(), DomainError> {
        self.host.lock().map_err(|_| DomainError::Conflict)?.stop()
    }
    pub(super) fn tool_event(
        &self,
        journal: &ToolJournal,
        registry: &ag1_core::tools::registry::Registry,
        scope: &Scope,
        event: Value,
        control: &ScanControl<'_>,
    ) -> Result<Option<Value>, DomainError> {
        if !self.catalog["handlers"]
            .as_array()
            .is_some_and(|handlers| handlers.iter().any(|name| name == &event["type"]))
        {
            return Ok(None);
        }
        control.check()?;
        let branch = journal.snapshot(scope)?;
        let stored = journal.stored_snapshot(scope)?;
        let (configuration, _) = journal.model(scope)?;
        let mut snapshot = snapshot::configuration(
            &configuration,
            snapshot::session(&stored, &branch)?,
            false,
            journal.pending_messages()?,
        );
        snapshot["allTools"] = json!(registry.public_tools());
        self.event_with_snapshot(scope, &snapshot, &event, control, || {
            journal.snapshot(scope).map(|_| ())
        })
    }
    pub(super) fn event_with_snapshot(
        &self,
        scope: &Scope,
        snapshot: &Value,
        event: &Value,
        control: &ScanControl<'_>,
        validate: impl FnOnce() -> Result<(), DomainError>,
    ) -> Result<Option<Value>, DomainError> {
        if !self.handles(event["type"].as_str().unwrap_or("")) {
            return Ok(None);
        }
        control.check()?;
        let mut host = self.host.lock().map_err(|_| DomainError::Conflict)?;
        // Revalidate after acquiring the host: another accepted control may
        // have advanced configuration while this callback waited its turn.
        validate()?;
        host.advance_scope(scope)?;
        let result = host.call_with_snapshot(
            scope,
            "event",
            &json!({"event":event}),
            snapshot,
            control,
            &mut DenyBridge,
        )?;
        self.check_catalog(&mut host, scope, control)?;
        Ok(result)
    }
    fn check_catalog(
        &self,
        host: &mut ExtensionHost,
        scope: &Scope,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        let catalog = host
            .call(scope, "catalog", &json!({}), control, &mut DenyBridge)?
            .ok_or(DomainError::Conflict)?;
        for key in [
            "generation",
            "registrationRevision",
            "extensions",
            "tools",
            "commands",
            "flags",
            "shortcuts",
            "renderers",
            "handlers",
        ] {
            if catalog.get(key) != self.catalog.get(key) {
                return Err(DomainError::Conflict);
            }
        }
        Ok(())
    }
}
