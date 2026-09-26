//! Read-only repository orientation through owned Git and workspace capabilities.
use crate::{
    file_tools::{FileExecution, FileOutcome},
    git::GitService,
    source::SourceService,
    workspace::Workspace,
};
use ag1_core::{
    DomainError,
    evidence::{Freshness, memory_sources},
    memory::MemoryService,
    tools::{
        approval::{AuthorizedCall, ExecutionFence},
        memory,
        registry::Origin,
        repo::{self, GitState, RepoMap, StackHint},
    },
};
use ag1_storage::Store;
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub struct RepoTools {
    workspace: Arc<Workspace>,
    git: GitService,
    store: Arc<Store>,
}
impl RepoTools {
    pub fn new(workspace: Arc<Workspace>, git: GitService, store: Arc<Store>) -> Self {
        Self {
            workspace,
            git,
            store,
        }
    }
    pub fn target_revision(&self) -> String {
        format!("repo:{}", self.workspace.id().as_str())
    }
    pub fn execute<F: Fn() -> u64>(
        &self,
        authorization: AuthorizedCall,
        execution: FileExecution<'_, F>,
    ) -> Result<FileOutcome, DomainError> {
        let FileExecution {
            registry,
            scope,
            policy_revision,
            now,
            cancel,
            deadline,
        } = execution;
        if scope.workspace != self.workspace.id().as_str()
            || authorization.call().name() != "repo"
            || authorization.call().origin() != Origin::BuiltIn
        {
            return Err(DomainError::PermissionDenied);
        }
        let call = authorization.consume(
            registry,
            ExecutionFence {
                scope,
                policy_revision,
                target_revision: &self.target_revision(),
                now: now(),
            },
        )?;
        let check = || {
            if cancel.load(Ordering::Acquire) {
                Err(DomainError::Cancelled)
            } else if Instant::now() >= deadline {
                Err(DomainError::TimedOut)
            } else {
                Ok(())
            }
        };
        check()?;
        let view = call.args()["view"].as_str().unwrap_or("all");
        let git = self.git_state(cancel, deadline)?;
        let map = self.map(&check)?;
        let (memory, _) = MemoryService::load(self.store.as_ref(), self.workspace.id().as_str())?;
        let mut sections = vec![];
        if view != "map" {
            sections.push(repo::format_git(&git));
        }
        if view != "status" {
            sections.push(repo::format_map(&map));
            let count = memory.value()["entries"].as_array().unwrap().len();
            if count > 0 {
                let source = std::cell::RefCell::new(SourceService::new(self.workspace.clone()));
                let text = memory::format_entries(&memory, |entry| {
                    memory_sources(entry, |reference| {
                        if reference.starts_with("file:") {
                            source.borrow_mut().validate(reference).status
                        } else {
                            Freshness::Unverified
                        }
                    })
                })?;
                sections.push(format!("Memory ({count}):\n{text}"));
            }
        }
        self.workspace.verify_root()?;
        check()?;
        let mut details =
            json!({"view":view,"dirtyCount":git.dirty.len(),"conflictCount":git.conflicts.len()});
        if let Some(branch) = git.branch {
            details["branch"] = json!(branch);
        }
        Ok(FileOutcome {
            result: json!({"content":[{"type":"text","text":sections.join("\n\n")}],"details":details}),
            published: None,
        })
    }
    fn git_state(&self, cancel: &AtomicBool, deadline: Instant) -> Result<GitState, DomainError> {
        let run = |args: &[&str]| {
            self.git.tool_command(
                &self.workspace,
                &args.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
                cancel,
                deadline,
                256 * 1024,
            )
        };
        let (inside, success) = run(&["rev-parse", "--is-inside-work-tree"])?;
        if !success {
            if inside.contains("not a git repository") {
                return Ok(GitState::default());
            }
            return Err(DomainError::Io(format!(
                "Cannot inspect repository: {inside}"
            )));
        }
        if inside.trim() != "true" {
            return Ok(GitState::default());
        }
        let required = |args: &[&str]| {
            let (text, success) = run(args)?;
            if success {
                Ok(text)
            } else {
                Err(DomainError::Io(format!(
                    "Repository inspection failed: {text}"
                )))
            }
        };
        let status = required(&["status", "--porcelain=v1", "-b", "-z"])?;
        let mut fields = status.split('\0').filter(|s| !s.is_empty());
        let header = fields
            .next()
            .and_then(|line| line.strip_prefix("## "))
            .ok_or_else(|| DomainError::InvalidInput("Git status header".into()))?;
        let unborn = header
            .strip_prefix("No commits yet on ")
            .or_else(|| header.strip_prefix("Initial commit on "));
        let branch = if header.starts_with("HEAD (no branch)") {
            None
        } else if let Some(branch) = unborn {
            Some(branch.to_owned())
        } else {
            Some(header.split("...").next().unwrap().to_owned())
        };
        let upstream = header.split_once("...").map(|(_, rest)| {
            rest.split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned()
        });
        let count = |word: &str| {
            header
                .split_once(word)
                .and_then(|(_, rest)| rest.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        };
        let mut git = GitState {
            inside_repo: true,
            branch,
            upstream,
            ahead: count("ahead "),
            behind: count("behind "),
            ..Default::default()
        };
        while let Some(field) = fields.next() {
            let bytes = field.as_bytes();
            if bytes.len() < 4 || bytes[2] != b' ' {
                return Err(DomainError::InvalidInput("Git status record".into()));
            }
            let path = &field[3..];
            let renamed = bytes[..2].iter().any(|b| matches!(b, b'R' | b'C'));
            let dirty = if renamed {
                let old = fields
                    .next()
                    .ok_or_else(|| DomainError::InvalidInput("Git rename source".into()))?;
                format!("{old} -> {path}")
            } else {
                path.into()
            };
            if [b"DD", b"AU", b"UD", b"UA", b"DU", b"AA", b"UU"]
                .iter()
                .any(|code| bytes.starts_with(*code))
            {
                git.conflicts.push(path.into());
            }
            git.dirty.push(dirty);
        }
        if unborn.is_none() {
            let head_args = [
                "log",
                "-1",
                "--oneline",
                "--no-decorate",
                "--no-show-signature",
                "--no-color",
            ];
            let head = required(&head_args)?;
            let stat = required(&[
                "diff",
                "--stat",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "HEAD",
                "--",
            ])?;
            if !stat.trim().is_empty() {
                git.diff_stat = Some(stat.trim().into());
            }
            if required(&head_args)? != head {
                return Err(DomainError::Conflict);
            }
            git.head = Some(head.trim().into());
        }
        if required(&["status", "--porcelain=v1", "-b", "-z"])? != status {
            return Err(DomainError::Conflict);
        }
        Ok(git)
    }
    fn map(&self, check: &impl Fn() -> Result<(), DomainError>) -> Result<RepoMap, DomainError> {
        let mut map = RepoMap::default();
        let mut layout = self.workspace.directory_entries("")?;
        let manifests = [
            ("package.json", "node"),
            ("pnpm-workspace.yaml", "node-workspace"),
            ("Cargo.toml", "rust"),
            ("pyproject.toml", "python"),
            ("go.mod", "go"),
            ("CMakeLists.txt", "cmake"),
            ("composer.json", "php"),
            ("Gemfile", "ruby"),
            ("mix.exs", "elixir"),
        ];
        let mut package = None;
        for (manifest, kind) in manifests {
            check()?;
            if !layout.iter().any(|entry| entry.name == manifest) {
                continue;
            }
            let mut hint = StackHint {
                kind: kind.into(),
                manifest: manifest.into(),
                ..Default::default()
            };
            if let Ok(snapshot) = self.workspace.snapshot(manifest, 1024 * 1024)
                && let Ok(text) = std::str::from_utf8(&snapshot.bytes)
            {
                if manifest == "package.json" {
                    if let Ok(value) = serde_json::from_str::<Value>(text) {
                        hint.name = value["name"].as_str().map(str::to_owned);
                        hint.detail = value["version"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(|version| {
                                format!("{}@{version}", hint.name.as_deref().unwrap_or("package"))
                            })
                            .or_else(|| hint.name.clone());
                        package = serde_json::from_str::<PackageEntries>(text).ok();
                    }
                } else {
                    // Match the reference's ordered double-quote/module/single-quote hints.
                    for pattern in [
                        r#"(?m)^name\s*=\s*"([^"]+)""#,
                        r"(?m)^module\s+(\S+)",
                        r"(?m)^name\s*=\s*'([^']+)'",
                    ] {
                        if let Some(capture) = regex::Regex::new(pattern).unwrap().captures(text) {
                            hint.name = Some(capture[1].into());
                            hint.detail = hint.name.clone();
                            break;
                        }
                    }
                }
            }
            map.stack.push(hint);
        }
        for candidate in [
            "src/main.ts",
            "src/index.ts",
            "src/main.py",
            "src/lib.rs",
            "main.go",
            "app.py",
        ] {
            check()?;
            if self.workspace.exists(candidate).unwrap_or(false) {
                map.entries.push(candidate.into());
            }
        }
        if let Some(package) = package {
            if let Some(main) = package.main {
                map.entries.push(main);
            }
            map.entries.extend(package.bin.0);
        }
        let mut seen = std::collections::BTreeSet::new();
        map.entries.retain(|entry| seen.insert(entry.clone()));
        for name in ["README.md", "readme.md", "README"] {
            check()?;
            if !layout.iter().any(|entry| entry.name == name) {
                continue;
            }
            if let Ok(snapshot) = self.workspace.snapshot(name, 1024 * 1024)
                && let Ok(text) = std::str::from_utf8(&snapshot.bytes)
            {
                let preview = text
                    .split('\n')
                    .take(16)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_owned();
                if !preview.is_empty() {
                    map.readme_preview = Some(preview);
                }
            }
            break;
        }
        layout.retain(|entry| {
            ![".git", "node_modules", "dist", ".ag1"].contains(&entry.name.as_str())
        });
        layout.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| ag1_core::tools::list::compare(&a.name, &b.name))
        });
        map.layout = layout
            .iter()
            .take(24)
            .map(|entry| format!("{}{}", entry.name, if entry.directory { "/" } else { "" }))
            .collect();
        if layout.len() > 24 {
            map.layout.push(format!("+{} more", layout.len() - 24));
        }
        Ok(map)
    }
}

