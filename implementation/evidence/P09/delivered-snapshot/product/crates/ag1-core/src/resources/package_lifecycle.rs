//! Package lifecycle over an explicit host port. The manager owns no filesystem,
//! process, credentials or session. Production ports must admit owned installation
//! roots, persist intent, consume approval and bound all I/O before invoking it.
use super::packages::{self, Command, Source};
use crate::DomainError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    User,
    Project,
    Temporary,
}
/// Captured configuration entry. Filters belong to resource discovery; update
/// identity deliberately depends only on the source and its configuration scope.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfiguredPackage {
    pub source: String,
    pub scope: Scope,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub kind: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Options>,
}
pub trait Port {
    fn checkpoint(&mut self) -> Result<(), DomainError>;
    fn exists(&mut self, path: &str) -> Result<bool, DomainError>;
    fn read_json(&mut self, path: &str) -> Result<Option<Value>, DomainError>;
    fn create_directory(&mut self, path: &str) -> Result<(), DomainError>;
    fn write_absent(&mut self, path: &str, bytes: &[u8]) -> Result<(), DomainError>;
    fn remove_owned_tree(
        &mut self,
        path: &str,
        prune_root: Option<&str>,
    ) -> Result<(), DomainError>;
    fn run(&mut self, command: Run) -> Result<String, DomainError>;
    fn progress(&mut self, event: Value) -> Result<(), DomainError>;
}
pub struct Config {
    pub cwd: String,
    pub agent_dir: String,
    pub home: String,
    pub temporary_dir: String,
    pub npm: Command,
    pub offline: bool,
}
pub struct Manager {
    config: Config,
    global_npm: Option<(Command, String)>,
}
fn invalid(reason: &str) -> DomainError {
    DomainError::InvalidInput(reason.into())
}
fn path(base: &str, tail: &str) -> Result<String, DomainError> {
    if base.len() > 8192
        || tail.len() > 8192
        || !base.starts_with('/')
        || base.contains('\0')
        || tail.contains('\0')
    {
        return Err(invalid("package path"));
    }
    let input = Path::new(base).join(tail);
    let mut output = PathBuf::from("/");
    for c in input.components() {
        match c {
            Component::Normal(v) => output.push(v),
            Component::ParentDir => {
                output.pop();
            }
            Component::RootDir | Component::CurDir => {}
            _ => return Err(invalid("package path prefix")),
        }
    }
    output
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("package path encoding"))
}
fn contained(base: &str, tail: &str) -> Result<String, DomainError> {
    let result = path(base, tail)?;
    if result == base || !Path::new(&result).starts_with(base) {
        return Err(DomainError::PermissionDenied);
    }
    Ok(result)
}
fn error_message(error: &DomainError) -> String {
    match error {
        DomainError::InvalidInput(s) | DomainError::Io(s) => s.clone(),
        _ => format!("{error:?}"),
    }
}
pub fn validate_execution_source(source: &Source) -> Result<(), DomainError> {
    let embedded_url = match source {
        Source::Git { repo, .. } => Some(repo.as_str()),
        Source::Npm { spec, .. } => crate::tools::research::rx(r"[A-Za-z][A-Za-z0-9+.-]*://")
            .find(spec)
            .map(|found| &spec[found.start()..]),
        Source::Local { .. } => None,
    };
    if let Some(input) = embedded_url
        && let Ok(url) = url::Url::parse(input)
        && (url.password().is_some()
            || (matches!(url.scheme(), "http" | "https") && !url.username().is_empty()))
    {
        return Err(invalid("credential-bearing package URL"));
    }
    match source {
        Source::Npm { spec, name, .. } if spec.starts_with('-') || name.starts_with('-') => {
            Err(invalid("unsafe npm option-like package spec"))
        }
        Source::Git {
            reference: Some(reference),
            ..
        } if reference.starts_with('-') || reference.contains(['\0', '\n', '\r']) => {
            Err(invalid("unsafe Git checkout ref"))
        }
        _ => Ok(()),
    }
}
impl Manager {
    fn configured_identity(
        &self,
        source: &str,
        scope: Option<Scope>,
    ) -> Result<String, DomainError> {
        let parsed = packages::parse(source)?;
        let local = if let Source::Local { path: input } = &parsed {
            let base = match scope {
                Some(Scope::User) => self.config.agent_dir.clone(),
                Some(Scope::Project) => path(&self.config.cwd, ".ag1")?,
                _ => self.config.cwd.clone(),
            };
            self.local(input, &base)?
        } else {
            String::new()
        };
        Ok(parsed.identity(&local))
    }
    fn validate_configured(entries: &[ConfiguredPackage]) -> Result<(), DomainError> {
        if entries.len() > 4096
            || entries.iter().map(|e| e.source.len()).sum::<usize>() > 4 * 1024 * 1024
        {
            return Err(DomainError::LimitExceeded("configured packages".into()));
        }
        for entry in entries {
            if entry.scope == Scope::Temporary {
                return Err(invalid("temporary package in persisted configuration"));
            }
            packages::parse(&entry.source)?;
        }
        Ok(())
    }
    /// User settings precede project settings for explicit updates. A matching
    /// pinned/local/offline entry still counts as a match; update_source handles
    /// those no-ops. This plan performs no I/O and grants no execution authority.
    pub fn configured_updates(
        &self,
        entries: &[ConfiguredPackage],
        requested: Option<&str>,
    ) -> Result<Vec<ConfiguredPackage>, DomainError> {
        Self::validate_configured(entries)?;
        let identity = requested
            .filter(|s| !s.is_empty())
            .map(|s| self.configured_identity(s, None))
            .transpose()?;
        let ordered = [Scope::User, Scope::Project]
            .into_iter()
            .flat_map(|scope| entries.iter().filter(move |entry| entry.scope == scope))
            .collect::<Vec<_>>();
        let mut selected = Vec::new();
        for entry in &ordered {
            let candidate = self.configured_identity(&entry.source, Some(entry.scope))?;
            if identity
                .as_ref()
                .is_none_or(|identity| candidate == *identity)
            {
                selected.push((*entry).clone());
            }
        }
        if selected.is_empty()
            && let Some(requested) = requested.filter(|s| !s.is_empty())
        {
            let mut message = format!("No matching package found for {requested}");
            for entry in ordered {
                let suggested = match packages::parse(&entry.source)? {
                    Source::Npm { name, spec, .. } => {
                        requested.trim() == name || requested.trim() == spec
                    }
                    Source::Git {
                        host,
                        path,
                        reference,
                        ..
                    } => {
                        let short = format!("{host}/{path}");
                        requested.trim() == short
                            || reference.is_some_and(|r| requested.trim() == format!("{short}@{r}"))
                    }
                    Source::Local { .. } => false,
                };
                if suggested {
                    message.push_str(&format!(". Did you mean {}?", entry.source));
                    break;
                }
            }
            return Err(invalid(&message));
        }
        Ok(selected)
    }
    /// Project wins identity collisions, including when its entry is pinned.
    /// Installation existence and remote queries occur only after this selection.
    pub fn update_check_candidates(
        &self,
        entries: &[ConfiguredPackage],
    ) -> Result<Vec<ConfiguredPackage>, DomainError> {
        if self.config.offline {
            return Ok(Vec::new());
        }
        Self::validate_configured(entries)?;
        let mut seen = std::collections::BTreeSet::new();
        let mut selected = Vec::new();
        for scope in [Scope::Project, Scope::User] {
            for entry in entries.iter().filter(|entry| entry.scope == scope) {
                let identity = self.configured_identity(&entry.source, Some(scope))?;
                if !seen.insert(identity) {
                    continue;
                }
                match packages::parse(&entry.source)? {
                    Source::Npm { pinned: false, .. } | Source::Git { pinned: false, .. } => {
                        selected.push(entry.clone());
                    }
                    _ => {}
                }
            }
        }
        Ok(selected)
    }
    /// Build read queries from the captured configuration. The supplied port may
    /// resolve the native npm root, but this method never fetches a registry or
    /// starts an installation. Query transport needs its own host capabilities.
    pub fn prepare_update_checks(
        &mut self,
        port: &mut impl Port,
        entries: &[ConfiguredPackage],
    ) -> Result<Vec<super::package_queries::Query>, DomainError> {
        let selected = self.update_check_candidates(entries)?;
        let mut queries = Vec::with_capacity(selected.len());
        for entry in selected {
            port.checkpoint()?;
            let parsed = packages::parse(&entry.source)?;
            let installed = self.installed_path(port, &parsed, entry.scope)?;
            queries.push(super::package_queries::Query::new(
                entry.source,
                entry.scope,
                installed,
            )?);
        }
        Ok(queries)
    }
    /// Approved host orchestration. Each source retains its own SDK progress
    /// sequence; a failed source stops the batch. The outer owner must journal
    /// the batch before calling this mutating method.
    pub fn update_configured(
        &mut self,
        port: &mut impl Port,
        entries: &[ConfiguredPackage],
        requested: Option<&str>,
    ) -> Result<(), DomainError> {
        for entry in self.configured_updates(entries, requested)? {
            port.checkpoint()?;
            self.update_source(port, &entry.source, entry.scope)?;
        }
        Ok(())
    }
    /// Resolving an installed unpinned package must never consult the registry.
    /// A true result is a request for an approved install, not permission to run it.
    pub fn needs_install(
        &mut self,
        port: &mut impl Port,
        source: &Source,
        scope: Scope,
    ) -> Result<bool, DomainError> {
        port.checkpoint()?;
        if matches!(source, Source::Local { .. }) {
            return Ok(false);
        }
        let installed = self.installed_path(port, source, scope)?;
        if !port.exists(&installed)? {
            return Ok(true);
        }
        if matches!(source, Source::Npm { pinned: true, .. }) {
            return self
                .pinned_matches(port, source, &installed)
                .map(|value| !value);
        }
        Ok(false)
    }
    pub fn new(config: Config) -> Result<Self, DomainError> {
        for root in [
            &config.cwd,
            &config.agent_dir,
            &config.home,
            &config.temporary_dir,
        ] {
            if path(root, "")? != *root {
                return Err(invalid("normalized package roots"));
            }
        }
        packages::npm_command(Some(
            &std::iter::once(config.npm.command.clone())
                .chain(config.npm.args.clone())
                .collect::<Vec<_>>(),
        ))?;
        Ok(Self {
            config,
            global_npm: None,
        })
    }
    pub fn set_npm(&mut self, npm: Command) -> Result<(), DomainError> {
        packages::npm_command(Some(
            &std::iter::once(npm.command.clone())
                .chain(npm.args.clone())
                .collect::<Vec<_>>(),
        ))?;
        self.config.npm = npm;
        Ok(())
    }
    fn run(
        &self,
        port: &mut impl Port,
        command: &str,
        args: Vec<String>,
        cwd: Option<&str>,
        capture: bool,
    ) -> Result<String, DomainError> {
        port.checkpoint()?;
        if args.len() > 128 || args.iter().any(|s| s.len() > 8192 || s.contains('\0')) {
            return Err(DomainError::LimitExceeded(
                "package command arguments".into(),
            ));
        }
        let command = Run {
            kind: if capture { "capture" } else { "run" }.into(),
            command: command.into(),
            args,
            options: cwd.map(|cwd| Options {
                cwd: cwd.into(),
                timeout_ms: capture.then_some(10000),
                env: None,
            }),
        };
        let result = port.run(command)?;
        if result.len() > 65536 {
            return Err(DomainError::LimitExceeded("package command capture".into()));
        }
        Ok(result)
    }
    fn npm(
        &self,
        port: &mut impl Port,
        args: Vec<String>,
        cwd: Option<&str>,
    ) -> Result<(), DomainError> {
        let mut argv = self.config.npm.args.clone();
        argv.extend(args);
        self.run(port, &self.config.npm.command, argv, cwd, false)?;
        Ok(())
    }
    pub fn global_npm_root(&mut self, port: &mut impl Port) -> Result<String, DomainError> {
        if let Some((command, root)) = &self.global_npm
            && command == &self.config.npm
        {
            return Ok(root.clone());
        }
        port.checkpoint()?;
        let mut args = self.config.npm.args.clone();
        args.extend(["root".into(), "-g".into()]);
        let root = port
            .run(Run {
                kind: "sync".into(),
                command: self.config.npm.command.clone(),
                args,
                options: None,
            })?
            .trim()
            .to_owned();
        if path(&root, "")? != root {
            return Err(invalid("npm root response"));
        }
        self.global_npm = Some((self.config.npm.clone(), root.clone()));
        Ok(root)
    }
    fn temporary(&self, prefix: &str) -> Result<String, DomainError> {
        let hash = format!("{:x}", Sha256::digest(format!("{prefix}-").as_bytes()));
        path(
            &self.config.temporary_dir,
            &format!("ag1-extensions/{prefix}/{}", &hash[..8]),
        )
    }
    fn npm_root(
        &mut self,
        port: &mut impl Port,
        scope: Scope,
        temporary: bool,
    ) -> Result<String, DomainError> {
        if temporary {
            return self.temporary("npm");
        }
        if scope == Scope::Project {
            return path(&self.config.cwd, ".ag1/npm");
        }
        path(&self.global_npm_root(port)?, "..")
    }
    fn git_root(&self, scope: Scope) -> Result<Option<String>, DomainError> {
        match scope {
            Scope::Temporary => Ok(None),
            Scope::Project => path(&self.config.cwd, ".ag1/git").map(Some),
            Scope::User => path(&self.config.agent_dir, "git").map(Some),
        }
    }
    pub fn installed_path(
        &mut self,
        port: &mut impl Port,
        source: &Source,
        scope: Scope,
    ) -> Result<String, DomainError> {
        match source {
            Source::Npm { name, .. } => {
                if scope == Scope::User {
                    contained(&self.global_npm_root(port)?, name)
                } else {
                    let root = self.npm_root(port, scope, scope == Scope::Temporary)?;
                    contained(&root, &format!("node_modules/{name}"))
                }
            }
            Source::Git {
                host, path: repo, ..
            } => {
                if scope == Scope::Temporary {
                    let hash = format!(
                        "{:x}",
                        Sha256::digest(format!("git-{host}-{repo}").as_bytes())
                    );
                    let root = path(
                        &self.config.temporary_dir,
                        &format!("ag1-extensions/git-{host}/{}", &hash[..8]),
                    )?;
                    contained(&root, repo)
                } else {
                    contained(&self.git_root(scope)?.unwrap(), &format!("{host}/{repo}"))
                }
            }
            Source::Local { path: input } => {
                let base = match scope {
                    Scope::Project => path(&self.config.cwd, ".ag1")?,
                    Scope::User => self.config.agent_dir.clone(),
                    Scope::Temporary => self.config.cwd.clone(),
                };
                self.local(input, &base)
            }
        }
    }
    fn local(&self, input: &str, base: &str) -> Result<String, DomainError> {
        let input = input.trim();
        if input == "~" {
            return Ok(self.config.home.clone());
        }
        if let Some(rest) = input.strip_prefix('~') {
            path(&self.config.home, rest.trim_start_matches('/'))
        } else {
            path(base, input)
        }
    }
    fn ensure_ignore(&self, port: &mut impl Port, root: &str) -> Result<(), DomainError> {
        port.checkpoint()?;
        port.create_directory(root)?;
        let ignore = path(root, ".gitignore")?;
        if !port.exists(&ignore)? {
            port.write_absent(&ignore, b"*\n!.gitignore\n")?;
        }
        Ok(())
    }
    fn ensure_npm(&self, port: &mut impl Port, root: &str) -> Result<(), DomainError> {
        self.ensure_ignore(port, root)?;
        let manifest = path(root, "package.json")?;
        if !port.exists(&manifest)? {
            port.write_absent(
                &manifest,
                b"{\n  \"name\": \"ag1-extensions\",\n  \"private\": true\n}",
            )?;
        }
        Ok(())
    }
    fn install_npm(
        &mut self,
        port: &mut impl Port,
        spec: &str,
        scope: Scope,
        temporary: bool,
    ) -> Result<(), DomainError> {
        if scope == Scope::User && !temporary {
            return self.npm(port, vec!["install".into(), "-g".into(), spec.into()], None);
        }
        let root = self.npm_root(port, scope, temporary)?;
        self.ensure_npm(port, &root)?;
        self.npm(
            port,
            vec!["install".into(), spec.into(), "--prefix".into(), root],
            None,
        )
    }
    fn install_git(
        &mut self,
        port: &mut impl Port,
        source: &Source,
        scope: Scope,
    ) -> Result<(), DomainError> {
        let Source::Git {
            repo, reference, ..
        } = source
        else {
            return Err(invalid("git package source"));
        };
        let target = self.installed_path(port, source, scope)?;
        if port.exists(&target)? {
            return Ok(());
        }
        if let Some(root) = self.git_root(scope)? {
            self.ensure_ignore(port, &root)?;
        }
        let parent = Path::new(&target)
            .parent()
            .and_then(Path::to_str)
            .ok_or_else(|| invalid("package parent"))?;
        port.create_directory(parent)?;
        self.run(
            port,
            "git",
            vec!["clone".into(), repo.clone(), target.clone()],
            None,
            false,
        )?;
        if let Some(reference) = reference {
            if reference.starts_with('-') || reference.contains(['\0', '\n', '\r']) {
                return Err(invalid("unsafe Git checkout ref"));
            }
            self.run(
                port,
                "git",
                vec!["checkout".into(), reference.clone()],
                Some(&target),
                false,
            )?;
        }
        if port.exists(&path(&target, "package.json")?)? {
            self.npm(port, vec!["install".into()], Some(&target))?;
        }
        Ok(())
    }
    pub fn pinned_matches(
        &mut self,
        port: &mut impl Port,
        source: &Source,
        installed: &str,
    ) -> Result<bool, DomainError> {
        let Source::Npm { .. } = source else {
            return Err(invalid("npm package source"));
        };
        let manifest = port.read_json(&path(installed, "package.json")?)?;
        packages::pinned_npm_matches(source, manifest.as_ref())
    }
    pub fn install(
        &mut self,
        port: &mut impl Port,
        source: &str,
        scope: Scope,
    ) -> Result<(), DomainError> {
        let parsed = packages::parse(source)?;
        validate_execution_source(&parsed)?;
        self.with_progress(
            port,
            "install",
            source,
            format!("Installing {source}..."),
            |this, port| match &parsed {
                Source::Npm { spec, .. } => {
                    this.install_npm(port, spec, scope, scope == Scope::Temporary)
                }
                Source::Git { .. } => this.install_git(port, &parsed, scope),
                Source::Local { path: input } => {
                    let target = this.local(input, &this.config.cwd)?;
                    if port.exists(&target)? {
                        Ok(())
                    } else {
                        Err(invalid(&format!("Path does not exist: {target}")))
                    }
                }
            },
        )
    }
    pub fn remove(
        &mut self,
        port: &mut impl Port,
        source: &str,
        scope: Scope,
    ) -> Result<(), DomainError> {
        let parsed = packages::parse(source)?;
        validate_execution_source(&parsed)?;
        self.with_progress(
            port,
            "remove",
            source,
            format!("Removing {source}..."),
            |this, port| match &parsed {
                Source::Npm { name, .. } => {
                    if scope == Scope::User {
                        return this.npm(
                            port,
                            vec!["uninstall".into(), "-g".into(), name.clone()],
                            None,
                        );
                    }
                    let root = this.npm_root(port, scope, scope == Scope::Temporary)?;
                    if port.exists(&root)? {
                        this.npm(
                            port,
                            vec!["uninstall".into(), name.clone(), "--prefix".into(), root],
                            None,
                        )?;
                    }
                    Ok(())
                }
                Source::Git { .. } => {
                    let target = this.installed_path(port, &parsed, scope)?;
                    if port.exists(&target)? {
                        port.remove_owned_tree(&target, this.git_root(scope)?.as_deref())?;
                    }
                    Ok(())
                }
                Source::Local { .. } => Ok(()),
            },
        )
    }
    fn with_progress<P: Port>(
        &mut self,
        port: &mut P,
        action: &str,
        source: &str,
        message: String,
        run: impl FnOnce(&mut Self, &mut P) -> Result<(), DomainError>,
    ) -> Result<(), DomainError> {
        port.checkpoint()?;
        port.progress(json!({"type":"start","action":action,"source":source,"message":message}))?;
        let result = run(self, port);
        match &result{Ok(())=>port.progress(json!({"type":"complete","action":action,"source":source}))?,Err(error)=>port.progress(json!({"type":"error","action":action,"source":source,"message":error_message(error)}))?,}
        result
    }
    pub fn update_source(
        &mut self,
        port: &mut impl Port,
        source: &str,
        scope: Scope,
    ) -> Result<(), DomainError> {
        if self.config.offline {
            return Ok(());
        }
        let parsed = packages::parse(source)?;
        validate_execution_source(&parsed)?;
        match &parsed {
            Source::Local { .. }
            | Source::Npm { pinned: true, .. }
            | Source::Git { pinned: true, .. } => return Ok(()),
            _ => {}
        }
        self.with_progress(
            port,
            "update",
            source,
            format!("Updating {source}..."),
            |this, port| match &parsed {
                Source::Npm { name, .. } => this.install_npm(
                    port,
                    &format!("{name}@latest"),
                    scope,
                    scope == Scope::Temporary,
                ),
                Source::Git { .. } => this.update_git(port, &parsed, scope),
                _ => Ok(()),
            },
        )
    }
    fn git_capture(
        &self,
        port: &mut impl Port,
        target: &str,
        args: Vec<String>,
    ) -> Result<String, DomainError> {
        self.run(port, "git", args, Some(target), true)
    }
    fn update_git(
        &mut self,
        port: &mut impl Port,
        source: &Source,
        scope: Scope,
    ) -> Result<(), DomainError> {
        let target = self.installed_path(port, source, scope)?;
        if !port.exists(&target)? {
            return self.install_git(port, source, scope);
        }
        let upstream = (|| {
            let upstream = self.git_capture(
                port,
                &target,
                vec![
                    "rev-parse".into(),
                    "--abbrev-ref".into(),
                    "@{upstream}".into(),
                ],
            )?;
            let branch = upstream
                .trim()
                .strip_prefix("origin/")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("unsupported upstream"))?
                .to_owned();
            self.git_capture(
                port,
                &target,
                vec!["rev-parse".into(), "@{upstream}".into()],
            )?;
            Ok::<_, DomainError>(("@{upstream}".to_owned(), branch))
        })();
        let (reference, branch) = match upstream {
            Ok(pair) => pair,
            Err(error) => {
                if matches!(error, DomainError::Cancelled | DomainError::TimedOut) {
                    return Err(error);
                }
                let fallback = self.run(
                    port,
                    "git",
                    vec![
                        "remote".into(),
                        "set-head".into(),
                        "origin".into(),
                        "-a".into(),
                    ],
                    Some(&target),
                    false,
                );
                if matches!(
                    fallback,
                    Err(DomainError::Cancelled | DomainError::TimedOut)
                ) {
                    fallback?;
                }
                self.git_capture(
                    port,
                    &target,
                    vec!["rev-parse".into(), "origin/HEAD".into()],
                )?;
                let branch = match self.git_capture(
                    port,
                    &target,
                    vec!["symbolic-ref".into(), "refs/remotes/origin/HEAD".into()],
                ) {
                    Ok(value) => value,
                    Err(error @ (DomainError::Cancelled | DomainError::TimedOut)) => {
                        return Err(error);
                    }
                    Err(_) => String::new(),
                };
                (
                    "origin/HEAD".into(),
                    branch
                        .trim()
                        .strip_prefix("refs/remotes/origin/")
                        .unwrap_or(branch.trim())
                        .to_owned(),
                )
            }
        };
        if branch.len() > 4096 || branch.contains(['\0', '\n', '\r', ':']) {
            return Err(invalid("unsafe Git upstream ref"));
        }
        let refspec = if branch.is_empty() {
            "+HEAD:refs/remotes/origin/HEAD".into()
        } else {
            format!("+refs/heads/{branch}:refs/remotes/origin/{branch}")
        };
        self.run(
            port,
            "git",
            vec![
                "fetch".into(),
                "--prune".into(),
                "--no-tags".into(),
                "origin".into(),
                refspec,
            ],
            Some(&target),
            false,
        )?;
        let local = self.git_capture(port, &target, vec!["rev-parse".into(), "HEAD".into()])?;
        let remote =
            self.git_capture(port, &target, vec!["rev-parse".into(), reference.clone()])?;
        if local.trim() == remote.trim() {
            return Ok(());
        }
        self.run(
            port,
            "git",
            vec!["reset".into(), "--hard".into(), reference],
            Some(&target),
            false,
        )?;
        self.run(
            port,
            "git",
            vec!["clean".into(), "-fdx".into()],
            Some(&target),
            false,
        )?;
        if port.exists(&path(&target, "package.json")?)? {
            self.npm(port, vec!["install".into()], Some(&target))?;
        }
        Ok(())
    }
    pub fn refresh_temporary(
        &mut self,
        port: &mut impl Port,
        source: &str,
    ) -> Result<(), DomainError> {
        if self.config.offline {
            return Ok(());
        }
        let parsed = packages::parse(source)?;
        validate_execution_source(&parsed)?;
        let result = self.with_progress(
            port,
            "pull",
            source,
            format!("Refreshing {source}..."),
            |this, port| this.update_git(port, &parsed, Scope::Temporary),
        );
        if matches!(result, Err(DomainError::Cancelled | DomainError::TimedOut)) {
            result
        } else {
            Ok(())
        }
    }
}
