//! Native resource assembly. Settings and manifests are data; discovering an
//! extension never starts it or turns a manifest field into an execution grant.
use crate::{
    resource_fs::{Discovery, ResourceFs, ScanControl, validate_diagnostics},
    resource_patterns::{Filters, is_pattern},
};
use ag1_core::{
    DomainError,
    resources::{
        catalog::{self, Catalog, Kind, Metadata, ResolvedResource, Trust},
        skills::Diagnostic,
    },
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
};
const KINDS: [Kind; 4] = [Kind::Extension, Kind::Skill, Kind::Prompt, Kind::Theme];
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiscoveryReason {
    Startup,
    Reload,
}
pub(crate) fn extension_resource_paths(
    result: &Value,
    host: &crate::extension_host::ExtensionHost,
) -> Result<Paths, DomainError> {
    let fields = result
        .as_object()
        .ok_or_else(|| DomainError::InvalidInput("extension resource discovery result".into()))?;
    if fields
        .keys()
        .any(|key| !["skillPaths", "promptPaths", "themePaths"].contains(&key.as_str()))
    {
        return Err(DomainError::PermissionDenied);
    }
    let mut paths = Paths::default();
    for (key, kind) in [
        ("skillPaths", Kind::Skill),
        ("promptPaths", Kind::Prompt),
        ("themePaths", Kind::Theme),
    ] {
        let Some(entries) = fields.get(key) else {
            continue;
        };
        let entries = entries
            .as_array()
            .ok_or_else(|| DomainError::InvalidInput("extension resource paths".into()))?;
        if entries.len() > 256 {
            return Err(DomainError::LimitExceeded(
                "extension resource paths".into(),
            ));
        }
        for entry in entries {
            let path = entry["path"]
                .as_str()
                .ok_or_else(|| DomainError::InvalidInput("extension resource path".into()))?;
            let extension = entry["extensionPath"]
                .as_str()
                .ok_or(DomainError::PermissionDenied)?;
            if !host.admits_extension(extension) {
                return Err(DomainError::PermissionDenied);
            }
            let owner = Path::new(extension);
            let base = owner
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or(DomainError::PermissionDenied)?;
            let name = base
                .strip_suffix(".ts")
                .or_else(|| base.strip_suffix(".js"))
                .unwrap_or(base);
            let metadata = Metadata {
                source: format!("extension:{name}"),
                scope: "temporary".into(),
                origin: "top-level".into(),
                base_dir: Some(text(owner.parent().ok_or(DomainError::PermissionDenied)?)?),
            };
            paths.add(kind, path.into(), &metadata, true)?;
        }
    }
    paths.check_bound()?;
    Ok(paths)
}
fn push_note(
    notes: &mut Vec<(usize, Diagnostic)>,
    note: (usize, Diagnostic),
) -> Result<(), DomainError> {
    validate_diagnostics(notes.iter().map(|n| &n.1).chain(std::iter::once(&note.1)))?;
    notes.push(note);
    Ok(())
}
fn metadata_cost(metadata: &Metadata) -> usize {
    std::mem::size_of::<Metadata>()
        + metadata.source.len()
        + metadata.scope.len()
        + metadata.origin.len()
        + metadata.base_dir.as_ref().map_or(0, String::len)
}
fn validate_resource(path: &str, metadata: &Metadata) -> Result<(), DomainError> {
    if path.len() > 8192
        || path.contains('\0')
        || metadata.source.len() > 8192
        || metadata.base_dir.as_ref().is_some_and(|v| v.len() > 8192)
        || !["user", "project", "temporary"].contains(&metadata.scope.as_str())
        || !["package", "top-level"].contains(&metadata.origin.as_str())
    {
        return Err(DomainError::InvalidInput("resource metadata".into()));
    }
    Ok(())
}
fn capture_cost(resource: &catalog::CapturedResource) -> usize {
    std::mem::size_of::<catalog::CapturedResource>()
        + resource.bytes.len()
        + resource.resource.path.len()
        + resource.canonical_path.len()
        + resource.sha256.len()
        + metadata_cost(&resource.resource.metadata)
}
fn text(path: &Path) -> Result<String, DomainError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DomainError::InvalidInput("resource path encoding".into()))
}
pub fn resolve_path(base: &Path, input: &str, home: &Path) -> Result<PathBuf, DomainError> {
    if input.len() > 8192 || input.contains('\0') {
        return Err(DomainError::InvalidInput("resource path".into()));
    }
    let input = input.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let path = if input == "~" {
        home.to_path_buf()
    } else if let Some(tail) = input.strip_prefix("~/") {
        home.join(tail)
    } else if let Some(tail) = input.strip_prefix('~') {
        home.join(tail)
    } else {
        base.join(input)
    };
    if !path.is_absolute() {
        return Err(DomainError::InvalidInput("absolute resource base".into()));
    }
    let mut out = PathBuf::from("/");
    for component in path.components() {
        match component {
            Component::Normal(s) => out.push(s),
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::CurDir => {}
            Component::Prefix(_) => {
                return Err(DomainError::InvalidInput("resource prefix".into()));
            }
        }
    }
    Ok(out)
}
#[derive(Clone, Default, Serialize)]
pub struct Paths {
    pub extensions: Vec<ResolvedResource>,
    pub skills: Vec<ResolvedResource>,
    pub prompts: Vec<ResolvedResource>,
    pub themes: Vec<ResolvedResource>,
}
impl Paths {
    fn check_bound(&self) -> Result<(), DomainError> {
        let mut count = 0usize;
        let mut bytes = 0usize;
        for resource in KINDS.iter().flat_map(|k| self.get(*k)) {
            validate_resource(&resource.path, &resource.metadata)?;
            count += 1;
            bytes += resource.path.len()
                + metadata_cost(&resource.metadata)
                + std::mem::size_of::<ResolvedResource>();
            if count > 4096 || bytes > 4 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("retained resource paths".into()));
            }
        }
        Ok(())
    }
    pub fn get(&self, kind: Kind) -> &[ResolvedResource] {
        match kind {
            Kind::Extension => &self.extensions,
            Kind::Skill => &self.skills,
            Kind::Prompt => &self.prompts,
            Kind::Theme => &self.themes,
        }
    }
    fn get_mut(&mut self, kind: Kind) -> &mut Vec<ResolvedResource> {
        match kind {
            Kind::Extension => &mut self.extensions,
            Kind::Skill => &mut self.skills,
            Kind::Prompt => &mut self.prompts,
            Kind::Theme => &mut self.themes,
        }
    }
    fn add(
        &mut self,
        kind: Kind,
        path: String,
        metadata: &Metadata,
        enabled: bool,
    ) -> Result<(), DomainError> {
        validate_resource(&path, metadata)?;
        let retained = KINDS
            .iter()
            .flat_map(|k| self.get(*k))
            .map(|r| {
                r.path.len() + metadata_cost(&r.metadata) + std::mem::size_of::<ResolvedResource>()
            })
            .sum::<usize>();
        if retained + path.len() + metadata_cost(metadata) + std::mem::size_of::<ResolvedResource>()
            > 4 * 1024 * 1024
        {
            return Err(DomainError::LimitExceeded("retained resource paths".into()));
        }
        if self.extensions.len() + self.skills.len() + self.prompts.len() + self.themes.len()
            >= 4096
        {
            return Err(DomainError::LimitExceeded("resolved resources".into()));
        }
        let items = self.get_mut(kind);
        if !items.iter().any(|r| r.path == path) {
            items.push(ResolvedResource {
                path,
                enabled,
                metadata: metadata.clone(),
            });
        }
        Ok(())
    }
}
/// Host-pinned installation records, not deserializable extension input. The
/// installer supplies the normalized identity and registered root after settlement.
pub struct InstalledPackage {
    pub source: String,
    pub scope: String,
    pub identity: String,
    pub root: PathBuf,
}
pub struct Config {
    pub cwd: PathBuf,
    pub agent_dir: PathBuf,
    pub home: PathBuf,
    pub additional: Paths,
    pub disabled: Vec<Kind>,
    pub installed: Vec<InstalledPackage>,
    pub system: Option<String>,
    pub append_system: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct ContextFile {
    pub path: String,
    pub content: String,
}
pub struct Snapshot {
    pub override_evidence: Option<crate::resource_overrides::Evidence>,
    pub catalog: Catalog,
    pub paths: Paths,
    pub agents_files: Vec<ContextFile>,
    pub system: Option<String>,
    pub append_system: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub fingerprint: String,
    captured: Vec<catalog::CapturedResource>,
    loaded_paths: Paths,
    contributions: Paths,
    extension_notes: Vec<(Kind, usize, Diagnostic)>,
}
impl Snapshot {
    pub(crate) fn skill_bodies(&self) -> Vec<(ag1_core::resources::skills::Skill, String)> {
        self.catalog
            .skills
            .iter()
            .filter_map(|skill| {
                self.captured
                    .iter()
                    .find(|c| c.kind == Kind::Skill && c.canonical_path == skill.file_path)
                    .and_then(|c| String::from_utf8(c.bytes.clone()).ok())
                    .map(|body| (skill.clone(), body))
            })
            .collect()
    }
}
pub struct ResourceLoader {
    overrides: Option<crate::resource_overrides::RegisteredOverride>,
    fs: ResourceFs,
    config: Config,
    generation: u64,
    current: Option<Snapshot>,
}
struct Build<'a> {
    loader: &'a ResourceLoader,
    control: &'a ScanControl<'a>,
    diagnostics: Vec<Diagnostic>,
    observed: Vec<(PathBuf, String, PathBuf)>,
}
fn entries(value: Option<&Value>) -> Result<Vec<String>, DomainError> {
    let Some(value) = value else {
        return Ok(vec![]);
    };
    let values = value
        .as_array()
        .ok_or_else(|| DomainError::InvalidInput("resource paths must be an array".into()))?;
    if values.len() > 256 {
        return Err(DomainError::LimitExceeded("resource path entries".into()));
    }
    values
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| s.len() <= 8192)
                .map(str::to_owned)
                .ok_or_else(|| DomainError::InvalidInput("resource path entry".into()))
        })
        .collect()
}
impl ResourceLoader {
    pub(crate) fn workspace_root(&self) -> &Path {
        &self.config.cwd
    }
    /// Resolve explicitly supplied package resources without auto-discovery,
    /// installation, extension execution or publication. Non-local packages must
    /// already have a host-pinned installation record. Callers still capture and
    /// authorize extension entries before loading them into a helper.
    pub fn resolve_sources(
        &self,
        sources: &[Value],
        scope: ag1_core::resources::package_lifecycle::Scope,
        control: &ScanControl<'_>,
    ) -> Result<(Paths, Vec<Diagnostic>), DomainError> {
        use ag1_core::resources::package_lifecycle::Scope;
        control.check()?;
        if sources.len() > 256 {
            return Err(DomainError::LimitExceeded("resource packages".into()));
        }
        let project = self.config.cwd.join(".ag1");
        let (scope, base) = match scope {
            Scope::User => ("user", &self.config.agent_dir),
            Scope::Project => ("project", &project),
            Scope::Temporary => ("temporary", &self.config.cwd),
        };
        let settings = serde_json::json!({"packages": sources});
        let mut build = Build {
            loader: self,
            control,
            diagnostics: vec![],
            observed: vec![],
        };
        let paths = build.package_scopes(&[(scope, base, &settings)])?;
        for (path, hash, canonical) in &build.observed {
            let (_, revision, current) = self.fs.read(path, 1024 * 1024, control)?;
            if &revision.sha256 != hash || &current != canonical {
                return Err(DomainError::Conflict);
            }
        }
        control.check()?;
        Ok((paths, build.diagnostics))
    }

