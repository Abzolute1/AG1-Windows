//! Package settings edits use the same prepared, approval-bound file CAS as the
//! native write tool. Unknown settings and package filter fields are preserved.
use crate::{
    file_tools::{FileExecution, FileOutcome, FileService, PreparedFile},
    resources::resolve_path,
    workspace::Workspace,
};
use ag1_core::{
    DomainError,
    resources::packages::{self, Source},
    tools::{
        approval::AuthorizedCall,
        registry::{Registry, Scope, ValidatedCall},
    },
};
use serde_json::{Value, json};
use std::{
    path::{Component, Path},
    sync::Arc,
};
#[derive(Clone, Copy)]
pub enum Change {
    Add,
    Remove,
}
fn relative(base: &Path, path: &Path) -> String {
    let base = base.components().collect::<Vec<_>>();
    let path = path.components().collect::<Vec<_>>();
    let common = base.iter().zip(&path).take_while(|(a, b)| a == b).count();
    let mut parts = vec!["..".to_owned(); base.len() - common];
    parts.extend(path[common..].iter().filter_map(|p| {
        if let Component::Normal(s) = p {
            Some(s.to_string_lossy().into_owned())
        } else {
            None
        }
    }));
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}
fn identity(source: &str, base: &Path, home: &Path) -> Result<String, DomainError> {
    let parsed = packages::parse(source)?;
    let local = match &parsed {
        Source::Local { path } => resolve_path(base, path, home)?
            .to_string_lossy()
            .into_owned(),
        _ => String::new(),
    };
    Ok(parsed.identity(&local))
}
pub fn edit_document(
    document: &Value,
    change: Change,
    source: &str,
    cwd: &Path,
    settings_base: &Path,
    home: &Path,
) -> Result<Option<Value>, DomainError> {
    packages::validate_settings_document(document)?;
    let mut object = document
        .as_object()
        .cloned()
        .ok_or_else(|| DomainError::InvalidInput("settings object".into()))?;
    let current = match object.get("packages") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| DomainError::InvalidInput("settings packages".into()))?
            .clone(),
        None => vec![],
    };
    if current.len() > 256 {
        return Err(DomainError::LimitExceeded("configured packages".into()));
    }
    let key = identity(source, cwd, home)?;
    let mut next = vec![];
    let mut found = false;
    for package in &current {
        let value = package
            .as_str()
            .or_else(|| package.get("source").and_then(Value::as_str))
            .ok_or_else(|| DomainError::InvalidInput("configured package source".into()))?;
        let matched = identity(value, settings_base, home)? == key;
        found |= matched;
        if !matched || matches!(change, Change::Add) {
            next.push(package.clone());
        }
    }
    match change {
        Change::Add if !found => {
            if next.len() >= 256 {
                return Err(DomainError::LimitExceeded("configured packages".into()));
            }
            let normalized = match packages::parse(source)? {
                Source::Local { path } => relative(settings_base, &resolve_path(cwd, &path, home)?),
                _ => source.into(),
            };
            next.push(Value::String(normalized));
        }
        Change::Remove if found => {}
        _ => return Ok(None),
    }
    object.insert("packages".into(), Value::Array(next));
    Ok(Some(Value::Object(object)))
}
pub struct PackageSettings {
    workspace: Arc<Workspace>,
    files: FileService,
    path: String,
    cwd: std::path::PathBuf,
    base: std::path::PathBuf,
    home: std::path::PathBuf,
}
pub struct PreparedSettings {
    file: PreparedFile,
    call: ValidatedCall,
    document: Value,
}
impl PreparedSettings {
    pub fn call(&self) -> &ValidatedCall {
        &self.call
    }
    pub fn target_revision(&self) -> &str {
        self.file.target_revision()
    }
    pub fn document(&self) -> &Value {
        &self.document
    }
}
impl PackageSettings {
    pub fn new(
        workspace: Arc<Workspace>,
        path: &str,
        cwd: &Path,
        home: &Path,
    ) -> Result<Self, DomainError> {
        let files = FileService::new(workspace.clone());
        let path = files.relative_path(path)?;
        if path.is_empty() || !cwd.is_absolute() || !home.is_absolute() {
            return Err(DomainError::InvalidInput("package settings scope".into()));
        }
        let base = workspace
            .root()
            .join(&path)
            .parent()
            .ok_or_else(|| DomainError::InvalidInput("settings parent".into()))?
            .to_path_buf();
        Ok(Self {
            workspace,
            files,
            path,
            cwd: cwd.into(),
            base,
            home: home.into(),
        })
    }
    pub fn prepare(
        &self,
        registry: &Registry,
        scope: &Scope,
        operation: &str,
        change: Change,
        source: &str,
    ) -> Result<Option<PreparedSettings>, DomainError> {
        if scope.workspace != self.workspace.id().as_str() {
            return Err(DomainError::PermissionDenied);
        }
        let snapshot = match self.workspace.snapshot(&self.path, 1024 * 1024) {
            Ok(value) => Some(value),
            Err(DomainError::NotFound) => None,
            Err(error) => return Err(error),
        };
        let document = match &snapshot {
            Some(s) => serde_json::from_slice(&s.bytes)
                .map_err(|_| DomainError::InvalidInput("settings JSON".into()))?,
            None => json!({}),
        };
        let Some(document) =
            edit_document(&document, change, source, &self.cwd, &self.base, &self.home)?
        else {
            return Ok(None);
        };
        let content = serde_json::to_string_pretty(&document)
            .map_err(|_| DomainError::InvalidInput("settings serialization".into()))?
            + "\n";
        if content.len() > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("settings bytes".into()));
        }
        let call = registry.prepare(
            operation,
            operation,
            "write",
            &json!({"path":self.path,"content":content}),
            true,
        )?;
        let file = self.files.prepare(&call, scope)?;
        if file.source_revision() != snapshot.as_ref().map(|s| &s.revision) {
            return Err(DomainError::Conflict);
        }
        Ok(Some(PreparedSettings {
            file,
            call,
            document,
        }))
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        prepared: PreparedSettings,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<FileOutcome, DomainError> {
        self.files.execute(prepared.file, authorization, execution)
    }
}
