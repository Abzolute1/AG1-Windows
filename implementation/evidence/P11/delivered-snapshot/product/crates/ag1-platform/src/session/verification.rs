//! Fresh, read-only verifier discovery. No repository cache, command execution,
//! symlink traversal or full-file reads outside the existing root capability.
use crate::workspace::Workspace;
use ag1_core::{
    DomainError,
    runtime::verification::{Index, MARKERS, SOURCE_EXTENSIONS},
    tools::list::compare,
};
use regex::Regex;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

struct Ignore {
    directory_only: bool,
    pattern: String,
    glob: Option<Regex>,
}
fn ignore_rules(text: &str) -> Result<Vec<Ignore>, DomainError> {
    let mut rules = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(['#', '!']))
    {
        if rules.len() == 4096 {
            return Err(DomainError::LimitExceeded("verifier ignore rules".into()));
        }
        let directory_only = line.ends_with('/');
        let pattern = line
            .strip_suffix('/')
            .unwrap_or(line)
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_owned();
        let glob = if pattern.contains('*') {
            Some(
                Regex::new(&format!(
                    "(^|/){}($|/)",
                    pattern
                        .split('*')
                        .map(regex::escape)
                        .collect::<Vec<_>>()
                        .join("[^/]*")
                ))
                .map_err(|_| DomainError::InvalidInput("verifier ignore pattern".into()))?,
            )
        } else {
            None
        };
        rules.push(Ignore {
            directory_only,
            pattern,
            glob,
        });
    }
    Ok(rules)
}
fn ignored(path: &str, directory: bool, rules: &[Ignore]) -> bool {
    const NOISE: &[&str] = &[
        ".git",
        ".hg",
        ".svn",
        "node_modules",
        "dist",
        "build",
        "coverage",
        ".next",
        ".turbo",
        ".cache",
        "Debug",
    ];
    if path.split('/').any(|part| NOISE.contains(&part)) {
        return true;
    }
    rules.iter().any(|rule| {
        if rule.directory_only && !directory && !path.starts_with(&format!("{}/", rule.pattern)) {
            return false;
        }
        if let Some(glob) = &rule.glob {
            return glob.is_match(path);
        }
        if rule.pattern.contains('/') {
            path == rule.pattern || path.starts_with(&format!("{}/", rule.pattern))
        } else {
            path.split('/').any(|part| part == rule.pattern)
        }
    })
}
fn check(cancelled: &AtomicBool) -> Result<(), DomainError> {
    if cancelled.load(Ordering::Acquire) {
        Err(DomainError::Cancelled)
    } else {
        Ok(())
    }
}
struct Scan<'a> {
    workspace: &'a Workspace,
    cancelled: &'a AtomicBool,
    rules: Vec<Ignore>,
    files: Vec<String>,
    entries: usize,
}
impl Scan<'_> {
    fn directory(&mut self, path: &str, depth: usize) -> Result<(), DomainError> {
        check(self.cancelled)?;
        if depth > 32 {
            return Err(DomainError::LimitExceeded(
                "verifier directory depth".into(),
            ));
        }
        let mut entries = self.workspace.directory_entries(path)?;
        self.entries += entries.len();
        if self.entries > 20000 {
            return Err(DomainError::LimitExceeded(
                "verifier directory entries".into(),
            ));
        }
        // libuv's directory enumeration is lexical; root files get metadata
        // priority before walking any subdirectory, even at the 1000-file cap.
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        if depth == 0 {
            entries.sort_by(|a, b| {
                a.directory.cmp(&b.directory).then_with(|| {
                    if !a.directory && !b.directory {
                        (!MARKERS.contains(&a.name.as_str()))
                            .cmp(&(!MARKERS.contains(&b.name.as_str())))
                            .then_with(|| compare(&a.name, &b.name))
                    } else {
                        a.name.cmp(&b.name)
                    }
                })
            });
        }
        for entry in entries {
            check(self.cancelled)?;
            if self.files.len() == 1000 {
                break;
            }
            let child = if path.is_empty() {
                entry.name
            } else {
                format!("{path}/{}", entry.name)
            };
            if child.len() > 4096 {
                return Err(DomainError::LimitExceeded("verifier path".into()));
            }
            if entry.symlink || ignored(&child, entry.directory, &self.rules) {
                continue;
            }
            if entry.directory {
                self.directory(&child, depth + 1)?;
            } else if self.workspace.regular_file(&child)? {
                self.files.push(child);
            }
        }
        Ok(())
    }
}
pub fn capture(workspace: &Workspace, cancelled: &AtomicBool) -> Result<Index, DomainError> {
    check(cancelled)?;
    let ignore = match workspace.snapshot(".gitignore", 64 * 1024) {
        Ok(snapshot) => String::from_utf8_lossy(&snapshot.bytes).into_owned(),
        Err(DomainError::NotFound) => String::new(),
        Err(error) => return Err(error),
    };
    let mut scan = Scan {
        workspace,
        cancelled,
        rules: ignore_rules(&ignore)?,
        files: Vec::new(),
        entries: 0,
    };
    scan.directory("", 0)?;
    let mut scripts = BTreeMap::new();
    let package = match workspace.snapshot("package.json", 1024 * 1024) {
        Ok(snapshot) => Some(snapshot),
        Err(DomainError::NotFound) => None,
        Err(error) => return Err(error),
    };
    if let Some(snapshot) = package
        && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&snapshot.bytes)
        && let Some(object) = value["scripts"].as_object()
    {
        scripts.extend(object.iter().filter_map(|(name, value)| {
            value
                .as_str()
                .map(|script| (name.clone(), script.to_owned()))
        }));
    }
    let mut sources = BTreeMap::new();
    let mut bytes = 0;
    for path in &scan.files {
        check(cancelled)?;
        if !path
            .rsplit_once('.')
            .is_some_and(|(_, ext)| SOURCE_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        {
            continue;
        }
        let snapshot = match workspace.snapshot(path, 2 * 1024 * 1024) {
            Ok(snapshot) => snapshot,
            Err(DomainError::NotFound) => continue,
            Err(error) => return Err(error),
        };
        bytes += snapshot.bytes.len();
        if bytes > 16 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("verifier source bytes".into()));
        }
        sources.insert(
            path.clone(),
            String::from_utf8_lossy(&snapshot.bytes).into_owned(),
        );
    }
    workspace.verify_root()?;
    check(cancelled)?;
    Index::captured(scan.files, scripts, &sources)
}