// serde_json::Value sorts object keys. Package bin order is user-visible, so
// retain the input order and apply JavaScript's integer-key ordering locally.
#[derive(Default, serde::Deserialize)]
struct PackageEntries {
    #[serde(default)]
    main: Option<String>,
    #[serde(default)]
    bin: BinEntries,
}
#[derive(Default)]
struct BinEntries(Vec<String>);
impl<'de> serde::Deserialize<'de> for BinEntries {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = BinEntries;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("package bin string or object")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<BinEntries, E> {
                Ok(BinEntries(vec![value.into()]))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<BinEntries, E> {
                Ok(BinEntries::default())
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<BinEntries, M::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    if let Some(value) = value.as_str() {
                        if let Some((_, existing)) =
                            entries.iter_mut().find(|(name, _)| name == &key)
                        {
                            *existing = value.to_owned();
                        } else {
                            entries.push((key, value.to_owned()));
                        }
                    }
                }
                let integer = |key: &str| {
                    key.parse::<u32>()
                        .ok()
                        .filter(|n| *n != u32::MAX && n.to_string() == key)
                };
                entries.sort_by(|(a, _), (b, _)| match (integer(a), integer(b)) {
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                });
                Ok(BinEntries(
                    entries.into_iter().map(|(_, value)| value).collect(),
                ))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}
