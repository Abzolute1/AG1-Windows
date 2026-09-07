//! Read-only resource discovery over host-registered workspace capabilities.
//! Symlink aliases may resolve within those roots; their bytes are read through
//! the same no-follow descriptor primitives as native source tools.
use crate::{io_error, workspace::Workspace};
use ag1_core::{
    DomainError,
    resources::{
        catalog::{CapturedResource, Kind, ResolvedResource, Trust},
        skills::Diagnostic,
    },
};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
pub struct ResourceFs {
    roots: Vec<Arc<Workspace>>,
}
#[derive(Clone, Copy)]
pub enum Discovery {
    Skills {
        root_markdown: bool,
    },
    Prompts,
    /// Standalone loader includes hidden Markdown and does not apply ignore files.
    DirectPrompts,
    Themes,
    Recursive(Kind),
    Extensions,
}
pub struct Scan {
    pub paths: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}
pub struct ScanControl<'a> {
    pub cancel: &'a AtomicBool,
    pub deadline: Instant,
}
struct Walk<'a> {
    control: ScanControl<'a>,
    result: Scan,
    visited: BTreeSet<PathBuf>,
    entries: usize,
    ignore_bytes: usize,
    rules: Vec<String>,
}
pub(crate) fn validate_diagnostics<'a>(
    values: impl Iterator<Item = &'a Diagnostic>,
) -> Result<(), DomainError> {
    let mut bytes = 0usize;
    for (index, value) in values.enumerate() {
        bytes += std::mem::size_of::<Diagnostic>()
            + value.kind.len()
            + value.path.len()
            + value.message.len()
            + value.collision.as_ref().map_or(0, |v| v.to_string().len());
        if index >= 4096 || bytes > 4 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("resource diagnostics".into()));
        }
    }
    Ok(())
}
impl Walk<'_> {
    fn note(&mut self, value: Diagnostic) -> Result<(), DomainError> {
        validate_diagnostics(
            self.result
                .diagnostics
                .iter()
                .chain(std::iter::once(&value)),
        )?;
        self.result.diagnostics.push(value);
        Ok(())
    }
}
impl ScanControl<'_> {
    pub fn check(&self) -> Result<(), DomainError> {
        if self.cancel.load(Ordering::Acquire) {
            Err(DomainError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(DomainError::TimedOut)
        } else {
            Ok(())
        }
    }
}
fn text(path: &Path) -> Result<String, DomainError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| DomainError::InvalidInput("resource path encoding".into()))
}
impl ResourceFs {
    pub fn new(mut roots: Vec<Arc<Workspace>>) -> Result<Self, DomainError> {
        if roots.is_empty() || roots.len() > 64 {
            return Err(DomainError::LimitExceeded("resource roots".into()));
        }
        for root in &roots {
            root.verify_root()?;
        }
        roots.sort_by_key(|r| std::cmp::Reverse(r.root().as_os_str().len()));
        Ok(Self { roots })
    }
    /// Pure lexical capability check, useful for optional ancestor discovery.
    pub fn admits_path(&self, path: &Path) -> bool {
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            && self.roots.iter().any(|root| path.starts_with(root.root()))
    }
    fn resolve(&self, path: &Path) -> Result<(Arc<Workspace>, String, PathBuf), DomainError> {
        if !path.is_absolute() || path.as_os_str().len() > 8192 {
            return Err(DomainError::InvalidInput("absolute resource path".into()));
        }
        // Resolve aliases only as metadata. No target contents are read until the
        // canonical destination is admitted to one of the registered descriptors.
        let canonical = std::fs::canonicalize(path).map_err(io_error)?;
        for root in &self.roots {
            if let Ok(relative) = canonical.strip_prefix(root.root()) {
                root.verify_root()?;
                return Ok((root.clone(), text(relative)?, canonical));
            }
        }
        Err(DomainError::PermissionDenied)
    }
    pub fn is_directory(&self, path: &Path) -> Result<bool, DomainError> {
        let (root, relative, _) = self.resolve(path)?;
        root.directory_exists(&relative)
    }
    pub fn exists(&self, path: &Path) -> Result<bool, DomainError> {
        match self.resolve(path) {
            Ok((root, relative, _)) => {
                if relative.is_empty() {
                    Ok(true)
                } else {
                    root.exists(&relative)
                }
            }
            Err(DomainError::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }
    fn directory(
        &self,
        path: &Path,
    ) -> Result<(PathBuf, Vec<ag1_core::tools::list::Entry>), DomainError> {
        let (root, relative, canonical) = self.resolve(path)?;
        let mut entries = root.directory_entries(&relative)?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        if std::fs::canonicalize(path).map_err(io_error)? != canonical {
            return Err(DomainError::Conflict);
        }
        Ok((canonical, entries))
    }
    pub fn read(
        &self,
        path: &Path,
        limit: usize,
        control: &ScanControl<'_>,
    ) -> Result<(Vec<u8>, ag1_core::FileRevision, PathBuf), DomainError> {
        control.check()?;
        if limit == 0 || limit > 1024 * 1024 {
            return Err(DomainError::LimitExceeded("resource document input".into()));
        }
        let (root, relative, canonical) = self.resolve(path)?;
        let mut bytes = vec![];
        let revision = root.stream(&relative, limit as u64, |chunk| {
            control.check()?;
            bytes.extend_from_slice(chunk);
            Ok(())
        })?;
        if std::fs::canonicalize(path).map_err(io_error)? != canonical {
            return Err(DomainError::Conflict);
        }
        control.check()?;
        Ok((bytes, revision, canonical))
    }
    pub fn capture(
        &self,
        resource: ResolvedResource,
        kind: Kind,
        trust: Trust,
        control: &ScanControl<'_>,
    ) -> Result<CapturedResource, DomainError> {
        let (bytes, revision, canonical) =
            self.read(Path::new(&resource.path), 1024 * 1024, control)?;
        Ok(CapturedResource {
            resource,
            kind,
            canonical_path: text(&canonical)?,
            bytes,
            sha256: revision.sha256,
            trust,
        })
    }
    pub fn scan(
        &self,
        path: &Path,
        discovery: Discovery,
        control: ScanControl<'_>,
    ) -> Result<Scan, DomainError> {
        control.check()?;
        let mut walk = Walk {
            control,
            result: Scan {
                paths: vec![],
                diagnostics: vec![],
            },
            visited: BTreeSet::new(),
            entries: 0,
            ignore_bytes: 0,
            rules: vec![],
        };
        if !self.exists(path)? {
            return Ok(walk.result);
        }
        if !self.is_directory(path)? {
            walk.result.paths.push(text(path)?);
            return Ok(walk.result);
        }
        self.walk(path, path, discovery, 0, &mut walk)?;
        if walk.result.paths.len() > 4096 {
            return Err(DomainError::LimitExceeded("discovered resources".into()));
        }
        Ok(walk.result)
    }
    fn ignore_rules(
        &self,
        dir: &Path,
        root: &Path,
        walk: &mut Walk<'_>,
    ) -> Result<Gitignore, DomainError> {
        let prefix = dir
            .strip_prefix(root)
            .map_err(|_| DomainError::PermissionDenied)?;
        let prefix = if prefix.as_os_str().is_empty() {
            String::new()
        } else {
            format!("{}/", text(prefix)?)
        };
        for filename in [".gitignore", ".ignore", ".fdignore"] {
            let path = dir.join(filename);
            if !self.exists(&path)? {
                continue;
            }
            let (bytes, _, _) = self.read(&path, 65536, &walk.control)?;
            walk.ignore_bytes += bytes.len();
            if walk.ignore_bytes > 1024 * 1024 {
                return Err(DomainError::LimitExceeded("resource ignore files".into()));
            }
            let body = String::from_utf8_lossy(&bytes);
            for line in body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }
                if walk.rules.len() >= 4096 || line.len() > 4096 {
                    return Err(DomainError::LimitExceeded(
                        "resource ignore patterns".into(),
                    ));
                }
                let (negated, pattern) = if let Some(tail) = line.strip_prefix('!') {
                    (true, tail)
                } else {
                    // The SDK removes only the leading backslash here; at the
                    // root this is subsequently interpreted as a negation by
                    // its ignore matcher. Preserve that captured helper behavior.
                    (
                        false,
                        if line.starts_with("\\!") {
                            &line[1..]
                        } else {
                            line
                        },
                    )
                };
                let pattern = pattern.strip_prefix('/').unwrap_or(pattern);
                walk.rules.push(format!(
                    "{}{prefix}{pattern}",
                    if negated { "!" } else { "" }
                ));
            }
        }
        let mut builder = GitignoreBuilder::new(root);
        for line in &walk.rules {
            builder
                .add_line(None, line)
                .map_err(|_| DomainError::InvalidInput("resource ignore pattern".into()))?;
        }
        builder
            .build()
            .map_err(|_| DomainError::InvalidInput("resource ignore patterns".into()))
    }
    fn extension_entries(
        &self,
        path: &Path,
        control: &ScanControl<'_>,
    ) -> Result<Option<Vec<String>>, DomainError> {
        let manifest = path.join("package.json");
        if self.exists(&manifest)? {
            let (bytes, _, _) = self.read(&manifest, 1024 * 1024, control)?;
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
                && let Some(entries) = value["ag1"]["extensions"].as_array()
            {
                if entries.len() > 256 {
                    return Err(DomainError::LimitExceeded(
                        "extension manifest entries".into(),
                    ));
                }
                let mut found = vec![];
                for value in entries {
                    let entry = value
                        .as_str()
                        .ok_or_else(|| DomainError::InvalidInput("extension entry path".into()))?;
                    if entry.len() > 8192 || entry.contains('\0') {
                        return Err(DomainError::InvalidInput("extension entry path".into()));
                    }
                    let mut target = PathBuf::new();
                    for component in path.join(entry).components() {
                        match component {
                            std::path::Component::ParentDir => {
                                target.pop();
                            }
                            std::path::Component::CurDir => {}
                            other => target.push(other.as_os_str()),
                        }
                    }
                    if self.exists(&target)? {
                        found.push(text(&target)?);
                    }
                }
                if !found.is_empty() {
                    return Ok(Some(found));
                }
            }
        }
        for name in ["index.ts", "index.js"] {
            let candidate = path.join(name);
            if self.exists(&candidate)? {
                return Ok(Some(vec![text(&candidate)?]));
            }
        }
        Ok(None)
    }
    fn walk(
        &self,
        root: &Path,
        dir: &Path,
        mode: Discovery,
        depth: usize,
        walk: &mut Walk<'_>,
    ) -> Result<(), DomainError> {
        walk.control.check()?;
        if depth > 32 {
            return Err(DomainError::LimitExceeded(
                "resource discovery depth".into(),
            ));
        }
        let (canonical, entries) = self.directory(dir)?;
        if !walk.visited.insert(canonical) {
            walk.note(Diagnostic {
                kind: "warning".into(),
                message: "resource directory cycle or duplicate alias skipped".into(),
                path: text(dir)?,
                collision: None,
            })?;
            return Ok(());
        }
        walk.entries += entries.len();
        if walk.entries > 20000 {
            return Err(DomainError::LimitExceeded(
                "resource directory entries".into(),
            ));
        }
        if matches!(mode, Discovery::Extensions)
            && let Some(entries) = self.extension_entries(dir, &walk.control)?
        {
            walk.result.paths.extend(entries);
            return Ok(());
        }
        let ignore = if matches!(mode, Discovery::DirectPrompts) {
            Gitignore::empty()
        } else {
            self.ignore_rules(dir, root, walk)?
        };
        if matches!(
            mode,
            Discovery::Skills { .. } | Discovery::Recursive(Kind::Skill)
        ) {
            for entry in &entries {
                if entry.name == "SKILL.md" {
                    let candidate = dir.join(&entry.name);
                    let is_directory = match self.is_directory(&candidate) {
                        Ok(value) => value,
                        Err(DomainError::NotFound) => continue,
                        Err(DomainError::PermissionDenied) => {
                            walk.note(Diagnostic {
                                kind: "warning".into(),
                                message: "resource alias is outside registered roots".into(),
                                path: text(&candidate)?,
                                collision: None,
                            })?;
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    if !is_directory
                        && !ignore
                            .matched_path_or_any_parents(&candidate, false)
                            .is_ignore()
                    {
                        walk.result.paths.push(text(&candidate)?);
                        return Ok(());
                    }
                }
            }
        }
        for entry in entries {
            walk.control.check()?;
            if !matches!(mode, Discovery::DirectPrompts)
                && (entry.name.starts_with('.') || entry.name == "node_modules")
            {
                continue;
            }
            let candidate = dir.join(&entry.name);
            let is_dir = if entry.symlink {
                match self.is_directory(&candidate) {
                    Ok(v) => v,
                    Err(DomainError::NotFound) => continue,
                    Err(DomainError::PermissionDenied) => {
                        walk.note(Diagnostic {
                            kind: "warning".into(),
                            message: "resource alias is outside registered roots".into(),
                            path: text(&candidate)?,
                            collision: None,
                        })?;
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            } else {
                entry.directory
            };
            if ignore
                .matched_path_or_any_parents(&candidate, is_dir)
                .is_ignore()
            {
                continue;
            }
            let include = match mode {
                Discovery::Skills { root_markdown } => {
                    !is_dir && root_markdown && depth == 0 && entry.name.ends_with(".md")
                }
                Discovery::Prompts | Discovery::DirectPrompts => {
                    !is_dir && entry.name.ends_with(".md")
                }
                Discovery::Themes => !is_dir && entry.name.ends_with(".json"),
                Discovery::Recursive(kind) => {
                    !is_dir
                        && match kind {
                            Kind::Skill => depth == 0 && entry.name.ends_with(".md"),
                            Kind::Prompt => entry.name.ends_with(".md"),
                            Kind::Theme => entry.name.ends_with(".json"),
                            Kind::Extension => {
                                entry.name.ends_with(".js") || entry.name.ends_with(".ts")
                            }
                        }
                }
                Discovery::Extensions => {
                    !is_dir && (entry.name.ends_with(".ts") || entry.name.ends_with(".js"))
                }
            };
            if include {
                walk.result.paths.push(text(&candidate)?);
                if walk.result.paths.len() > 4096 {
                    return Err(DomainError::LimitExceeded("discovered resources".into()));
                }
            } else if is_dir && matches!(mode, Discovery::Skills { .. } | Discovery::Recursive(_)) {
                self.walk(root, &candidate, mode, depth + 1, walk)?;
            } else if is_dir
                && matches!(mode, Discovery::Extensions)
                && let Some(entries) = self.extension_entries(&candidate, &walk.control)?
            {
                walk.result.paths.extend(entries);
                if walk.result.paths.len() > 4096 {
                    return Err(DomainError::LimitExceeded("discovered resources".into()));
                }
            }
        }
        Ok(())
    }
}
