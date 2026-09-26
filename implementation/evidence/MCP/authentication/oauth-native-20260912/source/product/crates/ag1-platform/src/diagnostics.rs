use ag1_core::DomainError;
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub level: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}
pub fn cargo_diagnostics(bytes: &[u8]) -> Result<Vec<Diagnostic>, DomainError> {
    if bytes.len() > 1024 * 1024 {
        return Err(DomainError::LimitExceeded("compiler diagnostics".into()));
    }
    let mut diagnostics = vec![];
    for line in bytes.split(|b| *b == b'\n').filter(|line| !line.is_empty()) {
        if line.len() > 64 * 1024 {
            return Err(DomainError::LimitExceeded("compiler line".into()));
        }
        let value: Value = serde_json::from_slice(line)
            .map_err(|_| DomainError::InvalidInput("compiler JSON".into()))?;
        if value["reason"] != "compiler-message" {
            continue;
        }
        if diagnostics.len() >= 20 {
            return Err(DomainError::LimitExceeded(
                "compiler diagnostics count".into(),
            ));
        }
        let message = value["message"]["message"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("compiler message".into()))?;
        let level = value["message"]["level"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("compiler level".into()))?;
        let span = value["message"]["spans"]
            .as_array()
            .and_then(|spans| spans.iter().find(|span| span["is_primary"] == true));
        diagnostics.push(Diagnostic {
            message: message.chars().take(240).collect(),
            level: level.chars().take(32).collect(),
            file: span
                .and_then(|s| s["file_name"].as_str())
                .map(|s| s.chars().take(4096).collect()),
            line: span
                .and_then(|s| s["line_start"].as_u64())
                .and_then(|n| u32::try_from(n).ok()),
        });
    }
    Ok(diagnostics)
}

#[derive(Debug, PartialEq, Eq)]
pub enum TargetKind {
    Web,
    Rust,
    Tauri,
    Godot,
}

#[derive(Debug)]
pub struct ManifestSummary {
    pub entry_points: Vec<String>,
    pub scripts: std::collections::BTreeMap<String, String>,
    pub package_manager_files: Vec<String>,
    pub languages: std::collections::BTreeMap<String, usize>,
    pub truncated: bool,
}

/// Bounded map inputs for A37. Full source/compiler semantics are later core work.
#[cfg(target_os = "linux")]
pub fn manifest_summary(
    workspace: &crate::workspace::Workspace,
    index: &crate::workspace::SourceIndex,
) -> Result<ManifestSummary, DomainError> {
    use std::collections::BTreeMap;
    if &index.workspace != workspace.id() || index.files.len() > 1000 {
        return Err(DomainError::Conflict);
    }
    let mut result = ManifestSummary {
        entry_points: vec![],
        scripts: BTreeMap::new(),
        package_manager_files: vec![],
        languages: BTreeMap::new(),
        truncated: false,
    };
    for path in index.files.keys() {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("other");
        if result.languages.len() < 64 || result.languages.contains_key(extension) {
            *result
                .languages
                .entry(extension.chars().take(32).collect())
                .or_default() += 1;
        } else {
            result.truncated = true;
        }
        if [
            "package.json",
            "package-lock.json",
            "pnpm-lock.yaml",
            "yarn.lock",
            "bun.lockb",
            "Cargo.toml",
            "Cargo.lock",
        ]
        .contains(&path.as_str())
        {
            result.package_manager_files.push(path.clone());
        }
    }
    if let Some(revision) = index.files.get("package.json") {
        let snapshot = workspace.snapshot("package.json", 1024 * 1024)?;
        if &snapshot.revision != revision {
            return Err(DomainError::Conflict);
        }
        let value: Value = serde_json::from_slice(&snapshot.bytes)
            .map_err(|_| DomainError::InvalidInput("package manifest JSON".into()))?;
        for key in ["main", "module", "types"] {
            if let Some(path) = value[key].as_str() {
                if path.len() > 4096 {
                    return Err(DomainError::LimitExceeded("manifest path".into()));
                }
                result.entry_points.push(path.into());
            }
        }
        if let Some(scripts) = value["scripts"].as_object() {
            result.truncated |= scripts.len() > 5;
            for (key, value) in scripts.iter().take(5) {
                if key.len() > 128 || value.as_str().is_some_and(|s| s.len() > 4096) {
                    return Err(DomainError::LimitExceeded("package script".into()));
                }
                if let Some(command) = value.as_str() {
                    result.scripts.insert(key.clone(), command.into());
                }
            }
        }
    }
    Ok(result)
}
/// Explicit manifest names from a bounded workspace index. This performs no builds or package installs.
pub fn discover_targets(paths: &[String]) -> Result<Vec<TargetKind>, DomainError> {
    if paths.len() > 1000 {
        return Err(DomainError::LimitExceeded("target discovery".into()));
    }
    let mut targets = vec![];
    for (manifest, kind) in [
        ("package.json", TargetKind::Web),
        ("Cargo.toml", TargetKind::Rust),
        ("src-tauri/tauri.conf.json", TargetKind::Tauri),
        ("project.godot", TargetKind::Godot),
    ] {
        if paths.iter().any(|path| path == manifest) {
            targets.push(kind);
        }
    }
    Ok(targets)
}
