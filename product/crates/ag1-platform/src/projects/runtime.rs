use crate::{
    io_error,
    process::{OwnedProcess, PINNED_CWD_FD, ProcessSpec, ProcessSupervisor},
    resource_fs::ScanControl,
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::{
    DomainError,
    projects::{Availability, Diagnostic, Kind, Lifecycle, Operation, Project, Target},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Explicit host configuration, never accepted from generated applications.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Command {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Toolchains {
    pub node: Option<PathBuf>,
    pub npm: Option<PathBuf>,
    pub cargo: Option<PathBuf>,
    pub rustc: Option<PathBuf>,
    pub godot: Option<PathBuf>,
    /// Individually admitted, read-only runtime roots. Never pass a user home.
    pub mounts: Vec<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReadyHttp {
    pub address: SocketAddr,
    pub path: String,
    pub expected_body: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Launch {
    pub commands: Vec<Command>,
    pub readiness: Option<ReadyHttp>,
    pub timeout_ms: u64,
}
struct Running {
    children: Vec<OwnedProcess>,
    stdout: Vec<fs::File>,
    stdout_bytes: Vec<usize>,
    started: Instant,
    launch: Launch,
    record: Operation,
}
struct WriterLease(fs::File);
impl Drop for WriterLease {
    fn drop(&mut self) {
        // Another thread can fork while this descriptor is open. Closing only
        // our copy leaves flock held until that child's pre-exec copy closes.
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}
/// The caller serializes this owner with its existing session/command actor.
/// Dropping it disconnects P03 groups; history never grants process control.
pub struct Runtime {
    pub workspace: Arc<Workspace>,
    pub project: Project,
    pub toolchains: Toolchains,
    supervisor: ProcessSupervisor,
    state: Arc<Workspace>,
    running: Option<Running>,
    serial: u64,
    display: Option<super::display::NativeDisplay>,
    git_source: Option<crate::git::GitService>,
    // Release only after owned project processes and display have dropped.
    _writer: WriterLease,
}
fn invalid(s: &str) -> DomainError {
    DomainError::InvalidInput(s.into())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn document<T: Serialize>(value: &T) -> Result<Vec<u8>, DomainError> {
    serde_json::to_vec_pretty(value).map_err(|_| invalid("project document"))
}

impl Runtime {
    /// Catalog and logs belong in a private destination outside the generated app.
    /// Loading reads an existing identity; it never rewrites project/session IDs.
    pub fn load(
        workspace: Arc<Workspace>,
        state: Arc<Workspace>,
        helper: &Path,
        toolchains: Toolchains,
    ) -> Result<Self, DomainError> {
        if state.root().starts_with(workspace.root()) || workspace.root().starts_with(state.root())
        {
            return Err(DomainError::PermissionDenied);
        }
        workspace.verify_root()?;
        state.verify_root()?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(state.root(), fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
        let writer = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(state.root().join("project-writer.lock"))
            .map_err(io_error)?;
        if unsafe { libc::flock(writer.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(DomainError::Conflict);
        }
        let writer = WriterLease(writer);
        let identity = if state.exists("project.json")? {
            serde_json::from_slice::<Value>(&state.snapshot("project.json", 65536)?.bytes)
                .map_err(|_| invalid("retained corrupt project catalog"))?
        } else {
            let mut random = [0; 32];
            fs::File::open("/dev/urandom")
                .map_err(io_error)?
                .read_exact(&mut random)
                .map_err(io_error)?;
            let value = json!({"id":format!("project-{}",digest(&random)),"session":format!("session-{}",digest(&random[..16])),"workspace":workspace.id().as_str()});
            state.create_checked("project.json", &document(&value)?, false, || Ok(()))?;
            value
        };
        if identity["workspace"] != workspace.id().as_str() {
            return Err(DomainError::Conflict);
        }
        let id = identity["id"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("project ID"))?
            .to_string();
        let session = identity["session"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("session ID"))?
            .to_string();
        let mut serial = 0;
        for file in state.scan(1000)? {
            if let Some(n) = file
                .strip_prefix("operation-")
                .or_else(|| file.strip_prefix("git-recovery-"))
                .and_then(|s| s.strip_suffix(".json"))
                .and_then(|s| s.parse::<u64>().ok())
            {
                serial = serial.max(n);
            }
        }
        let mut runtime = Self {
            workspace,
            state,
            supervisor: ProcessSupervisor::new(helper)?,
            toolchains,
            project: Project {
                id,
                session,
                targets: vec![],
            },
            running: None,
            serial,
            _writer: writer,
            display: None,
            git_source: None,
        };
        runtime.discover()?;
        Ok(runtime)
    }
    /// Create exactly one new child of an admitted parent. Failure cleanup uses
    /// the pinned inode, so a replaced path can never become a deletion target.
    pub fn create(
        parent: &Arc<Workspace>,
        name: &str,
        files: &BTreeMap<String, Vec<u8>>,
        control: &ScanControl<'_>,
    ) -> Result<Arc<Workspace>, DomainError> {
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || !matches!(
                Path::new(name).components().next(),
                Some(std::path::Component::Normal(_))
            )
            || files.is_empty()
            || files.len() > 100
        {
            return Err(invalid("new project destination/files"));
        }
        parent.create_directory_document_checked(name, ".ag1-project-created", b"1\n", || {
            control.check()
        })?;
        let pin = parent.pin_directory(name)?;
        let result = (|| {
            let workspace = WorkspaceService::new()?.register(&parent.root().join(name))?;
            for (path, bytes) in files {
                workspace.create_checked(path, bytes, true, || control.check())?;
            }
            Ok(workspace)
        })();
        if result.is_err() {
            parent.remove_directory_tree_checked(pin, || Ok(()))?;
        }
        result
    }
    pub fn discover(&mut self) -> Result<(), DomainError> {
        self.workspace.verify_root()?;
        // Target discovery concerns root manifests and Tauri configuration.
        // Unrelated source, caches and archived environments must not prevent
        // opening a project. Execution still binds its full source revision.
        let roots = self.workspace.directory_entries("")?;
        let mut paths: Vec<String> = roots
            .iter()
            .filter(|e| !e.directory && !e.symlink)
            .map(|e| e.name.clone())
            .collect();
        if roots
            .iter()
            .any(|e| e.name == "src-tauri" && e.directory && !e.symlink)
        {
            let mut pending = vec!["src-tauri".to_owned()];
            let mut visited = 0;
            while let Some(directory) = pending.pop() {
                visited += 1;
                if visited > 1000 || directory.split('/').count() > 32 {
                    return Err(DomainError::LimitExceeded(
                        "Tauri configuration traversal".into(),
                    ));
                }
                for entry in self.workspace.directory_entries(&directory)? {
                    if entry.symlink
                        || [".git", "node_modules", "target", ".ag1"].contains(&entry.name.as_str())
                    {
                        continue;
                    }
                    let path = format!("{directory}/{}", entry.name);
                    if entry.directory {
                        pending.push(path);
                    } else if path.ends_with(".json") || path.ends_with(".toml") {
                        if paths.len() >= 10000 {
                            return Err(DomainError::LimitExceeded(
                                "Tauri configuration files".into(),
                            ));
                        }
                        paths.push(path);
                    }
                }
            }
        }
        paths.sort();
        let mut targets = vec![];
        for (manifest, kind, tool) in [
            ("package.json", Kind::Web, &self.toolchains.node),
            ("Cargo.toml", Kind::Rust, &self.toolchains.cargo),
            (
                "src-tauri/tauri.conf.json",
                Kind::Tauri,
                &self.toolchains.cargo,
            ),
            ("project.godot", Kind::Godot, &self.toolchains.godot),
        ] {
            if !paths.iter().any(|p| p == manifest) {
                continue;
            }
            let mut manifests = vec![manifest.into()];
            if kind == Kind::Tauri {
                manifests.extend(
                    paths
                        .iter()
                        .filter(|p| {
                            p.starts_with("src-tauri/")
                                && (p.ends_with(".toml") || p.ends_with(".json"))
                        })
                        .cloned(),
                );
                if paths.iter().any(|p| p == "package.json") {
                    manifests.push("package.json".into());
                }
                // Parse the configuration, retaining plugin/capability inputs in the revision.
                let _: Value =
                    serde_json::from_slice(&self.workspace.snapshot(manifest, 1024 * 1024)?.bytes)
                        .map_err(|_| invalid("Tauri config JSON"))?;
            }
            let mut capabilities = BTreeMap::new();
            for action in ["install", "build", "run", "check"] {
                capabilities.insert(
                    action.into(),
                    match self.command(kind, action) {
                        Ok(_) => Availability::yes(),
                        Err(error) => Availability::no(format!("{error:?}")),
                    },
                );
            }
            capabilities.insert("preview".into(), Availability::yes());
            capabilities.insert(
                "embed".into(),
                Availability::no(
                    "Native window embedding is not supported; use launch/observe/stop",
                ),
            );
            capabilities.insert(
                "screenshot".into(),
                Availability::no("Admit a separate automation browser or authenticated Godot host"),
            );
            capabilities.insert(
                "package".into(),
                Availability::no(
                    "Select an installed platform packaging toolchain and explicit package command",
                ),
            );
            if kind == Kind::Godot {
                capabilities.insert(
                    "host".into(),
                    Availability::no("Connect the version/hash/nonce authenticated Godot host"),
                );
            }
            targets.push(Target {
                id: format!("{}:{kind:?}", self.project.id),
                kind,
                workspace: self.workspace.id().as_str().into(),
                manifests,
                toolchain: tool.iter().map(|p| p.display().to_string()).collect(),
                capabilities,
                lifecycle: Lifecycle::Idle,
            });
        }
        targets.push(Target {
            id: format!("{}:Native", self.project.id),
            kind: Kind::Native,
            workspace: self.workspace.id().as_str().into(),
            manifests: vec![],
            toolchain: vec![],
            capabilities: BTreeMap::from([
                ("run".into(), Availability::yes()),
                ("preview".into(), Availability::yes()),
                (
                    "focus".into(),
                    Availability::no("No owned native window handle has been admitted"),
                ),
                (
                    "embed".into(),
                    Availability::no("Native embedding unavailable"),
                ),
            ]),
            lifecycle: Lifecycle::Idle,
        });
        for record in self.history()? {
            if let Some(target) = targets.iter_mut().find(|t| t.id == record.target) {
                target.lifecycle = record.lifecycle;
            }
        }
        self.project.targets = targets;
        Ok(())
    }
    pub fn revision(&self) -> Result<String, DomainError> {
        if let Some(git) = &self.git_source {
            let files = git.files_bounded(&self.workspace, 10000)?;
            let mut rows = Vec::new();
            let mut total = 0u64;
            for path in &files {
                let revision =
                    self.workspace
                        .stream(path, crate::workspace::MAX_STREAM_READ, |bytes| {
                            total += bytes.len() as u64;
                            if total > 1024 * 1024 * 1024 {
                                return Err(DomainError::LimitExceeded(
                                    "Project source revision bytes".into(),
                                ));
                            }
                            Ok(())
                        });
                match revision {
                    Ok(r) => rows.push(json!([
                        path,
                        r.sha256,
                        r.size,
                        r.device,
                        r.inode,
                        r.modified_ns.to_string(),
                        r.changed_ns.to_string()
                    ])),
                    Err(DomainError::NotFound) => rows.push(json!([path, "deleted"])),
                    Err(error) => return Err(error),
                }
            }
            if git.files_bounded(&self.workspace, 10000)? != files {
                return Err(DomainError::Conflict);
            }
            self.workspace.verify_root()?;
            return Ok(digest(&document(&rows)?));
        }
        let files = self.workspace.scan(1000)?;
        let index = self.workspace.index_paths(&files, 16 * 1024 * 1024)?;
        let rows: Vec<_> = index
            .files
            .iter()
            .map(|(p, r)| {
                json!([
                    p,
                    r.sha256,
                    r.size,
                    r.device,
                    r.inode,
                    r.modified_ns.to_string(),
                    r.changed_ns.to_string()
                ])
            })
            .collect();
        self.workspace.verify_root()?;
        Ok(digest(&document(&rows)?))
    }
    /// Git projects use the existing P03 tracked/untracked source inventory.
    /// Ignored dependencies and archived environments are not build inputs in
    /// that inventory. Every included file still participates in the revision.
    pub fn bind_git_source(&mut self, helper: &Path, git: &Path) -> Result<(), DomainError> {
        if self.running.is_some() {
            return Err(DomainError::Conflict);
        }
        if self
            .workspace
            .directory_entries("")?
            .iter()
            .any(|e| e.name == ".git" && !e.symlink)
        {
            self.git_source = Some(crate::git::GitService::new(helper, git, self.state.root())?);
        }
        Ok(())
    }
    pub fn command(&self, kind: Kind, action: &str) -> Result<Vec<Command>, DomainError> {
        let command = |executable: &Option<PathBuf>,
                       argv: Vec<String>,
                       cwd: &str|
         -> Result<Command, DomainError> {
            Ok(Command {
                executable: executable.clone().filter(|p| p.is_file()).ok_or_else(|| {
                    DomainError::Unavailable(format!("Configure installed {kind:?} toolchain"))
                })?,
                argv,
                cwd: cwd.into(),
            })
        };
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let frontend = || -> Result<Command, DomainError> {
            let package: Value = serde_json::from_slice(
                &self.workspace.snapshot("package.json", 1024 * 1024)?.bytes,
            )
            .map_err(|_| invalid("package.json"))?;
            let script = match action {
                "check" | "build" => "build",
                "run" => "dev",
                "install" => "",
                _ => return Err(invalid("project action")),
            };
            if !script.is_empty() && !package["scripts"][script].is_string() {
                return Err(DomainError::Unavailable(format!(
                    "Add package.json scripts.{script}"
                )));
            }
            let npm = self
                .toolchains
                .npm
                .as_ref()
                .filter(|p| p.is_file())
                .ok_or_else(|| DomainError::Unavailable("Configure npm CLI".into()))?;
            let mut argv = vec![npm.display().to_string()];
            argv.extend(if action == "install" {
                strings(&["ci", "--no-audit", "--no-fund", "--cache", "target/ag1-npm"])
            } else {
                strings(&["run", script])
            });
            command(&self.toolchains.node, argv, "")
        };
        match kind {
            Kind::Web => Ok(vec![frontend()?]),
            Kind::Rust | Kind::Tauri => {
                let verb = match action {
                    "install" => "fetch",
                    "build" => "build",
                    "check" => "check",
                    "run" => "run",
                    _ => return Err(invalid("Cargo action")),
                };
                let mut argv = vec![verb.into()];
                if action != "install" {
                    argv.push("--message-format=json".into());
                }
                let native = command(
                    &self.toolchains.cargo,
                    argv,
                    if kind == Kind::Tauri { "src-tauri" } else { "" },
                )?;
                if kind == Kind::Tauri && self.workspace.exists("package.json")? {
                    let mut frontend = frontend()?;
                    if action == "run" {
                        let config: Value = serde_json::from_slice(
                            &self
                                .workspace
                                .snapshot("src-tauri/tauri.conf.json", 1024 * 1024)?
                                .bytes,
                        )
                        .map_err(|_| invalid("Tauri config"))?;
                        if let Some(url) = config["build"]["devUrl"].as_str() {
                            let url =
                                reqwest::Url::parse(url).map_err(|_| invalid("Tauri devUrl"))?;
                            if url.scheme() != "http"
                                || !matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
                            {
                                return Err(DomainError::Unavailable(
                                    "Configure a loopback HTTP Tauri devUrl".into(),
                                ));
                            }
                            let package: Value = serde_json::from_slice(
                                &self.workspace.snapshot("package.json", 1024 * 1024)?.bytes,
                            )
                            .map_err(|_| invalid("package.json"))?;
                            if package["scripts"]["dev"]
                                .as_str()
                                .is_some_and(|s| s.split_whitespace().next() == Some("vite"))
                            {
                                frontend.argv.extend([
                                    "--".into(),
                                    "--host".into(),
                                    "127.0.0.1".into(),
                                    "--port".into(),
                                    url.port_or_known_default().unwrap().to_string(),
                                    "--strictPort".into(),
                                ]);
                            }
                        }
                    }
                    Ok(vec![frontend, native])
                } else {
                    Ok(vec![native])
                }
            }
            Kind::Godot => Ok(vec![command(
                &self.toolchains.godot,
                match action {
                    "install" | "check" | "build" => {
                        strings(&["--headless", "--editor", "--import", "--quit"])
                    }
                    "run" => vec![],
                    _ => return Err(invalid("Godot action")),
                },
                "",
            )?]),
            Kind::Native if action == "check" => Ok(vec![
                super::checks::detect(&self.workspace, &self.toolchains)?.1,
            ]),
            Kind::Native => Err(DomainError::Unavailable(
                "Admit an explicit native command".into(),
            )),
        }
    }
    fn save(&mut self, record: &Operation) -> Result<(), DomainError> {
        let name = format!("{}.json", record.id);
        let bytes = document(record)?;
        if self.state.exists(&name)? {
            let old = self.state.snapshot(&name, 1024 * 1024)?;
            self.state.replace(&name, &old.revision, &bytes)?;
        } else {
            self.state.create_checked(&name, &bytes, false, || Ok(()))?;
        }
        if let Some(target) = self
            .project
            .targets
            .iter_mut()
            .find(|t| t.id == record.target)
        {
            target.lifecycle = record.lifecycle;
        }
        Ok(())
    }
    pub fn history(&self) -> Result<Vec<Operation>, DomainError> {
        let mut records = vec![];
        for file in self.state.scan(1000)? {
            if file.starts_with("operation-") && file.ends_with(".json") {
                let mut record: Operation =
                    serde_json::from_slice(&self.state.snapshot(&file, 1024 * 1024)?.bytes)
                        .map_err(|_| invalid("corrupt operation history retained"))?;
                if matches!(record.lifecycle, Lifecycle::Starting | Lifecycle::Ready)
                    && self
                        .running
                        .as_ref()
                        .is_none_or(|r| r.record.id != record.id)
                {
                    record.lifecycle = Lifecycle::RecoveryRequired;
                }
                records.push(record);
            }
        }
        records.sort_by_key(|r| {
            r.id.strip_prefix("operation-")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0)
        });
        Ok(records)
    }
    fn spawn(
        &self,
        command: &Command,
        log: &Path,
        timeout_ms: u64,
    ) -> Result<OwnedProcess, DomainError> {
        let root = self
            .workspace
            .root()
            .to_str()
            .ok_or(DomainError::PermissionDenied)?;
        if !command.executable.is_absolute() || !command.executable.is_file() {
            return Err(DomainError::Unavailable(
                "Executable missing; configure the target toolchain".into(),
            ));
        }
        let pinned = self.workspace.pin_directory("")?;
        self.workspace
            .pin_directory(&command.cwd)?
            .verify_identity()?;
        let mut args = vec![
            "--die-with-parent",
            "--new-session",
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--cap-drop",
            "ALL",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/home/project",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        // A minimal filesystem excludes host homes, agent state, credentials and
        // tuned model paths. Explicit toolchain mounts are read-only capabilities.
        for path in [
            "/usr",
            "/bin",
            "/lib",
            "/lib64",
            "/etc/ssl",
            "/etc/fonts",
            "/etc/resolv.conf",
            "/etc/hosts",
        ] {
            if Path::new(path).exists() {
                args.extend(["--ro-bind".into(), path.into(), path.into()]);
            }
        }
        for path in &self.toolchains.mounts {
            if !path.is_absolute()
                || !path.is_dir()
                || path == Path::new("/")
                || path == Path::new("/home")
                || path == Path::new("/home/alex")
            {
                return Err(DomainError::PermissionDenied);
            }
            args.extend([
                "--ro-bind".into(),
                path.display().to_string(),
                path.display().to_string(),
            ]);
        }
        if let Some(display) = &self.display {
            let socket = format!("/tmp/.X11-unix/X{}", display.number);
            args.extend([
                "--ro-bind".into(),
                socket.clone(),
                socket,
                "--ro-bind".into(),
                display.auth.display().to_string(),
                "/tmp/ag1-xauthority".into(),
            ]);
        }
        args.extend([
            "--bind-fd".into(),
            PINNED_CWD_FD.into(),
            root.into(),
            "--chdir".into(),
            self.workspace
                .root()
                .join(&command.cwd)
                .display()
                .to_string(),
            "--".into(),
            command.executable.display().to_string(),
        ]);
        args.extend(command.argv.clone());
        let path = self
            .toolchains
            .node
            .iter()
            .chain(self.toolchains.cargo.iter())
            .filter_map(|p| p.parent())
            .map(|p| p.display().to_string())
            .chain(["/usr/bin".into(), "/bin".into()])
            .collect::<Vec<_>>()
            .join(":");
        let mut env = BTreeMap::from([
            ("HOME".into(), "/home/project".into()),
            ("PATH".into(), path),
            ("LC_ALL".into(), "C.UTF-8".into()),
            (
                "CARGO_HOME".into(),
                self.workspace
                    .root()
                    .join("target/ag1-cargo")
                    .display()
                    .to_string(),
            ),
            (
                "CARGO_TARGET_DIR".into(),
                self.workspace.root().join("target").display().to_string(),
            ),
        ]);
        if let Some(display) = &self.display {
            env.extend([
                ("DISPLAY".into(), format!(":{}", display.number)),
                ("XAUTHORITY".into(), "/tmp/ag1-xauthority".into()),
                ("GDK_BACKEND".into(), "x11".into()),
                ("WEBKIT_DISABLE_DMABUF_RENDERER".into(), "1".into()),
                ("LIBGL_ALWAYS_SOFTWARE".into(), "1".into()),
            ]);
        }
        if let Some(rustc) = &self.toolchains.rustc {
            env.insert("RUSTC".into(), rustc.display().to_string());
        }
        pinned.verify_identity()?;
        self.supervisor.spawn_project_with_cwd(
            ProcessSpec {
                executable: "/usr/bin/bwrap".into(),
                argv: args,
                cwd: pinned.process_cwd(),
                env,
                timeout_ms,
                grace_ms: 100,
                resident: false,
            },
            log,
        )
    }
    pub fn start(
        &mut self,
        target: &str,
        action: &str,
        launch: Launch,
        control: &ScanControl<'_>,
    ) -> Result<String, DomainError> {
        control.check()?;
        if self.running.is_some()
            || launch.commands.is_empty()
            || launch.commands.len() > 4
            || !(1..=86_400_000).contains(&launch.timeout_ms)
        {
            return Err(DomainError::Conflict);
        }
        if !self.project.targets.iter().any(|t| t.id == target) {
            return Err(DomainError::NotFound);
        }
        if let Some(ready) = &launch.readiness {
            if !ready.address.ip().is_loopback()
                || ready.expected_body.is_empty()
                || ready.expected_body.len() > 8192
                || !ready.path.starts_with('/')
                || ready.path.contains(['\r', '\n', ' '])
            {
                return Err(invalid("loopback readiness identity required"));
            }
            // Occupancy is only a rejection. It never creates a process handle.
            let reservation =
                std::net::TcpListener::bind(ready.address).map_err(|_| DomainError::Conflict)?;
            drop(reservation);
        }
        self.serial += 1;
        let id = format!("operation-{}", self.serial);
        let record = Operation {
            id: id.clone(),
            target: target.into(),
            action: action.into(),
            commands: launch
                .commands
                .iter()
                .map(|c| {
                    std::iter::once(c.executable.display().to_string())
                        .chain(c.argv.clone())
                        .collect()
                })
                .collect(),
            source_revision: self.revision()?,
            lifecycle: Lifecycle::Starting,
            exits: vec![None; launch.commands.len()],
            logs: (0..launch.commands.len())
                .flat_map(|n| {
                    [
                        format!("{id}-{n}-stdout.log"),
                        format!("{id}-{n}-stderr.log"),
                    ]
                })
                .collect(),
            diagnostics: vec![],
            elapsed_ms: 0,
            ready_ms: None,
            first_log_ms: None,
            stop_ms: None,
            target_rss_bytes: 0,
            browser_rss_bytes: 0,
            display_rss_bytes: 0,
        };
        self.save(&record)?; // Durable intent before any external effect.
        let mut running = Running {
            children: vec![],
            stdout: vec![],
            stdout_bytes: vec![],
            started: Instant::now(),
            launch: launch.clone(),
            record,
        };
        for (n, command) in launch.commands.iter().enumerate() {
            if action != "run" && n > 0 {
                break;
            }
            use std::os::unix::fs::OpenOptionsExt;
            running.stdout.push(
                fs::OpenOptions::new()
                    .mode(0o600)
                    .write(true)
                    .create_new(true)
                    .open(self.state.root().join(&running.record.logs[n * 2]))
                    .map_err(io_error)?,
            );
            running.stdout_bytes.push(0);
            match self.spawn(
                command,
                &self.state.root().join(&running.record.logs[n * 2 + 1]),
                launch.timeout_ms,
            ) {
                Ok(child) => running.children.push(child),
                Err(error) => {
                    for child in &mut running.children {
                        child.stop()?;
                    }
                    running.record.lifecycle = Lifecycle::Failed;
                    self.save(&running.record)?;
                    return Err(error);
                }
            }
        }
        self.running = Some(running);
        Ok(id)
    }
    pub fn poll(&mut self, control: &ScanControl<'_>) -> Result<Operation, DomainError> {
        let outcome = self.poll_inner(control);
        if outcome.is_err() && self.running.is_some() {
            if let Some(r) = &mut self.running {
                r.record.lifecycle = Lifecycle::Failed;
            }
            self.stop()?;
        }
        outcome
    }
    fn poll_inner(&mut self, control: &ScanControl<'_>) -> Result<Operation, DomainError> {
        if let Err(error) = control.check().and_then(|_| self.workspace.verify_root()) {
            self.stop()?;
            return Err(error);
        }
        let running = self.running.as_mut().ok_or(DomainError::NotFound)?;
        let mut all_exited = true;
        let mut failed = false;
        let mut primary_exited = false;
        let tauri_run = running.record.action == "run"
            && self
                .project
                .targets
                .iter()
                .any(|t| t.id == running.record.target && t.kind == Kind::Tauri);
        let count = running.launch.commands.len();
        for (n, child) in running.children.iter_mut().enumerate() {
            // Drain the separate stdout into the complete private log before projection.
            for _ in 0..32 {
                let bytes = child.read_stdout(32768)?;
                if bytes.is_empty() {
                    break;
                }
                running.stdout[n].write_all(&bytes).map_err(io_error)?;
                running.stdout_bytes[n] += bytes.len();
                if running.stdout_bytes[n] > 8 * 1024 * 1024 {
                    return Err(DomainError::LimitExceeded(
                        "project stdout retained; stop required".into(),
                    ));
                }
            }
            running.stdout[n].sync_data().map_err(io_error)?;
            if (child.tail(0)?.total_seen > 0 || running.stdout_bytes[n] > 0)
                && running.record.first_log_ms.is_none()
            {
                running.record.first_log_ms = Some(running.started.elapsed().as_millis() as u64);
            }
            if child.tail(0)?.truncated {
                return Err(DomainError::LimitExceeded(
                    "project stderr quota exceeded; prefix retained".into(),
                ));
            }
            match child.try_wait()? {
                Some(status) => {
                    running.record.exits[n] = status.code();
                    primary_exited |= tauri_run && n + 1 == count;
                    failed |= !status.success();
                }
                None => all_exited = false,
            }
        }
        running.record.elapsed_ms = running.started.elapsed().as_millis() as u64;
        running.record.display_rss_bytes = running.record.display_rss_bytes.max(
            self.display
                .as_ref()
                .map(|d| owned_rss(d.process.identity().pid))
                .unwrap_or(0),
        );
        running.record.target_rss_bytes = running.record.target_rss_bytes.max(
            running
                .children
                .iter()
                .map(|c| owned_rss(c.identity().pid))
                .sum(),
        );
        let next = if all_exited && !failed && running.record.action != "run" {
            running.launch.commands.get(running.children.len()).cloned()
        } else {
            None
        };
        if failed {
            running.record.lifecycle = Lifecycle::Failed;
        } else if (all_exited && next.is_none()) || primary_exited {
            running.record.lifecycle = Lifecycle::Exited;
        } else if running.launch.readiness.as_ref().is_some_and(|ready| {
            http_ready(ready)
                && running
                    .children
                    .iter()
                    .any(|c| owns_listener(c.identity().pid, ready.address.port()))
        }) {
            running.record.lifecycle = Lifecycle::Ready;
            running
                .record
                .ready_ms
                .get_or_insert(running.record.elapsed_ms);
        } else {
            running.record.lifecycle = Lifecycle::Starting;
        }
        // Without an explicit runtime handshake, a live PID means Starting only.
        let record = running.record.clone();
        self.save(&record)?;
        if let Some(command) = next {
            let r = self.running.as_ref().unwrap();
            let n = r.children.len();
            let stdout_path = self.state.root().join(&r.record.logs[n * 2]);
            let stderr_path = self.state.root().join(&r.record.logs[n * 2 + 1]);
            let timeout = r.launch.timeout_ms;
            use std::os::unix::fs::OpenOptionsExt;
            let stdout = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(stdout_path)
                .map_err(io_error)?;
            let child = self.spawn(&command, &stderr_path, timeout)?;
            let r = self.running.as_mut().unwrap();
            r.stdout.push(stdout);
            r.stdout_bytes.push(0);
            r.children.push(child);
        }
        if failed || primary_exited {
            self.stop_children()?;
        }
        Ok(record)
    }
    fn stop_children(&mut self) -> Result<(), DomainError> {
        if let Some(r) = &mut self.running {
            for (n, c) in r.children.iter_mut().enumerate() {
                c.stop()?;
                loop {
                    let bytes = c.read_stdout(32768)?;
                    if bytes.is_empty() {
                        break;
                    }
                    r.stdout[n].write_all(&bytes).map_err(io_error)?;
                }
                r.stdout[n].sync_all().map_err(io_error)?;
            }
        }
        Ok(())
    }
    pub fn finish(&mut self, control: &ScanControl<'_>) -> Result<Operation, DomainError> {
        loop {
            let record = self.poll(control)?;
            if matches!(record.lifecycle, Lifecycle::Exited | Lifecycle::Failed) {
                self.stop_children()?;
                let mut running = self.running.take().unwrap();
                self.collect_diagnostics(&mut running.record)?;
                self.save(&running.record)?;
                return Ok(running.record);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub fn stop(&mut self) -> Result<Option<Operation>, DomainError> {
        let start = Instant::now();
        if self.running.is_none() {
            return Ok(None);
        }
        if let Err(error) = self.stop_children() {
            let mut record = self.running.as_ref().unwrap().record.clone();
            record.lifecycle = Lifecycle::RecoveryRequired;
            self.save(&record)?;
            return Err(error);
        }
        let mut running = self.running.take().unwrap();
        running.record.stop_ms = Some(start.elapsed().as_millis() as u64);
        running.record.elapsed_ms = running.started.elapsed().as_millis() as u64;
        if !matches!(
            running.record.lifecycle,
            Lifecycle::Failed | Lifecycle::Exited
        ) {
            running.record.lifecycle = Lifecycle::Stopped;
        }
        self.collect_diagnostics(&mut running.record)?;
        self.save(&running.record)?;
        Ok(Some(running.record))
    }
    fn collect_diagnostics(&self, record: &mut Operation) -> Result<(), DomainError> {
        for (n, log) in record.logs.iter().enumerate() {
            if !self.state.exists(log)? {
                continue;
            }
            let mut bytes = vec![];
            self.state.stream(log, 64 * 1024 * 1024, |chunk| {
                let keep = (1024 * 1024usize)
                    .saturating_sub(bytes.len())
                    .min(chunk.len());
                bytes.extend_from_slice(&chunk[..keep]);
                Ok(())
            })?;
            record
                .diagnostics
                .extend(parse_diagnostics(&bytes, &format!("process-{n}")));
            if record
                .exits
                .get(n / 2)
                .is_some_and(|v| v.is_some_and(|c| c != 0))
            {
                record.diagnostics.push(Diagnostic {
                    source: format!("process-{n}"),
                    severity: "error".into(),
                    message: format!(
                        "Process exited {:?}; full log retained",
                        record.exits[n / 2]
                    ),
                    file: None,
                    line: None,
                    column: None,
                    code: None,
                });
            }
        }
        Ok(())
    }
    pub fn log(
        &self,
        operation: &str,
        stream: usize,
        offset: u64,
        limit: usize,
    ) -> Result<Vec<u8>, DomainError> {
        if limit > 32768 {
            return Err(DomainError::LimitExceeded("project log page".into()));
        }
        let record = self
            .history()?
            .into_iter()
            .find(|r| r.id == operation)
            .ok_or(DomainError::NotFound)?;
        let path = record.logs.get(stream).ok_or(DomainError::NotFound)?;
        let mut result = vec![];
        let mut position = 0u64;
        self.state.stream(path, 64 * 1024 * 1024, |bytes| {
            let begin = offset.saturating_sub(position).min(bytes.len() as u64) as usize;
            let remaining = limit.saturating_sub(result.len());
            let end = (begin + remaining).min(bytes.len());
            result.extend_from_slice(&bytes[begin..end]);
            position += bytes.len() as u64;
            Ok(())
        })?;
        Ok(result)
    }
}
fn http_ready(ready: &ReadyHttp) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&ready.address, Duration::from_millis(30))
    else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(50)));
    if write!(
        stream,
        "GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        ready.path, ready.address
    )
    .is_err()
    {
        return false;
    }
    let mut bytes = vec![];
    if stream.take(16384).read_to_end(&mut bytes).is_err() {
        return false;
    }
    let text = String::from_utf8_lossy(&bytes);
    (text.starts_with("HTTP/1.1 200") || text.starts_with("HTTP/1.0 200"))
        && text
            .split_once("\r\n\r\n")
            .is_some_and(|(_, body)| body == ready.expected_body)
}
pub fn parse_diagnostics(bytes: &[u8], source: &str) -> Vec<Diagnostic> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            // HTTP access timestamps contain colons but are not compiler
            // locations. Keep these in the raw runtime log, not Problems.
            static ACCESS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(r#"^\S+ - \S+ \[[^]]+\] "[A-Z]+ [^"]+ HTTP/[0-9.]+" [0-9]{3} "#)
                    .unwrap()
            });
            if ACCESS.is_match(line) {
                return None;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && value["reason"] == "compiler-message"
            {
                let m = &value["message"];
                let span = m["spans"]
                    .as_array()
                    .and_then(|s| s.iter().find(|v| v["is_primary"] == true));
                return Some(Diagnostic {
                    source: source.into(),
                    severity: m["level"].as_str().unwrap_or("error").into(),
                    message: m["message"]
                        .as_str()
                        .unwrap_or("Compiler diagnostic")
                        .chars()
                        .take(2048)
                        .collect(),
                    file: span
                        .and_then(|s| s["file_name"].as_str())
                        .map(str::to_string),
                    line: span.and_then(|s| s["line_start"].as_u64()),
                    column: span.and_then(|s| s["column_start"].as_u64()),
                    code: m["code"]["code"].as_str().map(str::to_string),
                });
            }
            if serde_json::from_str::<Value>(line).is_ok()
                || line.trim_start().starts_with("Compiling ")
                || line.trim_start().starts_with("Checking ")
            {
                return None;
            }
            if let Some(diagnostic) = text_diagnostic(line, source) {
                return Some(diagnostic);
            }
            let lower = line.to_lowercase();
            if ["error", "panic", "failed", "exception", "segmentation"]
                .iter()
                .any(|s| lower.contains(s))
            {
                Some(Diagnostic {
                    source: source.into(),
                    severity: "error".into(),
                    message: line.chars().take(2048).collect(),
                    file: None,
                    line: None,
                    column: None,
                    code: None,
                })
            } else {
                None
            }
        })
        .take(128)
        .collect()
}
/// Linux process-tree RSS includes descendants; it never grants stop authority.
pub(super) fn owned_rss(pid: u32) -> u64 {
    let own = fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|n| n.parse::<u64>().ok())
        })
        .unwrap_or(0)
        * 1024;
    let children =
        fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap_or_default();
    own + children
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .map(owned_rss)
        .sum::<u64>()
}

fn owns_listener(pid: u32, port: u16) -> bool {
    let inodes = fs::read_to_string("/proc/net/tcp")
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let c = l.split_whitespace().collect::<Vec<_>>();
            if c.len() > 9
                && c[3] == "0A"
                && c[1]
                    .split_once(':')
                    .is_some_and(|(_, p)| u16::from_str_radix(p, 16) == Ok(port))
            {
                Some(format!("socket:[{}]", c[9]))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    fn has(pid: u32, inodes: &[String], depth: usize) -> bool {
        if depth > 32 {
            return false;
        }
        if fs::read_dir(format!("/proc/{pid}/fd")).is_ok_and(|d| {
            d.flatten().any(|e| {
                fs::read_link(e.path()).is_ok_and(|p| inodes.iter().any(|i| p == Path::new(i)))
            })
        }) {
            return true;
        }
        fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
            .unwrap_or_default()
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .any(|p| has(p, inodes, depth + 1))
    }
    has(pid, &inodes, 0)
}

impl Runtime {
    pub fn preview_grant(&self) -> Result<super::preview::Grant, DomainError> {
        self.workspace.verify_root()?;
        let running = self.running.as_ref().ok_or(DomainError::NotFound)?;
        let ready = running.launch.readiness.as_ref().ok_or_else(|| {
            DomainError::Unavailable(
                "Native target has no web preview; use logs and runtime observations".into(),
            )
        })?;
        if running.record.lifecycle != Lifecycle::Ready
            || !running
                .children
                .iter()
                .any(|c| owns_listener(c.identity().pid, ready.address.port()))
            || !http_ready(ready)
        {
            return Err(DomainError::Conflict);
        }
        super::preview::Grant::new(
            &running.record.id,
            &self.project.session,
            &format!("http://{}/", ready.address),
        )
    }
    pub fn verify_preview(&self, grant: &super::preview::Grant) -> Result<(), DomainError> {
        let current = self.preview_grant()?;
        if !grant.open
            || current.operation != grant.operation
            || current.session != grant.session
            || current.origin != grant.origin
        {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}

impl Runtime {
    pub fn open_private_display(
        &mut self,
        xvfb: &Path,
        control: &ScanControl<'_>,
    ) -> Result<(), DomainError> {
        if self.display.is_some() || self.running.is_some() {
            return Err(DomainError::Conflict);
        }
        self.display = Some(super::display::NativeDisplay::start(
            &self.supervisor,
            xvfb,
            self.state.root(),
            control,
        )?);
        Ok(())
    }
    pub fn close_private_display(&mut self) -> Result<(), DomainError> {
        self.stop()?;
        if let Some(mut display) = self.display.take() {
            display.stop()?;
        }
        Ok(())
    }
}

impl Runtime {
    pub fn observed_pids(&self) -> Vec<u32> {
        fn tree(pid: u32, out: &mut Vec<u32>) {
            if out.len() > 4096 {
                return;
            }
            out.push(pid);
            for p in fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))
                .unwrap_or_default()
                .split_whitespace()
                .filter_map(|p| p.parse().ok())
            {
                tree(p, out);
            }
        }
        let mut pids = vec![];
        if let Some(r) = &self.running {
            for c in &r.children {
                tree(c.identity().pid, &mut pids);
            }
        }
        pids
    }
    pub fn restore_version(
        &mut self,
        git: &crate::git::GitService,
        preview: &crate::git::VersionPreview,
    ) -> Result<(), DomainError> {
        if self.running.is_some() {
            return Err(DomainError::Conflict);
        }
        self.serial += 1;
        let name = format!("git-recovery-{}.json", self.serial);
        use base64::Engine;
        let mut recovery = json!({"project":self.project.id,"session":self.project.session,"path":preview.path,"expected_head":preview.head,"source_commit":preview.source,"before_base64":base64::engine::general_purpose::STANDARD.encode(&preview.before),"candidate_base64":base64::engine::general_purpose::STANDARD.encode(&preview.candidate),"state":"intent","session_rollback":false});
        let revision = self
            .state
            .create_checked(&name, &document(&recovery)?, false, || Ok(()))?;
        let result = git.apply_version(&self.workspace, preview);
        recovery["state"] = json!(match &result {
            Ok(_) => "applied",
            Err(DomainError::DurabilityUnknown) => "recovery_required",
            Err(_) => "rejected",
        });
        self.state
            .replace(&name, &revision, &document(&recovery)?)?;
        result.map(|_| ())
    }
}

impl Runtime {
    /// Host composition only, for a trusted preview shell on this private display.
    pub fn private_display_environment(&self) -> Result<BTreeMap<String, String>, DomainError> {
        let d = self.display.as_ref().ok_or(DomainError::NotFound)?;
        Ok(BTreeMap::from([
            ("DISPLAY".into(), format!(":{}", d.number)),
            ("XAUTHORITY".into(), d.auth.display().to_string()),
            ("GDK_BACKEND".into(), "x11".into()),
            ("WEBKIT_DISABLE_DMABUF_RENDERER".into(), "1".into()),
            ("LIBGL_ALWAYS_SOFTWARE".into(), "1".into()),
        ]))
    }
}

fn text_diagnostic(line: &str, source: &str) -> Option<Diagnostic> {
    use std::sync::LazyLock;
    static LOCATION: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^(.+?)(?:\((\d+),(\d+)\)|:(\d+)(?::(\d+))?):\s*(.*)$").unwrap()
    });
    static LEVEL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"^(error|warning|note)(?:\[([^]]+)\]|\s+([A-Z]+[0-9]+))?:\s*(.*)$")
            .unwrap()
    });
    static ANSI: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap());
    let cleaned = ANSI.replace_all(line, "");
    let c = LOCATION.captures(&cleaned)?;
    let body = c.get(6)?.as_str();
    let detail = LEVEL.captures(body);
    Some(Diagnostic {
        source: source.into(),
        severity: detail
            .as_ref()
            .map(|d| d[1].to_string())
            .unwrap_or("error".into()),
        message: detail
            .as_ref()
            .map(|d| d[4].to_string())
            .unwrap_or_else(|| body.to_string())
            .chars()
            .take(2048)
            .collect(),
        file: Some(c[1].into()),
        line: c
            .get(2)
            .or_else(|| c.get(4))
            .and_then(|n| n.as_str().parse().ok()),
        column: c
            .get(3)
            .or_else(|| c.get(5))
            .and_then(|n| n.as_str().parse().ok()),
        code: detail
            .as_ref()
            .and_then(|d| d.get(2).or_else(|| d.get(3)))
            .map(|v| v.as_str().into()),
    })
}