    /// Replace host-observed installation records and reload under one generation
    /// fence. Rejected/stale/cancelled candidates retain both the prior published
    /// snapshot and the prior installation map. This never installs a package.
    pub fn reload_installed(
        &mut self,
        expected: u64,
        installed: Vec<InstalledPackage>,
        control: &ScanControl<'_>,
    ) -> Result<u64, DomainError> {
        control.check()?;
        if expected != self.generation {
            return Err(DomainError::Conflict);
        }
        if installed.len() > 256 {
            return Err(DomainError::LimitExceeded("installed packages".into()));
        }
        for record in &installed {
            if record.source.len() > 8192
                || record.identity.len() > 16384
                || !["user", "project", "temporary"].contains(&record.scope.as_str())
                || !record.root.is_absolute()
                || record.root.as_os_str().len() > 8192
            {
                return Err(DomainError::InvalidInput("installed package record".into()));
            }
        }
        let previous = std::mem::replace(&mut self.config.installed, installed);
        match self.reload(expected, control) {
            Ok(generation) => Ok(generation),
            Err(error) => {
                self.config.installed = previous;
                Err(error)
            }
        }
    }
    pub fn new(fs: ResourceFs, config: Config) -> Result<Self, DomainError> {
        config.additional.check_bound()?;
        for path in [&config.cwd, &config.agent_dir, &config.home] {
            if !path.is_absolute() || path.as_os_str().len() > 8192 {
                return Err(DomainError::InvalidInput(
                    "resource configuration root".into(),
                ));
            }
        }
        if !fs.is_directory(&config.cwd)? || config.installed.len() > 256 {
            return Err(DomainError::InvalidInput("resource configuration".into()));
        }
        if config
            .system
            .as_ref()
            .is_some_and(|s| s.len() > 1024 * 1024)
            || config
                .append_system
                .as_ref()
                .is_some_and(|s| s.len() > 1024 * 1024)
        {
            return Err(DomainError::LimitExceeded("system prompt input".into()));
        }
        Ok(Self {
            fs,
            config,
            generation: 0,
            overrides: None,
            current: None,
        })
    }
    /// Native SDK embedding only; JSON resource settings cannot install callbacks.
    pub fn with_overrides(
        mut self,
        registration: crate::resource_overrides::RegisteredOverride,
    ) -> Result<Self, DomainError> {
        if self.generation != 0 || self.current.is_some() {
            return Err(DomainError::Conflict);
        }
        self.overrides = Some(registration);
        Ok(self)
    }
    fn apply_overrides(
        &self,
        mut candidate: Snapshot,
        changed: &[Kind],
        full: bool,
        control: &ScanControl<'_>,
    ) -> Result<Snapshot, DomainError> {
        let Some(registration) = &self.overrides else {
            return Ok(candidate);
        };
        use crate::resource_overrides::View;
        let mut input = View {
            catalog: std::mem::take(&mut candidate.catalog),
            agents_files: std::mem::take(&mut candidate.agents_files),
            system: candidate.system.take(),
            append_system: std::mem::take(&mut candidate.append_system),
        };
        let mut baseline = None;
        if !full && let Some(current) = &self.current {
            let mut source = current
                .override_evidence
                .as_ref()
                .map(|e| e.original.as_ref().clone())
                .ok_or(DomainError::Conflict)?;
            for kind in KINDS {
                if changed.contains(&kind) {
                    match kind {
                        Kind::Skill => source.catalog.skills = input.catalog.skills.clone(),
                        Kind::Prompt => source.catalog.prompts = input.catalog.prompts.clone(),
                        Kind::Theme => source.catalog.themes = input.catalog.themes.clone(),
                        Kind::Extension => {
                            source.catalog.extensions = input.catalog.extensions.clone()
                        }
                    }
                } else {
                    match kind {
                        Kind::Skill => input.catalog.skills = current.catalog.skills.clone(),
                        Kind::Prompt => input.catalog.prompts = current.catalog.prompts.clone(),
                        Kind::Theme => input.catalog.themes = current.catalog.themes.clone(),
                        Kind::Extension => {
                            input.catalog.extensions = current.catalog.extensions.clone()
                        }
                    }
                }
            }
            source.catalog.diagnostics = input.catalog.diagnostics.clone();
            // Preserve diagnostics belonging to untouched resource groups. The
            // native override view combines group diagnostics; unknown host notes
            // survive until an override explicitly replaces them or a full reload.
            let mut owners = BTreeMap::new();
            for resource in &current.captured {
                owners.insert(resource.resource.path.as_str(), resource.kind);
            }
            for skill in &current.catalog.skills {
                owners
                    .entry(skill.file_path.as_str())
                    .or_insert(Kind::Skill);
            }
            for prompt in &current.catalog.prompts {
                owners
                    .entry(prompt.file_path.as_str())
                    .or_insert(Kind::Prompt);
            }
            for theme in &current.catalog.themes {
                owners
                    .entry(theme.source_path.as_str())
                    .or_insert(Kind::Theme);
            }
            for extension in &current.catalog.extensions {
                owners
                    .entry(extension.path.as_str())
                    .or_insert(Kind::Extension);
            }
            let diagnostic_key = |d: &Diagnostic| -> Result<String, DomainError> {
                Ok(format!(
                    "{:x}",
                    Sha256::digest(
                        serde_json::to_vec(d)
                            .map_err(|_| DomainError::InvalidInput("resource diagnostic".into()))?
                    )
                ))
            };
            let mut known = input
                .catalog
                .diagnostics
                .iter()
                .map(diagnostic_key)
                .collect::<Result<BTreeSet<_>, _>>()?;
            for diagnostic in &current.catalog.diagnostics {
                control.check()?;
                if owners
                    .get(diagnostic.path.as_str())
                    .is_none_or(|kind| !changed.contains(kind))
                    && known.insert(diagnostic_key(diagnostic)?)
                {
                    validate_diagnostics(
                        input
                            .catalog
                            .diagnostics
                            .iter()
                            .chain(std::iter::once(diagnostic)),
                    )?;
                    input.catalog.diagnostics.push(diagnostic.clone());
                }
            }
            source.catalog.fingerprint = input.catalog.fingerprint.clone();
            baseline = Some(source);
        }
        let (view, evidence) = crate::resource_overrides::apply(
            registration,
            input,
            baseline,
            changed,
            full,
            control,
        )?;
        candidate.catalog = view.catalog;
        candidate.agents_files = view.agents_files;
        candidate.system = view.system;
        candidate.append_system = view.append_system;
        candidate.fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &candidate.fingerprint,
                    &evidence.name,
                    &evidence.before_sha256,
                    &evidence.after_sha256,
                    &candidate.catalog.fingerprint
                ))
                .map_err(|_| DomainError::InvalidInput("resource override identity".into()))?
            )
        );
        candidate.override_evidence = Some(evidence);
        Ok(candidate)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.current.as_ref()
    }
    /// Invoke an already admitted host's data-resource hooks, validate provenance,
    /// and publish through the ordinary atomic resource-generation boundary.
    /// Session lifecycle events and system-prompt rebuilding remain the P10 caller's job.
    pub fn discover_extensions(
        &mut self,
        expected: u64,
        host: &mut crate::extension_host::ExtensionHost,
        scope: &ag1_core::tools::registry::Scope,
        reason: DiscoveryReason,
        control: &ScanControl<'_>,
        bridge: &mut impl crate::extension_host::Bridge,
    ) -> Result<u64, DomainError> {
        control.check()?;
        if expected != self.generation || host.scope() != scope {
            return Err(DomainError::Conflict);
        }
        if self.current.is_none() {
            return Err(DomainError::Unavailable(
                "resources have not been loaded".into(),
            ));
        }
        host.verify_workspace(&self.config.cwd)?;
        let result = host
            .call(
                scope,
                "event",
                &serde_json::json!({"event":{"type":"resources_discover","cwd":self.config.cwd,"reason":reason}}),
                control,
                bridge,
            )?
            .ok_or_else(|| DomainError::InvalidInput("extension resource discovery result".into()))?;
        let additions = extension_resource_paths(&result, host)?;
        host.verify_workspace(&self.config.cwd)?;
        self.extend_resources(expected, additions, control)
    }
    /// Build and revalidate the complete candidate before one generation swap.
    /// Errors, cancellation and stale caller generations leave the old snapshot.
    pub fn reload(&mut self, expected: u64, control: &ScanControl<'_>) -> Result<u64, DomainError> {
        control.check()?;
        if expected != self.generation {
            return Err(DomainError::Conflict);
        }
        let candidate = self.build(control)?;
        control.check()?;
        let next = self
            .generation
            .checked_add(1)
            .ok_or_else(|| DomainError::LimitExceeded("resource generation".into()))?;
        self.current = Some(candidate);
        self.generation = next;
        Ok(next)
    }
    pub fn build(&self, control: &ScanControl<'_>) -> Result<Snapshot, DomainError> {
        let mut build = Build {
            loader: self,
            control,
            diagnostics: vec![],
            observed: vec![],
        };
        let paths = build.resolve()?;
        let mut captured = vec![];
        let mut captured_bytes = 0;
        let mut loaded_paths = Paths::default();
        for kind in KINDS {
            let selected = if self.config.disabled.contains(&kind) {
                vec![]
            } else {
                paths.get(kind).to_vec()
            };
            let mut seen = BTreeSet::new();
            // Explicit extension entries precede discovery so their tool and flag
            // definitions win. Additional data resources retain discovery order.
            let additional = self.config.additional.get(kind).to_vec();
            let ordered = if kind == Kind::Extension {
                additional.into_iter().chain(selected).collect::<Vec<_>>()
            } else {
                selected.into_iter().chain(additional).collect::<Vec<_>>()
            };
            for resource in ordered {
                if !resource.enabled || !seen.insert(resource.path.clone()) {
                    continue;
                }
                loaded_paths.add(kind, resource.path.clone(), &resource.metadata, true)?;
                if !self.fs.exists(Path::new(&resource.path))? {
                    build.note(Diagnostic {
                        kind: "error".into(),
                        message: format!("{} path does not exist", kind.plural()),
                        path: resource.path,
                        collision: None,
                    })?;
                    continue;
                }
                let mode = match kind {
                    Kind::Prompt => Discovery::DirectPrompts,
                    Kind::Theme => Discovery::Themes,
                    Kind::Skill => Discovery::Skills {
                        root_markdown: true,
                    },
                    Kind::Extension => Discovery::Extensions,
                };
                let scan = build.scan(Path::new(&resource.path), mode)?;
                for path in scan {
                    let mut resource = resource.clone();
                    resource.path = path;
                    let capture = self
                        .fs
                        .capture(resource, kind, Trust::UntrustedData, control)?;
                    build.observed.push((
                        PathBuf::from(&capture.resource.path),
                        capture.sha256.clone(),
                        PathBuf::from(&capture.canonical_path),
                    ));
                    captured_bytes += capture_cost(&capture);
                    captured.push(capture);
                    if captured.len() > 4096 || captured_bytes > 32 * 1024 * 1024 {
                        return Err(DomainError::LimitExceeded("resource capture bytes".into()));
                    }
                }
            }
        }
        let catalog = catalog::compile(&captured)?;
        let mut agents_files = vec![];
        let mut context_bytes = 0usize;
        let mut seen = BTreeSet::new();
        if let Some(file) = build.context(&self.config.agent_dir)? {
            seen.insert(file.path.clone());
            context_bytes += file.path.len() + file.content.len();
            if context_bytes > 32 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("resource context bytes".into()));
            }
            agents_files.push(file);
        }
        let mut ancestors = vec![];
        let mut dir = self.config.cwd.as_path();
        loop {
            if self.fs.admits_path(dir)
                && let Some(file) = build.context(dir)?
                && seen.insert(file.path.clone())
            {
                context_bytes += file.path.len() + file.content.len();
                if context_bytes > 32 * 1024 * 1024 {
                    return Err(DomainError::LimitExceeded("resource context bytes".into()));
                }
                ancestors.push(file);
            }
            let Some(parent) = dir.parent() else { break };
            dir = parent;
        }
        ancestors.reverse();
        agents_files.extend(ancestors);
        if agents_files
            .iter()
            .map(|f| f.path.len() + f.content.len())
            .sum::<usize>()
            > 32 * 1024 * 1024
        {
            return Err(DomainError::LimitExceeded("resource context bytes".into()));
        }
        let system = build.prompt(self.config.system.as_deref(), "SYSTEM.md")?;
        let append_system = build
            .prompt(self.config.append_system.as_deref(), "APPEND_SYSTEM.md")?
            .filter(|s| !s.is_empty())
            .into_iter()
            .collect::<Vec<_>>();
        control.check()?;
        if build.observed.len() > 8192 {
            return Err(DomainError::LimitExceeded("resource observations".into()));
        }
        // Re-read every captured file through its capability, including settings
        // and manifests. A changed source cannot silently enter this generation.
        for (path, hash, canonical) in &build.observed {
            let (_, revision, current_canonical) = self.fs.read(path, 1024 * 1024, control)?;
            if revision.sha256 != *hash || *canonical != current_canonical {
                return Err(DomainError::Conflict);
            }
        }
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    catalog.fingerprint.clone(),
                    &agents_files,
                    &system,
                    &append_system,
                    &build.observed
                ))
                .map_err(|_| DomainError::InvalidInput("resource snapshot identity".into()))?
            )
        );
        let candidate = Snapshot {
            override_evidence: None,
            catalog,
            paths,
            agents_files,
            system,
            append_system,
            diagnostics: build.diagnostics,
            fingerprint,
            captured,
            loaded_paths,
            contributions: Paths::default(),
            extension_notes: vec![],
        };
        let candidate = self.apply_overrides(candidate, &KINDS, true, control)?;
        if self.overrides.is_some() {
            for (path, hash, canonical) in &build.observed {
                let (_, revision, current) = self.fs.read(path, 1024 * 1024, control)?;
                if revision.sha256 != *hash || current != *canonical {
                    return Err(DomainError::Conflict);
                }
            }
        }
        Ok(candidate)
    }
    /// Host-admitted extension contributions can add data resources, never code.
    /// Only the affected resource kinds are refreshed. Other captured resources,
    /// context files and system prompts remain the current generation's bytes.
    pub fn extend_resources(
        &mut self,
        expected: u64,
        additions: Paths,
        control: &ScanControl<'_>,
    ) -> Result<u64, DomainError> {
        control.check()?;
        if expected != self.generation {
            return Err(DomainError::Conflict);
        }
        if !additions.extensions.is_empty() {
            return Err(DomainError::PermissionDenied);
        }
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| DomainError::Unavailable("resources have not been loaded".into()))?;
        let changed: Vec<_> = [Kind::Skill, Kind::Prompt, Kind::Theme]
            .into_iter()
            .filter(|k| !additions.get(*k).is_empty())
            .collect();
        if changed.is_empty() {
            return Ok(self.generation);
        }
        let mut loaded = current.loaded_paths.clone();
        let mut contributions = current.contributions.clone();
        for kind in &changed {
            if additions.get(*kind).len() > 256 {
                return Err(DomainError::LimitExceeded(
                    "extension resource paths".into(),
                ));
            }
            for resource in additions.get(*kind) {
                validate_resource(&resource.path, &resource.metadata)?;
                if !resource.enabled {
                    return Err(DomainError::InvalidInput(
                        "disabled extension contribution".into(),
                    ));
                }
                let path = text(&resolve_path(
                    &self.config.cwd,
                    &resource.path,
                    &self.config.home,
                )?)?;
                // Metadata remains untrusted provenance data, not an execution grant.
                // Validate it through the normal catalog before publication.
                if let Some(old) = contributions
                    .get_mut(*kind)
                    .iter_mut()
                    .find(|r| r.path == path)
                {
                    old.metadata = resource.metadata.clone();
                } else {
                    contributions.add(*kind, path.clone(), &resource.metadata, true)?;
                }
                loaded.add(*kind, path, &resource.metadata, true)?;
            }
        }
        let mut captured = vec![];
        let mut retained_bytes = 0usize;
        loaded.check_bound()?;
        contributions.check_bound()?;
        let mut notes = vec![];
        let mut extension_notes = vec![];
        let mut observed = vec![];
        for kind in KINDS {
            let group_start = captured.len();
            let notes_start = notes.len();
            if !changed.contains(&kind) {
                for (_, offset, note) in current.extension_notes.iter().filter(|n| n.0 == kind) {
                    push_note(&mut notes, (group_start + offset, note.clone()))?;
                    extension_notes.push((kind, *offset, note.clone()));
                }
                for input in current.captured.iter().filter(|r| r.kind == kind) {
                    retained_bytes += capture_cost(input);
                    if captured.len() >= 4096 || retained_bytes > 32 * 1024 * 1024 {
                        return Err(DomainError::LimitExceeded(
                            "extended resource capture".into(),
                        ));
                    }
                    captured.push(input.clone());
                }
                continue;
            }
            let mut seen = BTreeSet::new();
            for resource in loaded.get(kind) {
                control.check()?;
                if !self.fs.exists(Path::new(&resource.path))? {
                    if kind != Kind::Prompt {
                        push_note(
                            &mut notes,
                            (
                                captured.len(),
                                Diagnostic {
                                    kind: if kind == Kind::Skill {
                                        "error"
                                    } else {
                                        "warning"
                                    }
                                    .into(),
                                    message: format!(
                                        "{} path does not exist",
                                        if kind == Kind::Skill {
                                            "skill"
                                        } else {
                                            "theme"
                                        }
                                    ),
                                    path: resource.path.clone(),
                                    collision: None,
                                },
                            ),
                        )?;
                    }
                    continue;
                }
                let mode = match kind {
                    Kind::Skill => Discovery::Skills {
                        root_markdown: true,
                    },
                    Kind::Prompt => Discovery::DirectPrompts,
                    Kind::Theme => Discovery::Themes,
                    Kind::Extension => unreachable!(),
                };
                let scan = self.fs.scan(
                    Path::new(&resource.path),
                    mode,
                    ScanControl {
                        cancel: control.cancel,
                        deadline: control.deadline,
                    },
                )?;
                for note in scan.diagnostics {
                    push_note(&mut notes, (captured.len(), note))?;
                }
                for path in scan.paths {
                    if !seen.insert(path.clone()) {
                        continue;
                    }
                    let mut resource = resource.clone();
                    resource.path = path;
                    if let Some(owner) = contributions
                        .get(kind)
                        .iter()
                        .find(|r| Path::new(&resource.path).starts_with(&r.path))
                    {
                        resource.metadata = owner.metadata.clone();
                    }
                    let input = self
                        .fs
                        .capture(resource, kind, Trust::UntrustedData, control)?;
                    retained_bytes += capture_cost(&input);
                    observed.push((
                        input.resource.path.clone(),
                        input.sha256.clone(),
                        input.canonical_path.clone(),
                    ));
                    captured.push(input);
                    if captured.len() > 4096 || retained_bytes > 32 * 1024 * 1024 {
                        return Err(DomainError::LimitExceeded(
                            "extended resource capture".into(),
                        ));
                    }
                }
            }
            for (index, note) in &notes[notes_start..] {
                extension_notes.push((kind, index - group_start, note.clone()));
            }
        }
        let catalog = catalog::compile_with_notes(&captured, &notes)?;
        for (path, hash, canonical) in observed {
            let (_, revision, current_path) =
                self.fs.read(Path::new(&path), 1024 * 1024, control)?;
            if hash != revision.sha256 || Path::new(&canonical) != current_path {
                return Err(DomainError::Conflict);
            }
        }
        control.check()?;
        let fingerprint = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&current.fingerprint, &catalog.fingerprint, &contributions))
                    .map_err(|_| DomainError::InvalidInput("extended resource identity".into()))?
            )
        );
        let candidate = Snapshot {
            override_evidence: None,
            catalog,
            paths: current.paths.clone(),
            agents_files: current.agents_files.clone(),
            system: current.system.clone(),
            append_system: current.append_system.clone(),
            diagnostics: current.diagnostics.clone(),
            fingerprint,
            captured,
            loaded_paths: loaded,
            contributions,
            extension_notes,
        };
        let candidate = self.apply_overrides(candidate, &changed, false, control)?;
        if self.overrides.is_some() {
            for input in candidate
                .captured
                .iter()
                .filter(|r| changed.contains(&r.kind))
            {
                let (_, revision, canonical) =
                    self.fs
                        .read(Path::new(&input.resource.path), 1024 * 1024, control)?;
                if revision.sha256 != input.sha256
                    || canonical.to_str() != Some(input.canonical_path.as_str())
                {
                    return Err(DomainError::Conflict);
                }
            }
        }
        let next = self
            .generation
            .checked_add(1)
            .ok_or_else(|| DomainError::LimitExceeded("resource generation".into()))?;
        self.current = Some(candidate);
        self.generation = next;
        Ok(next)
    }
}
impl Build<'_> {
    fn note(&mut self, value: Diagnostic) -> Result<(), DomainError> {
        validate_diagnostics(self.diagnostics.iter().chain(std::iter::once(&value)))?;
        self.diagnostics.push(value);
        Ok(())
    }
    fn read(&mut self, path: &Path) -> Result<String, DomainError> {
        let (bytes, revision, canonical) = self.loader.fs.read(path, 1024 * 1024, self.control)?;
        self.observed
            .push((path.into(), revision.sha256, canonical));
        if self.observed.len() > 8192 {
            return Err(DomainError::LimitExceeded("resource observations".into()));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    fn json(&mut self, path: &Path) -> Result<Option<Value>, DomainError> {
        if !self.loader.fs.exists(path)? {
            return Ok(None);
        }
        let text = self.read(path)?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| DomainError::InvalidInput("resource JSON document".into()))
    }
    fn scan(&mut self, path: &Path, mode: Discovery) -> Result<Vec<String>, DomainError> {
        let scan = self.loader.fs.scan(
            path,
            mode,
            ScanControl {
                cancel: self.control.cancel,
                deadline: self.control.deadline,
            },
        )?;
        validate_diagnostics(self.diagnostics.iter().chain(scan.diagnostics.iter()))?;
        self.diagnostics.extend(scan.diagnostics);
        Ok(scan.paths)
    }
    fn collect(
        &mut self,
        base: &Path,
        paths: &[String],
        kind: Kind,
    ) -> Result<Vec<String>, DomainError> {
        let mut result = vec![];
        let mut seen = BTreeSet::new();
        for path in paths {
            let path = resolve_path(base, path, &self.loader.config.home)?;
            let mode = if kind == Kind::Extension {
                Discovery::Extensions
            } else {
                Discovery::Recursive(kind)
            };
            for path in self.scan(&path, mode)? {
                if seen.insert(path.clone()) {
                    result.push(path);
                    if result.len() > 4096 {
                        return Err(DomainError::LimitExceeded("resource paths".into()));
                    }
                }
            }
        }
        Ok(result)
    }
    fn manifest_files(
        &mut self,
        root: &Path,
        paths: &[String],
        kind: Kind,
    ) -> Result<Vec<String>, DomainError> {
        let plain = paths
            .iter()
            .filter(|p| !is_pattern(p))
            .cloned()
            .collect::<Vec<_>>();
        let all = self.collect(root, &plain, kind)?;
        Filters::new(
            &paths
                .iter()
                .filter(|p| is_pattern(p))
                .cloned()
                .collect::<Vec<_>>(),
            false,
        )?
        .select(&all, &text(root)?)
    }
    fn package(
        &mut self,
        root: &Path,
        filter: Option<&Value>,
        metadata: &Metadata,
        result: &mut Paths,
    ) -> Result<(), DomainError> {
        if !self.loader.fs.exists(root)? {
            return Ok(());
        }
        if !self.loader.fs.is_directory(root)? {
            return result.add(Kind::Extension, text(root)?, metadata, true);
        }
        let manifest = self
            .json(&root.join("package.json"))?
            .and_then(|v| v.get("ag1").filter(|v| !v.is_null()).cloned());
        let mut has_resource = false;
        for kind in KINDS {
            let manifest_entries = manifest.as_ref().and_then(|m| m.get(kind.plural()));
            let patterns = entries(manifest_entries)?;
            let user = filter.and_then(|f| f.get(kind.plural()));
            let all = if user.is_some() {
                if !patterns.is_empty() {
                    self.manifest_files(root, &patterns, kind)?
                } else {
                    self.collect(root, &[kind.plural().into()], kind)?
                }
            } else if manifest_entries.is_some() {
                self.manifest_files(root, &patterns, kind)?
            } else if filter.is_some() || manifest.is_none() {
                self.collect(root, &[kind.plural().into()], kind)?
            } else {
                vec![]
            };
            if self.loader.fs.exists(&root.join(kind.plural()))? {
                has_resource = true;
            }
            let enabled = if let Some(user) = user {
                let patterns = entries(Some(user))?;
                if patterns.is_empty() {
                    vec![]
                } else {
                    Filters::new(&patterns, false)?.select(&all, &text(root)?)?
                }
            } else {
                all.clone()
            };
            for path in all {
                let active = enabled.contains(&path);
                result.add(kind, path, metadata, active)?;
            }
        }
        if manifest.is_none() && filter.is_none() && !has_resource {
            result.add(Kind::Extension, text(root)?, metadata, true)?;
        }
        Ok(())
    }
    fn package_scopes(&mut self, scopes: &[(&str, &Path, &Value)]) -> Result<Paths, DomainError> {
        let config = &self.loader.config;
        let mut result = Paths::default();
        let mut identities = BTreeSet::new();
        for &(scope, base, settings) in scopes {
            if let Some(packages) = settings.get("packages") {
                let packages = packages
                    .as_array()
                    .ok_or_else(|| DomainError::InvalidInput("resource packages".into()))?;
                if packages.len() > 256 {
                    return Err(DomainError::LimitExceeded("resource packages".into()));
                }
                for package in packages {
                    self.control.check()?;
                    let source = package
                        .as_str()
                        .or_else(|| package.get("source").and_then(Value::as_str))
                        .ok_or_else(|| DomainError::InvalidInput("package source".into()))?;
                    let parsed = ag1_core::resources::packages::parse(source)?;
                    let local = match &parsed {
                        ag1_core::resources::packages::Source::Local { path } => {
                            Some(resolve_path(base, path, &config.home)?)
                        }
                        _ => None,
                    };
                    let identity = parsed.identity(
                        &local
                            .as_ref()
                            .map(|p| text(p))
                            .transpose()?
                            .unwrap_or_default(),
                    );
                    if !identities.insert(identity.clone()) {
                        continue;
                    }
                    let root = if let Some(root) = local {
                        root
                    } else {
                        let installed = config
                            .installed
                            .iter()
                            .find(|p| p.source == source && p.scope == scope);
                        if let Some(installed) = installed {
                            if installed.identity != identity {
                                return Err(DomainError::Conflict);
                            }
                            installed.root.clone()
                        } else {
                            self.note(Diagnostic {
                                kind: "warning".into(),
                                message: "package is not installed in a registered root".into(),
                                path: source.into(),
                                collision: None,
                            })?;
                            continue;
                        }
                    };
                    if matches!(
                        parsed,
                        ag1_core::resources::packages::Source::Npm { pinned: true, .. }
                    ) {
                        let manifest = self.json(&root.join("package.json"))?;
                        if !ag1_core::resources::packages::pinned_npm_matches(
                            &parsed,
                            manifest.as_ref(),
                        )? {
                            self.note(Diagnostic {
                                kind: "warning".into(),
                                message: "installed package does not match the configured version; installation is required".into(),
                                path: source.into(), collision: None,
                            })?;
                            continue;
                        }
                    }
                    let base_dir =
                        if self.loader.fs.exists(&root)? && !self.loader.fs.is_directory(&root)? {
                            root.parent().unwrap_or(&root)
                        } else {
                            &root
                        };
                    let metadata = Metadata {
                        source: source.into(),
                        scope: scope.into(),
                        origin: "package".into(),
                        base_dir: Some(text(base_dir)?),
                    };
                    self.package(
                        &root,
                        package.as_object().map(|_| package),
                        &metadata,
                        &mut result,
                    )?;
                }
            }
        }
        Ok(result)
    }
    fn resolve(&mut self) -> Result<Paths, DomainError> {
        let config = &self.loader.config;
        let project = config.cwd.join(".ag1");
        let user = self
            .json(&config.agent_dir.join("settings.json"))?
            .unwrap_or_else(|| serde_json::json!({}));
        let project_settings = self
            .json(&project.join("settings.json"))?
            .unwrap_or_else(|| serde_json::json!({}));
        if !user.is_object() || !project_settings.is_object() {
            return Err(DomainError::InvalidInput("resource settings object".into()));
        }
        let scopes: [(&str, &Path, &Value); 2] = [
            ("project", &project, &project_settings),
            ("user", &config.agent_dir, &user),
        ];
        let mut result = self.package_scopes(&scopes)?;
        for kind in KINDS {
            for (scope, base, settings) in scopes {
                let configured = entries(settings.get(kind.plural()))?;
                let plain = configured
                    .iter()
                    .filter(|p| !is_pattern(p))
                    .cloned()
                    .collect::<Vec<_>>();
                let all = self.collect(base, &plain, kind)?;
                let patterns = configured
                    .iter()
                    .filter(|p| is_pattern(p))
                    .cloned()
                    .collect::<Vec<_>>();
                let enabled = Filters::new(&patterns, false)?.select(&all, &text(base)?)?;
                let metadata = Metadata {
                    source: "local".into(),
                    scope: scope.into(),
                    origin: "top-level".into(),
                    base_dir: None,
                };
                for path in all {
                    let active = enabled.contains(&path);
                    result.add(kind, path, &metadata, active)?;
                }
            }
        }
        for (scope, base, settings) in scopes {
            for kind in KINDS {
                let mode = match kind {
                    Kind::Extension => Discovery::Extensions,
                    Kind::Skill => Discovery::Skills {
                        root_markdown: true,
                    },
                    Kind::Prompt => Discovery::Prompts,
                    Kind::Theme => Discovery::Themes,
                };
                let mut all = self.scan(&base.join(kind.plural()), mode)?;
                if kind == Kind::Skill {
                    if scope == "project" {
                        let mut dir = config.cwd.as_path();
                        loop {
                            let root = dir.join(".agents/skills");
                            if root != config.home.join(".agents/skills")
                                && self.loader.fs.admits_path(&root)
                            {
                                all.extend(self.scan(
                                    &root,
                                    Discovery::Skills {
                                        root_markdown: false,
                                    },
                                )?);
                            }
                            if self.loader.fs.admits_path(&dir.join(".git"))
                                && self.loader.fs.exists(&dir.join(".git"))?
                            {
                                break;
                            }
                            let Some(parent) = dir.parent() else { break };
                            dir = parent;
                        }
                    } else {
                        let root = config.home.join(".agents/skills");
                        if self.loader.fs.admits_path(&root) {
                            all.extend(self.scan(
                                &root,
                                Discovery::Skills {
                                    root_markdown: false,
                                },
                            )?);
                        }
                    }
                }
                let enabled = Filters::new(&entries(settings.get(kind.plural()))?, true)?
                    .select(&all, &text(base)?)?;
                let metadata = Metadata {
                    source: "auto".into(),
                    scope: scope.into(),
                    origin: "top-level".into(),
                    base_dir: Some(text(base)?),
                };
                for path in all {
                    let active = enabled.contains(&path);
                    result.add(kind, path, &metadata, active)?;
                }
            }
        }
        for kind in KINDS {
            catalog::order(result.get_mut(kind));
        }
        Ok(result)
    }
    fn context(&mut self, dir: &Path) -> Result<Option<ContextFile>, DomainError> {
        for name in ["AGENTS.md", "CLAUDE.md"] {
            let path = dir.join(name);
            if self.loader.fs.exists(&path)? {
                return Ok(Some(ContextFile {
                    path: text(&path)?,
                    content: self.read(&path)?,
                }));
            }
        }
        Ok(None)
    }
    fn prompt(&mut self, source: Option<&str>, name: &str) -> Result<Option<String>, DomainError> {
        if let Some(source) = source {
            if source.is_empty() {
                return Ok(None);
            }
            if source.len() <= 8192 {
                let path = resolve_path(&self.loader.config.cwd, source, &self.loader.config.home)?;
                // A prompt can be literal text even when it fits our path byte
                // limit. Linux rejects a single component longer than NAME_MAX;
                // that cannot designate a prompt file. Keep this fallback local
                // to ambiguous prompt inputs, without hiding resource I/O errors.
                #[cfg(target_os = "linux")]
                if self.loader.fs.admits_path(&path)
                    && std::fs::symlink_metadata(&path)
                        .is_err_and(|error| error.raw_os_error() == Some(libc::ENAMETOOLONG))
                {
                    return Ok(Some(source.into()));
                }
                if self.loader.fs.admits_path(&path) && self.loader.fs.exists(&path)? {
                    return self.read(&path).map(Some);
                }
            }
            return Ok(Some(source.into()));
        }
        for base in [
            self.loader.config.cwd.join(".ag1"),
            self.loader.config.agent_dir.clone(),
        ] {
            let path = base.join(name);
            if self.loader.fs.exists(&path)? {
                return self.read(&path).map(Some);
            }
        }
        Ok(None)
    }
}

