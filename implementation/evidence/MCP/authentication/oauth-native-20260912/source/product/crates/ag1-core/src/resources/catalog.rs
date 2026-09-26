//! Deterministic resource selection. Host-verified bytes and trust grants are
//! inputs; this module cannot discover files, execute extensions or grant trust.
use super::{
    prompts::{SourceInfo, Template},
    skills::{Diagnostic, Skill},
};
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Skill,
    Prompt,
    Theme,
    Extension,
}
impl Kind {
    pub fn plural(self) -> &'static str {
        match self {
            Self::Skill => "skills",
            Self::Prompt => "prompts",
            Self::Theme => "themes",
            Self::Extension => "extensions",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub source: String,
    pub scope: String,
    pub origin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
}
impl Metadata {
    pub fn rank(&self) -> u8 {
        if self.origin == "package" {
            4
        } else {
            (if self.scope == "project" { 0 } else { 2 }) + u8::from(self.source != "local")
        }
    }
    pub fn source_info(&self, path: &str) -> SourceInfo {
        SourceInfo {
            path: path.into(),
            source: self.source.clone(),
            scope: self.scope.clone(),
            origin: self.origin.clone(),
            base_dir: self.base_dir.clone(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedResource {
    pub path: String,
    pub enabled: bool,
    pub metadata: Metadata,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    UntrustedData,
    TrustedCode,
}
/// Created by the host only after descriptor-based capture and root containment.
/// Original bytes remain available to the host's evidence/caching layer.
#[derive(Clone)]
pub struct CapturedResource {
    pub resource: ResolvedResource,
    pub kind: Kind,
    pub canonical_path: String,
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub trust: Trust,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeResource {
    pub name: String,
    pub source_path: String,
    pub source_info: SourceInfo,
    pub raw: Value,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionResource {
    pub path: String,
    pub sha256: String,
    pub source_info: SourceInfo,
    pub trust: Trust,
}
#[derive(Clone, Default, Serialize)]
pub struct Catalog {
    pub skills: Vec<Skill>,
    pub prompts: Vec<Template>,
    pub themes: Vec<ThemeResource>,
    pub extensions: Vec<ExtensionResource>,
    pub diagnostics: Vec<Diagnostic>,
    pub fingerprint: String,
}
fn collision(kind: &str, name: &str, path: &str, winner: &str) -> Diagnostic {
    Diagnostic {
        kind: "collision".into(),
        message: format!(
            "name \"{}{name}\" collision",
            if kind == "prompt" { "/" } else { "" }
        ),
        path: path.into(),
        collision: Some(
            json!({"resourceType":kind,"name":name,"winnerPath":winner,"loserPath":path}),
        ),
    }
}
fn value_cost(value: &Value) -> usize {
    std::mem::size_of::<Value>()
        + match value {
            Value::String(s) => s.len(),
            Value::Array(items) => items.iter().map(value_cost).sum(),
            Value::Object(items) => items
                .iter()
                .map(|(k, v)| k.len() + 64 + value_cost(v))
                .sum(),
            _ => 0,
        }
}
fn source_cost(source: &SourceInfo) -> usize {
    source.path.len()
        + source.source.len()
        + source.scope.len()
        + source.origin.len()
        + source.base_dir.as_ref().map_or(0, String::len)
        + std::mem::size_of::<SourceInfo>()
}
fn diagnostic_cost(d: &Diagnostic) -> usize {
    d.kind.len()
        + d.message.len()
        + d.path.len()
        + std::mem::size_of::<Diagnostic>()
        + d.collision.as_ref().map_or(0, value_cost)
}
fn charge_new(
    c: &Catalog,
    collisions: &[Diagnostic],
    counts: &mut [usize; 6],
    total: &mut usize,
) -> Result<(), DomainError> {
    for s in &c.skills[counts[0]..] {
        *total += std::mem::size_of::<Skill>()
            + s.name.len()
            + s.description.len()
            + s.file_path.len()
            + s.base_dir.len()
            + s.triggers
                .iter()
                .map(|t| t.len() + std::mem::size_of::<String>())
                .sum::<usize>()
            + s.source_info.as_ref().map_or(0, source_cost);
    }
    for p in &c.prompts[counts[1]..] {
        *total += std::mem::size_of::<Template>()
            + p.name.len()
            + p.description.len()
            + p.content.len()
            + p.file_path.len()
            + p.source_info.as_ref().map_or(0, source_cost);
    }
    for t in &c.themes[counts[2]..] {
        *total += std::mem::size_of::<ThemeResource>()
            + t.name.len()
            + t.source_path.len()
            + source_cost(&t.source_info)
            + value_cost(&t.raw);
    }
    for e in &c.extensions[counts[3]..] {
        *total += std::mem::size_of::<ExtensionResource>()
            + e.path.len()
            + e.sha256.len()
            + source_cost(&e.source_info);
    }
    *total += c.diagnostics[counts[4]..]
        .iter()
        .map(diagnostic_cost)
        .sum::<usize>()
        + collisions[counts[5]..]
            .iter()
            .map(diagnostic_cost)
            .sum::<usize>();
    *counts = [
        c.skills.len(),
        c.prompts.len(),
        c.themes.len(),
        c.extensions.len(),
        c.diagnostics.len(),
        collisions.len(),
    ];
    if *total > 32 * 1024 * 1024 {
        return Err(DomainError::LimitExceeded(
            "retained resource catalog".into(),
        ));
    }
    Ok(())
}
/// Validate a native host-derived view without granting paths or execution trust.
/// This runs before the loader can publish callback output as a new generation.
pub fn validate_view(catalog: &Catalog) -> Result<(), DomainError> {
    if catalog.fingerprint.len() > 256 {
        return Err(DomainError::LimitExceeded("resource fingerprint".into()));
    }
    if catalog.skills.len()
        + catalog.prompts.len()
        + catalog.themes.len()
        + catalog.extensions.len()
        > 4096
        || catalog.diagnostics.len() > 4096
    {
        return Err(DomainError::LimitExceeded("resource view entries".into()));
    }
    for theme in &catalog.themes {
        crate::tools::schema::bounded(&theme.raw, 1024 * 1024, 64)?;
        super::themes::parse(&theme.raw.to_string())?;
    }
    for diagnostic in &catalog.diagnostics {
        if let Some(value) = &diagnostic.collision {
            crate::tools::schema::bounded(value, 1024 * 1024, 64)?;
        }
    }
    let mut counts = [0; 6];
    let mut total = 0;
    charge_new(catalog, &[], &mut counts, &mut total)?;
    Ok(())
}
pub fn compile(resources: &[CapturedResource]) -> Result<Catalog, DomainError> {
    compile_with_notes(resources, &[])
}
/// Host discovery diagnostics retain their position among parse warnings. Name
/// collisions follow all discovery/parse warnings, matching the source loader.
pub fn compile_with_notes(
    resources: &[CapturedResource],
    notes: &[(usize, Diagnostic)],
) -> Result<Catalog, DomainError> {
    if notes.len() > 4096
        || notes.windows(2).any(|w| w[0].0 > w[1].0)
        || notes.iter().any(|(i, _)| *i > resources.len())
        || notes.iter().map(|(_, d)| diagnostic_cost(d)).sum::<usize>() > 4 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded(
            "resource discovery diagnostics".into(),
        ));
    }
    let mut notes = notes.iter().peekable();
    if resources.len() > 4096
        || resources.iter().map(|r| r.bytes.len()).sum::<usize>() > 32 * 1024 * 1024
    {
        return Err(DomainError::LimitExceeded("resource catalog bytes".into()));
    }
    let mut catalog = Catalog {
        skills: vec![],
        prompts: vec![],
        themes: vec![],
        extensions: vec![],
        diagnostics: vec![],
        fingerprint: String::new(),
    };
    let mut seen_skills = BTreeSet::new();
    let mut seen_extensions = BTreeSet::new();
    let mut collisions = vec![];
    let mut fingerprint = Sha256::new();
    let mut counts = [0; 6];
    let mut retained = 0;
    for (index, input) in resources.iter().enumerate() {
        while notes.peek().is_some_and(|(i, _)| *i == index) {
            catalog.diagnostics.push(notes.next().unwrap().1.clone());
        }
        charge_new(&catalog, &collisions, &mut counts, &mut retained)?;
        if input.bytes.len() > 1024 * 1024
            || input.resource.path.len() > 8192
            || input.canonical_path.len() > 8192
            || input.resource.metadata.source.len() > 8192
            || input
                .resource
                .metadata
                .base_dir
                .as_ref()
                .is_some_and(|s| s.len() > 8192)
            || !["user", "project", "temporary"].contains(&input.resource.metadata.scope.as_str())
            || !["package", "top-level"].contains(&input.resource.metadata.origin.as_str())
        {
            return Err(DomainError::LimitExceeded("resource document".into()));
        }
        if format!("{:x}", Sha256::digest(&input.bytes)) != input.sha256 {
            return Err(DomainError::Conflict);
        }
        fingerprint.update(
            serde_json::to_vec(&json!([
                input.resource,
                input.kind,
                input.canonical_path,
                input.sha256,
                input.trust
            ]))
            .map_err(|_| DomainError::InvalidInput("resource identity".into()))?,
        );
        if !input.resource.enabled {
            continue;
        }
        let path = &input.resource.path;
        let source = input.resource.metadata.source_info(path);
        let text = String::from_utf8_lossy(&input.bytes);
        match input.kind {
            Kind::Skill => {
                let loaded = super::skills::load(path, &text, Some(source));
                catalog.diagnostics.extend(loaded.diagnostics);
                if let Some(skill) = loaded.skill {
                    if seen_skills.contains(&input.canonical_path) {
                        continue;
                    }
                    if let Some(existing) = catalog.skills.iter().find(|s| s.name == skill.name) {
                        collisions.push(collision("skill", &skill.name, path, &existing.file_path));
                    } else {
                        seen_skills.insert(input.canonical_path.clone());
                        catalog.skills.push(skill);
                    }
                }
            }
            Kind::Prompt => match super::prompts::template(path, &text, Some(source)) {
                Ok(template) => {
                    if let Some(existing) = catalog.prompts.iter().find(|p| p.name == template.name)
                    {
                        collisions.push(collision(
                            "prompt",
                            &template.name,
                            path,
                            &existing.file_path,
                        ));
                    } else {
                        catalog.prompts.push(template);
                    }
                }
                Err(e) => catalog.diagnostics.push(Diagnostic {
                    kind: "warning".into(),
                    message: format!("failed to parse prompt: {e:?}"),
                    path: path.clone(),
                    collision: None,
                }),
            },
            Kind::Theme => match super::themes::parse(&text) {
                Ok(theme) => {
                    let name = theme.name;
                    if let Some(existing) = catalog.themes.iter().find(|t| t.name == name) {
                        collisions.push(collision("theme", &name, path, &existing.source_path));
                    } else {
                        catalog.themes.push(ThemeResource {
                            name,
                            source_path: path.clone(),
                            source_info: source,
                            raw: theme.raw,
                        });
                    }
                }
                Err(error) => catalog.diagnostics.push(Diagnostic {
                    kind: "warning".into(),
                    message: format!("failed to load theme: {error:?}"),
                    path: path.clone(),
                    collision: None,
                }),
            },
            Kind::Extension => {
                if !seen_extensions.insert(input.canonical_path.clone()) {
                    continue;
                }
                catalog.extensions.push(ExtensionResource {
                    path: path.clone(),
                    sha256: input.sha256.clone(),
                    source_info: source,
                    trust: input.trust,
                });
            }
        }
    }
    catalog.diagnostics.extend(notes.map(|(_, d)| d.clone()));
    charge_new(&catalog, &collisions, &mut counts, &mut retained)?;
    catalog.diagnostics.extend(collisions);
    catalog.fingerprint = format!("{:x}", fingerprint.finalize());
    Ok(catalog)
}
/// Preserve discovery order on equal ranks; CLI paths are supplied by the host
/// before these ranked paths, exactly as in the source resource loader.
pub fn order(resources: &mut [ResolvedResource]) {
    resources.sort_by_key(|r| r.metadata.rank());
}
/// Reload swaps only a fully built candidate; caller retires previous extension
/// processes before admitting executable entries under the new generation.
pub struct CatalogState {
    generation: u64,
    catalog: Catalog,
}
impl CatalogState {
    pub fn new(catalog: Catalog) -> Self {
        Self {
            generation: 1,
            catalog,
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }
    pub fn reload(&mut self, expected: u64, candidate: Catalog) -> Result<u64, DomainError> {
        if self.generation != expected {
            return Err(DomainError::Conflict);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| DomainError::LimitExceeded("resource generation".into()))?;
        self.catalog = candidate;
        self.generation = generation;
        Ok(generation)
    }
}