pub struct StandaloneSkills<'a> {
    pub cwd: &'a Path,
    pub agent_dir: &'a Path,
    pub home: &'a Path,
    pub paths: &'a [String],
    pub include_defaults: bool,
}
/// Standalone SDK loader preserves supplied order (defaults: user then project).
/// It intentionally differs from the full loader's ranked project precedence.
pub fn load_skills(
    fs: &ResourceFs,
    options: StandaloneSkills<'_>,
    control: &ScanControl<'_>,
) -> Result<Catalog, DomainError> {
    if options.paths.len() > 256 {
        return Err(DomainError::LimitExceeded("explicit skill paths".into()));
    }
    let user = options.agent_dir.join("skills");
    let project = options.cwd.join(".ag1/skills");
    let mut paths = vec![];
    if options.include_defaults {
        paths.push((user.clone(), "user", false));
        paths.push((project.clone(), "project", false));
    }
    for path in options.paths {
        let path = resolve_path(options.cwd, path, options.home)?;
        let scope = if !options.include_defaults && path.starts_with(&user) {
            "user"
        } else if !options.include_defaults && path.starts_with(&project) {
            "project"
        } else {
            "temporary"
        };
        paths.push((path, scope, true));
    }
    let mut captured = vec![];
    let mut notes = vec![];
    let mut bytes = 0;
    for (path, scope, explicit) in paths {
        control.check()?;
        if !fs.exists(&path)? {
            if explicit {
                push_note(
                    &mut notes,
                    (
                        captured.len(),
                        Diagnostic {
                            kind: "warning".into(),
                            message: "skill path does not exist".into(),
                            path: text(&path)?,
                            collision: None,
                        },
                    ),
                )?;
            }
            continue;
        }
        if !fs.is_directory(&path)? && !path.as_os_str().as_encoded_bytes().ends_with(b".md") {
            push_note(
                &mut notes,
                (
                    captured.len(),
                    Diagnostic {
                        kind: "warning".into(),
                        message: "skill path is not a markdown file".into(),
                        path: text(&path)?,
                        collision: None,
                    },
                ),
            )?;
            continue;
        }
        let scan = fs.scan(
            &path,
            Discovery::Skills {
                root_markdown: true,
            },
            ScanControl {
                cancel: control.cancel,
                deadline: control.deadline,
            },
        )?;
        for diagnostic in scan.diagnostics {
            push_note(&mut notes, (captured.len(), diagnostic))?;
        }
        for path in scan.paths {
            let metadata = Metadata {
                source: "local".into(),
                scope: scope.into(),
                origin: "top-level".into(),
                base_dir: Path::new(&path).parent().map(text).transpose()?,
            };
            let capture = fs.capture(
                ResolvedResource {
                    path,
                    enabled: true,
                    metadata,
                },
                Kind::Skill,
                Trust::UntrustedData,
                control,
            )?;
            bytes += capture.bytes.len();
            if captured.len() >= 4096 || bytes > 32 * 1024 * 1024 {
                return Err(DomainError::LimitExceeded("standalone skills".into()));
            }
            captured.push(capture);
        }
    }
    catalog::compile_with_notes(&captured, &notes)
}
