//! Contained local Git plumbing. No shell strings, hooks, filters or signing.
//! Exact network fetch/push uses separately bound account admission. Native
//! index/reference publication is separate from object acquisition.
use crate::{
    io_error,
    process::{PINNED_CWD_FD, PINNED_DIRECTORY_PREFIX, ProcessSpec, ProcessSupervisor},
    resource_fs::ScanControl,
    workspace::{Workspace, WorkspaceService},
};
use ag1_core::{DomainError, FileRevision};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, DomainError>;
#[path = "local_clone.rs"]
mod clone;
#[path = "local_metadata.rs"]
mod metadata;
#[path = "local_network.rs"]
mod network;
#[path = "local_storage.rs"]
mod storage;
#[path = "local_transition.rs"]
mod transition;
pub struct LocalGit {
    storage_active: std::sync::atomic::AtomicU8,
    storage_polled: std::sync::Mutex<Instant>,
    #[cfg(test)]
    storage_limits: Option<storage::Limits>,
    metadata: std::sync::Mutex<Option<Arc<metadata::Binding>>>,
    workspace: Arc<Workspace>,
    supervisor: ProcessSupervisor,
    git: PathBuf,
    bwrap: PathBuf,
    state: PathBuf,
    grant: Option<(Arc<crate::folder_access::FolderAccess>, u64, bool)>,
    github: Option<Arc<super::service::Service>>,
    #[cfg(test)]
    network_fixture: Option<(String, Vec<u8>)>,
    #[cfg(test)]
    transition_hook: Option<Arc<dyn Fn(&str, &str) + Send + Sync>>,
}
pub struct LocalPlan {
    pub(crate) action: String,
    pub(crate) args: Value,
    scope: String,
    head: String,
    branch: String,
    index: Option<(Vec<u8>, FileRevision)>,
    sources: Vec<(String, Option<crate::workspace::GitSnapshot>)>,
    config_revision: Option<FileRevision>,
    network: Option<super::service::GitAdmission>,
    clone_target: Option<clone::CloneTarget>,
    transition: Option<transition::Transition>,
    revision: String,
    expires: Instant,
    pub(crate) preview: Value,
}
impl LocalPlan {
    pub fn preview(&self) -> Value {
        let mut value = self.preview.clone();
        for key in ["status", "staged_diff"] {
            if let Some(text) = value[key].as_str() {
                if text.len() > 12000 {
                    let mut end = 12000;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    value[key] = Value::String(format!(
                        "{}\n[Preview clipped; use Changes for full paged review]",
                        &text[..end]
                    ));
                }
            }
        }
        value
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn may_mutate(&self) -> bool {
        !["status", "diff", "branches", "recovery_status"].contains(&self.action.as_str())
    }
}
impl LocalGit {
    pub fn new(
        workspace: Arc<Workspace>,
        helper: &Path,
        git: &Path,
        bwrap: &Path,
        state: &Path,
    ) -> Result<Self> {
        let git = git.canonicalize().map_err(io_error)?;
        let bwrap = bwrap.canonicalize().map_err(io_error)?;
        if !git.starts_with("/usr/") || !bwrap.is_file() {
            return Err(DomainError::Unavailable(
                "Local Git requires the configured system Git and bubblewrap".into(),
            ));
        }
        std::fs::create_dir_all(state).map_err(io_error)?;
        Ok(Self {
            storage_active: std::sync::atomic::AtomicU8::new(0),
            storage_polled: std::sync::Mutex::new(Instant::now()),
            #[cfg(test)]
            storage_limits: None,
            metadata: std::sync::Mutex::new(None),
            workspace,
            supervisor: ProcessSupervisor::new(helper)?,
            git,
            bwrap,
            state: state.canonicalize().map_err(io_error)?,
            grant: None,
            github: None,
            #[cfg(test)]
            network_fixture: None,
            #[cfg(test)]
            transition_hook: None,
        })
    }
    pub fn with_grant(
        mut self,
        authority: Arc<crate::folder_access::FolderAccess>,
        revision: u64,
        writable: bool,
    ) -> Self {
        self.grant = Some((authority, revision, writable));
        self
    }
    pub fn with_github(mut self, service: Arc<super::service::Service>) -> Self {
        self.github = Some(service);
        self
    }
    pub fn root(&self) -> &Path {
        self.workspace.root()
    }
    fn authority(&self, write: bool) -> Result<()> {
        if let Some((authority, revision, writable)) = &self.grant {
            authority.check(*revision)?;
            if write && !writable {
                return Err(DomainError::PermissionDenied);
            }
        }
        if let Some(binding) = self
            .metadata
            .lock()
            .map_err(|_| DomainError::Conflict)?
            .as_ref()
        {
            binding.verify(self, write)?;
        }
        Ok(())
    }
    fn layout(&self) -> Result<()> {
        self.authority(false)?;
        self.metadata()?.verify(self, false)
    }

    fn command(
        &self,
        args: &[String],
        input: Option<&[u8]>,
        private: Option<&Workspace>,
        env: BTreeMap<String, String>,
        control: &ScanControl<'_>,
    ) -> Result<(Vec<u8>, bool)> {
        self.command_guarded(args, input, private, env, control, None, false)
    }
    fn command_guarded(
        &self,
        args: &[String],
        input: Option<&[u8]>,
        private: Option<&Workspace>,
        env: BTreeMap<String, String>,
        control: &ScanControl<'_>,
        mut before_commit: Option<&dyn Fn(&crate::process::ProcessIdentity) -> Result<()>>,
        owned_clone: bool,
    ) -> Result<(Vec<u8>, bool)> {
        control.check()?;
        self.layout()?;
        self.storage_check(control)?;
        let pinned = self.workspace.pin_directory("")?;
        let binding = self.metadata()?;
        let metadata_pin = binding.git.pin_directory("")?;
        let common_pin = binding.common.pin_directory("")?;
        // Repository attributes may otherwise launch clean/process filters even
        // during status. Overlay only info/ with native highest-priority rules;
        // never edit the user's attributes or run their configured programs.
        let info_pin = binding.common.pin_directory("info")?;
        let object_info_pin = binding.common.pin_directory("objects/info")?;
        let info_temp = tempfile::Builder::new()
            .prefix("git-info-")
            .tempdir_in(&self.state)
            .map_err(io_error)?;
        std::fs::write(
            info_temp.path().join("attributes"),
            b"* -filter -text -ident -working-tree-encoding\n",
        )
        .map_err(io_error)?;
        for name in ["exclude", "sparse-checkout"] {
            match binding
                .common
                .snapshot(&format!("info/{name}"), 1024 * 1024)
            {
                Ok(snapshot) => {
                    std::fs::write(info_temp.path().join(name), snapshot.bytes).map_err(io_error)?
                }
                Err(DomainError::NotFound) => {}
                Err(error) => return Err(error),
            }
        }
        let info_workspace = WorkspaceService::new()?.register(info_temp.path())?;
        let overlay_pin = info_workspace.pin_directory("")?;
        let private_pin = private.map(|w| w.pin_directory("")).transpose()?;
        let mut argv = vec![
            "--die-with-parent",
            "--unshare-user",
            "--unshare-pid",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-net",
            "--ro-bind",
            "/usr",
            "/usr",
            "--symlink",
            "usr/bin",
            "/bin",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/tmp/ag1-home",
            if owned_clone {
                "--bind-fd"
            } else {
                "--ro-bind-fd"
            },
            PINNED_CWD_FD,
            "/workspace",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        for (pin, target) in [
            (&common_pin, "/gitcommon"),
            (&metadata_pin, binding.namespace_git_dir.as_str()),
        ] {
            argv.extend([
                "--bind-fd".into(),
                format!("{PINNED_DIRECTORY_PREFIX}{}", pin.process_cwd().display()),
                target.into(),
            ]);
        }
        argv.extend([
            "--ro-bind-fd".into(),
            format!(
                "{PINNED_DIRECTORY_PREFIX}{}",
                overlay_pin.process_cwd().display()
            ),
            "/gitcommon/info".into(),
        ]);
        argv.extend([
            "--ro-bind-fd".into(),
            format!(
                "{PINNED_DIRECTORY_PREFIX}{}",
                overlay_pin.process_cwd().display()
            ),
            "/gitcommon/objects/info".into(),
        ]);
        for lib in ["/lib", "/lib64"] {
            if Path::new(lib).exists() {
                argv.extend(["--ro-bind".into(), lib.into(), lib.into()]);
            }
        }
        if let Some(pin) = &private_pin {
            argv.extend([
                "--bind-fd".into(),
                format!("{PINNED_DIRECTORY_PREFIX}{}", pin.process_cwd().display()),
                "/git-private".into(),
            ]);
        }
        argv.extend(
            [
                "--chdir",
                "/workspace",
                "--",
                self.git.to_str().ok_or(DomainError::PermissionDenied)?,
                "--no-pager",
                "--literal-pathspecs",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
                "-c",
                "core.attributesFile=/dev/null",
                "-c",
                "diff.ignoreSubmodules=all",
                "-c",
                "status.submoduleSummary=false",
            ]
            .into_iter()
            .map(str::to_string),
        );
        argv.extend_from_slice(args);
        let mut environment = BTreeMap::from([
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("HOME".into(), "/tmp/ag1-home".into()),
            ("GIT_DIR".into(), binding.namespace_git_dir.clone()),
            ("GIT_COMMON_DIR".into(), "/gitcommon".into()),
            ("GIT_WORK_TREE".into(), "/workspace".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
            ("LC_ALL".into(), "C".into()),
        ]);
        environment.extend(env);
        let spool = self
            .state
            .join(format!("git-{}.log", crate::client::nonce()?));
        let mut child = self.supervisor.spawn_project_with_cwd(
            ProcessSpec {
                executable: self.bwrap.clone(),
                argv,
                cwd: pinned.process_cwd(),
                env: environment,
                timeout_ms: control
                    .deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .clamp(1, 300000) as u64,
                grace_ms: 100,
                resident: false,
            },
            &spool,
        )?;
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(spool);
        if let Some(bytes) = input {
            let mut offset = 0;
            while offset < bytes.len() {
                control.check()?;
                match child.try_write_input(&bytes[offset..bytes.len().min(offset + 32768)])? {
                    Some(0) => return Err(DomainError::Unavailable("Git closed its input".into())),
                    Some(count) => offset += count,
                    None => std::thread::sleep(Duration::from_millis(2)),
                }
            }
        }
        if before_commit.is_none() {
            child.close_input();
        }
        let mut bytes = Vec::new();
        let mut storage_polled = Instant::now();
        let success = loop {
            let admission = control.check().and_then(|_| {
                if storage_polled.elapsed() >= Duration::from_millis(100) {
                    self.storage_check(control)?;
                    storage_polled = Instant::now();
                }
                Ok(())
            });
            if let Err(error) = admission {
                if before_commit.is_some() && !abort_reference_transaction(&mut child) {
                    return Err(DomainError::DurabilityUnknown);
                }
                return Err(error);
            }
            let (part, done) = child.poll_stdout(32768)?;
            bytes.extend(part);
            let output_limit = if args.first().is_some_and(|a| a == "cat-file")
                && args.get(1).is_some_and(|a| a == "blob")
            {
                8 * 1024 * 1024
            } else {
                2 * 1024 * 1024
            };
            if bytes.len() > output_limit {
                return Err(DomainError::LimitExceeded("local Git output".into()));
            }
            if before_commit.is_some() && bytes.ends_with(b"prepare: ok\n") {
                if let Err(error) = before_commit.take().unwrap()(child.identity()) {
                    // Release Git's own prepared locks before terminating the
                    // owned child. Cleanup must still run after cancellation;
                    // it never deletes rediscovered lock paths itself.
                    return Err(if abort_reference_transaction(&mut child) {
                        error
                    } else {
                        DomainError::DurabilityUnknown
                    });
                }
                let commit = b"commit\n";
                let mut offset = 0;
                while offset < commit.len() {
                    if let Err(error) = control.check() {
                        return Err(if abort_reference_transaction(&mut child) {
                            error
                        } else {
                            DomainError::DurabilityUnknown
                        });
                    }
                    match child.try_write_input(&commit[offset..])? {
                        Some(0) => return Err(DomainError::DurabilityUnknown),
                        Some(count) => offset += count,
                        None => std::thread::sleep(Duration::from_millis(2)),
                    }
                }
                child.close_input();
            }
            if let Some(status) = done {
                break status.success();
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        self.storage_check(control)?;
        pinned.verify_identity()?;
        metadata_pin.verify_identity()?;
        common_pin.verify_identity()?;
        binding.verify(self, false)?;
        info_pin.verify_identity()?;
        object_info_pin.verify_identity()?;
        overlay_pin.verify_identity()?;
        if let Some(pin) = private_pin {
            pin.verify_identity()?;
        }
        control.check()?;
        Ok((bytes, success))
    }
    // Human-readable diffs can contain arbitrary Git blob bytes (including
    // link targets). Never apply this display conversion to OIDs or paths used
    // as native authority; run() below remains strict for plumbing responses.
    fn display(&self, args: &[&str], control: &ScanControl<'_>) -> Result<String> {
        let (bytes, ok) = self.command(
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            None,
            None,
            BTreeMap::new(),
            control,
        )?;
        if !ok {
            return Err(DomainError::InvalidInput(
                "Git could not produce the bounded display".into(),
            ));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    fn run(&self, args: &[&str], control: &ScanControl<'_>) -> Result<String> {
        let (bytes, ok) = self.command(
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            None,
            None,
            BTreeMap::new(),
            control,
        )?;
        if !ok {
            return Err(DomainError::InvalidInput(
                "Local Git rejected the command; inspect repository state and grants".into(),
            ));
        }
        String::from_utf8(bytes)
            .map_err(|_| DomainError::InvalidInput("UTF-8 local Git response".into()))
    }
    fn index(&self) -> Result<Option<(Vec<u8>, FileRevision)>> {
        match self.metadata_snapshot(".git/index", 8 * 1024 * 1024) {
            Ok(snapshot) => Ok(Some((snapshot.bytes, snapshot.revision))),
            Err(DomainError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn state(&self, control: &ScanControl<'_>) -> Result<(String, String)> {
        let branch = self
            .run(&["symbolic-ref", "--quiet", "HEAD"], control)?
            .trim()
            .to_string();
        if !branch.starts_with("refs/heads/")
            || branch.len() > 512
            || branch.contains(['\r', '\n', '\0'])
        {
            return Err(DomainError::InvalidInput(
                "A local branch is required".into(),
            ));
        }
        let (bytes, ok) = self.command(
            &["rev-parse".into(), "--verify".into(), "HEAD".into()],
            None,
            None,
            BTreeMap::new(),
            control,
        )?;
        let head = if ok {
            String::from_utf8(bytes)
                .map_err(|_| DomainError::Conflict)?
                .trim()
                .to_string()
        } else {
            String::new()
        };
        if !head.is_empty() && !oid(&head) {
            return Err(DomainError::Conflict);
        }
        Ok((head, branch))
    }
    pub fn prepare(
        &self,
        action: &str,
        args: &Value,
        scope: &str,
        control: &ScanControl<'_>,
    ) -> Result<LocalPlan> {
        let mut normalized = args.clone();
        if action == "fetch_pr" {
            let number = args["pull_number"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= u32::MAX as u64)
                .ok_or_else(|| {
                    DomainError::InvalidInput("A positive pull_number is required".into())
                })?;
            if args.get("branch").is_some() {
                return Err(DomainError::InvalidInput(
                    "PR fetch derives its tracking branch from pull_number".into(),
                ));
            }
            normalized["branch"] = json!(format!("pr/{number}"));
        }
        let args = &normalized;
        if !args.is_object()
            || args.to_string().len() > 24000
            || args.as_object().unwrap().keys().any(|k| {
                ![
                    "action",
                    "repo",
                    "folder",
                    "paths",
                    "message",
                    "author_name",
                    "author_email",
                    "branch",
                    "expected_remote_sha",
                    "destination",
                    "target_sha",
                    "pull_number",
                    "recovery_id",
                    "choice",
                ]
                .contains(&k.as_str())
            })
        {
            return Err(DomainError::InvalidInput(
                "Bounded local Git parameters required".into(),
            ));
        }
        if ![
            "status",
            "diff",
            "branches",
            "stage",
            "unstage",
            "commit",
            "branch_create",
            "fetch",
            "push",
            "clone",
            "checkout",
            "fetch_pr",
            "pull",
            "recover",
            "recovery_status",
        ]
        .contains(&action)
        {
            return Err(DomainError::Unavailable(
                "Local Git operation is not implemented".into(),
            ));
        }
        control.check()?;
        if ["recover", "recovery_status"].contains(&action) {
            return self.prepare_recovery(action, args, scope, control);
        }
        if action == "clone" {
            return self.prepare_clone(args, scope, control);
        }
        self.layout()?;
        self.authority(!["status", "diff", "branches"].contains(&action))?;
        if !["status", "diff", "branches"].contains(&action) {
            self.transition_ready()?;
        }
        let repo = args["repo"]
            .as_str()
            .ok_or_else(|| DomainError::InvalidInput("Select a GitHub repository".into()))?;
        super::repo(&json!({"repo":repo})).map_err(DomainError::InvalidInput)?;
        let config_revision = Some(self.metadata_snapshot(".git/config", 1024 * 1024)?.revision);
        let remote = self.run(
            &["config", "--no-includes", "--get", "remote.origin.url"],
            control,
        )?;
        let remote = remote.trim().trim_end_matches(".git");
        if remote != format!("https://github.com/{repo}")
            && remote != format!("git@github.com:{repo}")
        {
            return Err(DomainError::InvalidInput(
                "The selected repository does not match this folder's origin".into(),
            ));
        }
        let (head, branch) = self.state(control)?;
        let index = self.index()?;
        let mut sources = Vec::new();
        let mut total = 0;
        if action == "stage" {
            let paths = args["paths"]
                .as_array()
                .filter(|p| !p.is_empty() && p.len() <= 64)
                .ok_or_else(|| DomainError::InvalidInput("Choose 1–64 exact paths".into()))?;
            let mut seen = std::collections::BTreeSet::new();
            for path in paths {
                let path = path.as_str().ok_or(DomainError::PermissionDenied)?;
                if !exact_path(path) || !seen.insert(path) {
                    return Err(DomainError::PermissionDenied);
                }
                let snapshot = self.workspace.git_snapshot(path)?;
                if snapshot.is_none() {
                    // An absent entry is an exact index deletion, not a
                    // recursive directory pathspec or a made-up missing file.
                    let tracked = self.run(&["ls-files", "--stage", "-z", "--", path], control)?;
                    let row = tracked.strip_suffix('\0').ok_or(DomainError::NotFound)?;
                    let (metadata, name) = row.split_once('\t').ok_or(DomainError::Conflict)?;
                    let columns = metadata.split_whitespace().collect::<Vec<_>>();
                    if name != path
                        || columns.len() != 3
                        || !["100644", "100755", "120000"].contains(&columns[0])
                        || !oid(columns[1])
                        || columns[2] != "0"
                    {
                        return Err(DomainError::InvalidInput("Select an exact tracked file or link for deletion staging; directories and unresolved entries are not expanded".into()));
                    }
                }
                let bytes = snapshot.as_ref().map_or(0, |s| s.entry.bytes.len());
                if bytes > 8 * 1024 * 1024 {
                    return Err(DomainError::LimitExceeded("staging entry bytes".into()));
                }
                total += bytes;
                if total > 16 * 1024 * 1024 {
                    return Err(DomainError::LimitExceeded("staging snapshot bytes".into()));
                }
                sources.push((path.to_string(), snapshot));
            }
        }
        if action == "unstage" {
            let paths = args["paths"]
                .as_array()
                .filter(|p| !p.is_empty() && p.len() <= 64)
                .ok_or_else(|| DomainError::InvalidInput("Choose exact paths to unstage".into()))?;
            for path in paths {
                let path = path.as_str().ok_or(DomainError::PermissionDenied)?;
                if path.is_empty()
                    || path.starts_with('/')
                    || path
                        .split('/')
                        .any(|p| p == ".." || p == ".git" || p.is_empty())
                    || path.contains('\0')
                {
                    return Err(DomainError::PermissionDenied);
                }
            }
        }
        if action == "commit" {
            for key in ["message", "author_name", "author_email"] {
                let text = args[key]
                    .as_str()
                    .filter(|s| {
                        !s.trim().is_empty()
                            && s.len() <= if key == "message" { 16000 } else { 200 }
                            && !s.contains('\0')
                    })
                    .ok_or_else(|| {
                        DomainError::InvalidInput(format!("Explicit {key} is required"))
                    })?;
                if key != "message" && text.contains(['\r', '\n', '<', '>']) {
                    return Err(DomainError::InvalidInput(
                        "Invalid Git author identity".into(),
                    ));
                }
            }
        }
        if [
            "branch_create",
            "fetch",
            "fetch_pr",
            "push",
            "checkout",
            "pull",
        ]
        .contains(&action)
        {
            let branch = args["branch"]
                .as_str()
                .ok_or(DomainError::PermissionDenied)?;
            self.run(
                &["check-ref-format", &format!("refs/heads/{branch}")],
                control,
            )?;
            if head.is_empty() && !["fetch", "fetch_pr"].contains(&action) {
                return Err(DomainError::InvalidInput(
                    "Commit before creating another branch".into(),
                ));
            }
        }
        let status = self.run(
            &["status", "--short", "--branch", "--untracked-files=normal"],
            control,
        )?;
        let diff = self.display(
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
            ],
            control,
        )?;
        let revision = format!(
            "local-git-{:x}",
            Sha256::digest(
                json!([
                    scope,
                    action,
                    args,
                    head,
                    branch,
                    index.as_ref().map(|(_, r)| format!("{r:?}")),
                    sources
                        .iter()
                        .map(|(p, s)| (p, format!("{:?}", s.as_ref().map(|s| &s.revision))))
                        .collect::<Vec<_>>()
                ])
                .to_string()
            )
        );
        let mut preview = json!({"action":action,"folder":self.workspace.root(),"repository":repo,"branch":branch,"head":head,"status":status,"staged_diff":diff,"files":sources.iter().map(|(p,s)|match s {Some(s)=>json!({"path":p,"bytes":s.entry.bytes.len(),"sha256":s.revision.sha256,"mode":format!("{:o}",s.entry.mode),"operation":if s.entry.mode==0o120000{"stage link target bytes (not followed)"}else{"stage exact file bytes"}}),None=>json!({"path":p,"operation":"stage deletion (index only)","bytes":0})}).collect::<Vec<_>>(),"arguments":args,"limits":"Exact file/link bytes or tracked deletions; 1–64 exact paths / 8 MiB per entry / 16 MiB total. No symlink targets, filters, hooks, signing or shell are executed. Index reversal uses unstage, not file Undo. Linked worktree metadata is explicitly bound to Chat folders grants; metadata and working files must share a filesystem."});
        let network = if ["fetch", "fetch_pr", "push", "pull"].contains(&action) {
            if self
                .run(&["rev-parse", "--show-object-format"], control)?
                .trim()
                != "sha1"
            {
                return Err(DomainError::Unavailable(
                    "This GitHub network transport requires a SHA-1 repository".into(),
                ));
            }
            let expected=args["expected_remote_sha"].as_str().filter(|s|s.len()==40&&oid(s)).ok_or_else(||DomainError::InvalidInput("Provide the exact reviewed 40-character remote SHA (all zeros only when creating a new push branch)".into()))?;
            if action != "push" && expected == "0".repeat(40) {
                return Err(DomainError::InvalidInput(
                    "Fetch requires an existing reviewed commit".into(),
                ));
            }
            if action == "push" && expected != "0".repeat(40) {
                self.run(&["merge-base", "--is-ancestor", expected, &head], control)?;
            }
            let tracking = format!("refs/remotes/origin/{}", args["branch"].as_str().unwrap());
            let old = self.reference(&tracking, control)?;
            if action == "pull" && old != expected {
                return Err(DomainError::InvalidInput("Fetch this exact remote branch/SHA first, then review pull's working-file changes in a separate confirmation".into()));
            }
            let admission = self
                .github
                .as_ref()
                .ok_or_else(|| {
                    DomainError::Unavailable(
                        "Connect the shared GitHub service for network Git".into(),
                    )
                })?
                .prepare_git(scope, control)
                .map_err(super::tool::domain)?;
            preview["account"] = admission.account();
            preview["remote_url"] = json!(format!("https://github.com/{repo}.git"));
            preview["tracking_ref"] = json!(tracking);
            preview["tracking_before"] = json!(old);
            preview["remote_branch"] = json!(if action == "fetch_pr" {
                format!("refs/pull/{}/head", args["pull_number"].as_u64().unwrap())
            } else {
                format!("refs/heads/{}", args["branch"].as_str().unwrap())
            });
            preview["expected_remote_sha"] = json!(expected);
            preview["network_warning"] = json!(if action == "push" {
                "Publishes the exact reviewed local commit to this branch. Requires the exact expected remote head and fast-forward ancestry; may trigger repository CI. No automatic retries."
            } else {
                "Downloads the exact reviewed commit and updates only this remote-tracking ref with compare-and-swap. Working files, local branch and index are not changed."
            });
            if action == "pull" {
                preview["network_warning"] = json!(
                    "Revalidates this exact remote head, then fast-forwards the current same-named branch and applies the reviewed working-file/index changes. Fetch must already have completed. Divergence and conflicting dirty changes are refused; no merge, rebase, stash or automatic retries."
                );
            }
            Some(admission)
        } else {
            None
        };
        preview["metadata"] = self.metadata()?.identity();
        if !["status", "diff", "branches"].contains(&action) {
            preview["storage"] = self.storage_admit(action, control)?;
        }
        preview["display_limits"] = json!(
            "Diff text replaces invalid UTF-8 for display only; staged blob bytes are unchanged."
        );
        if Some(self.metadata_snapshot(".git/config", 1024 * 1024)?.revision) != config_revision {
            return Err(DomainError::Conflict);
        }
        let transition = if ["checkout", "pull"].contains(&action) {
            let t = self.prepare_transition(action, args, &head, &branch, control)?;
            t.describe(&mut preview, self.root());
            Some(t)
        } else {
            None
        };
        let revision = format!(
            "local-git-{:x}",
            Sha256::digest(json!([revision, preview, format!("{config_revision:?}")]).to_string())
        );
        Ok(LocalPlan {
            action: action.into(),
            args: args.clone(),
            scope: scope.into(),
            head,
            branch,
            index,
            sources,
            config_revision,
            network,
            clone_target: None,
            transition,
            revision,
            expires: Instant::now() + Duration::from_secs(300),
            preview,
        })
    }
    pub fn execute(
        &self,
        plan: LocalPlan,
        scope: &str,
        control: &ScanControl<'_>,
    ) -> Result<Value> {
        control.check()?;
        self.authority(plan.may_mutate())?;
        if plan.scope != scope || Instant::now() >= plan.expires {
            return Err(DomainError::PermissionDenied);
        }
        if !plan.may_mutate() {
            return self.execute_admitted(plan, scope, control);
        }
        let lease = storage::Lease::acquire(self)?;
        self.storage_admit(&plan.action, control)?;
        let mode = if plan.action == "recover" { 2 } else { 1 };
        self.storage_active
            .store(mode, std::sync::atomic::Ordering::Release);
        struct Active<'a>(&'a std::sync::atomic::AtomicU8);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.store(0, std::sync::atomic::Ordering::Release);
            }
        }
        let _active = Active(&self.storage_active);
        let result = self.execute_admitted(plan, scope, control);
        if result.is_ok() {
            lease
                .verify()
                .and_then(|_| self.storage_check_now(control))
                .map_err(|_| DomainError::DurabilityUnknown)?;
        }
        result
    }
    fn execute_admitted(
        &self,
        mut plan: LocalPlan,
        scope: &str,
        control: &ScanControl<'_>,
    ) -> Result<Value> {
        control.check()?;
        self.authority(plan.may_mutate())?;
        if plan.scope != scope || Instant::now() >= plan.expires {
            return Err(DomainError::PermissionDenied);
        }
        if plan.clone_target.is_some() {
            return self.execute_clone(plan, scope, control);
        }
        if plan.action == "recovery_status" {
            return self.recovery_status(&plan.args, control);
        }
        if plan.action == "recover" {
            return self.execute_recovery(plan, control);
        }
        if plan.may_mutate() {
            self.transition_ready()?;
        }
        if self.state(control)? != (plan.head.clone(), plan.branch.clone())
            || self.index()?.as_ref().map(|(_, r)| r) != plan.index.as_ref().map(|(_, r)| r)
            || Some(self.metadata_snapshot(".git/config", 1024 * 1024)?.revision)
                != plan.config_revision
        {
            return Err(DomainError::Conflict);
        }
        for (path, snapshot) in &plan.sources {
            if self.workspace.git_snapshot(path)?.as_ref() != snapshot.as_ref() {
                return Err(DomainError::Conflict);
            }
        }
        if let Some(admission) = plan.network.take() {
            return self
                .github
                .as_ref()
                .ok_or(DomainError::PermissionDenied)?
                .execute_git(admission, scope, control, |credential, c| {
                    let network = self.execute_network(&plan, credential, c)?;
                    if plan.action == "pull" {
                        self.execute_transition(plan, c)
                    } else {
                        Ok(network)
                    }
                });
        }
        if plan.transition.is_some() {
            return self.execute_transition(plan, control);
        }
        match plan.action.as_str() {
            "status" => {
                return Ok(json!({"status":self.run(&["status","--short","--branch"],control)?}));
            }
            "branches" => {
                return Ok(
                    json!({"branches":self.run(&["branch","--list","--no-color"],control)?}),
                );
            }
            "diff" => {
                return Ok(
                    json!({"diff":self.display(&["diff","--no-ext-diff","--no-textconv","--no-color"],control)?,"staged":plan.preview["staged_diff"],"display_limits":"Diff text replaces invalid UTF-8 for display only; staged blob bytes are unchanged."}),
                );
            }
            _ => {}
        }
        let lock = IndexLock::acquire(&self.metadata()?.git)?;
        if self.index()?.as_ref().map(|(_, r)| r) != plan.index.as_ref().map(|(_, r)| r) {
            return Err(DomainError::Conflict);
        }
        let temp = tempfile::Builder::new()
            .prefix("git-index-")
            .tempdir_in(&self.state)
            .map_err(io_error)?;
        if let Some((bytes, _)) = &plan.index {
            std::fs::write(temp.path().join("index"), bytes).map_err(io_error)?;
        }
        let private = WorkspaceService::new()?.register(temp.path())?;
        let mut env = BTreeMap::from([("GIT_INDEX_FILE".into(), "/git-private/index".into())]);
        let command = |args: Vec<String>,
                       input: Option<&[u8]>,
                       env: BTreeMap<String, String>|
         -> Result<String> {
            let (bytes, ok) = self.command(&args, input, Some(&private), env, control)?;
            if !ok {
                return Err(DomainError::InvalidInput(
                    "Git plumbing rejected the change; no automatic retry".into(),
                ));
            }
            String::from_utf8(bytes).map_err(|_| DomainError::Conflict)
        };
        if plan.index.is_none() {
            command(
                vec!["read-tree".into(), "--empty".into()],
                None,
                env.clone(),
            )?;
        }
        match plan.action.as_str() {
            "stage" => {
                // Deletions precede additions so a deliberately selected
                // directory/file transition never needs recursive replacement.
                for (path, _) in plan.sources.iter().filter(|(_, s)| s.is_none()) {
                    command(
                        vec![
                            "update-index".into(),
                            "--force-remove".into(),
                            "--".into(),
                            path.clone(),
                        ],
                        None,
                        env.clone(),
                    )?;
                }
                for (path, snapshot) in &plan.sources {
                    let Some(snapshot) = snapshot else { continue };
                    let hash = command(
                        vec![
                            "hash-object".into(),
                            "-w".into(),
                            "--no-filters".into(),
                            "--stdin".into(),
                        ],
                        Some(&snapshot.entry.bytes),
                        env.clone(),
                    )?;
                    let hash = hash.trim();
                    if !oid(hash) {
                        return Err(DomainError::Conflict);
                    }
                    let mode = format!("{:o}", snapshot.entry.mode);
                    command(
                        vec![
                            "update-index".into(),
                            "--add".into(),
                            "--cacheinfo".into(),
                            format!("{mode},{hash},{path}"),
                        ],
                        None,
                        env.clone(),
                    )?;
                }
            }
            "unstage" => {
                for path in plan.args["paths"].as_array().unwrap() {
                    let path = path.as_str().unwrap();
                    let tree = if plan.head.is_empty() {
                        String::new()
                    } else {
                        command(
                            vec![
                                "ls-tree".into(),
                                "-z".into(),
                                plan.head.clone(),
                                "--".into(),
                                path.into(),
                            ],
                            None,
                            env.clone(),
                        )?
                    };
                    if tree.is_empty() {
                        command(
                            vec![
                                "update-index".into(),
                                "--force-remove".into(),
                                "--".into(),
                                path.into(),
                            ],
                            None,
                            env.clone(),
                        )?;
                    } else {
                        let (metadata, name) = tree
                            .trim_end_matches('\0')
                            .split_once('\t')
                            .ok_or(DomainError::Conflict)?;
                        if name != path {
                            return Err(DomainError::Conflict);
                        }
                        let columns: Vec<_> = metadata.split_whitespace().collect();
                        if columns.len() != 3 || columns[1] != "blob" || !oid(columns[2]) {
                            return Err(DomainError::Conflict);
                        }
                        command(
                            vec![
                                "update-index".into(),
                                "--add".into(),
                                "--cacheinfo".into(),
                                format!("{},{},{path}", columns[0], columns[2]),
                            ],
                            None,
                            env.clone(),
                        )?;
                    }
                }
            }
            "commit" => {
                let tree = command(vec!["write-tree".into()], None, env.clone())?
                    .trim()
                    .to_string();
                if !oid(&tree) {
                    return Err(DomainError::Conflict);
                }
                for (key, arg) in [
                    ("GIT_AUTHOR_NAME", "author_name"),
                    ("GIT_COMMITTER_NAME", "author_name"),
                    ("GIT_AUTHOR_EMAIL", "author_email"),
                    ("GIT_COMMITTER_EMAIL", "author_email"),
                ] {
                    env.insert(key.into(), plan.args[arg].as_str().unwrap().into());
                }
                let mut args = vec![
                    "commit-tree".into(),
                    "--no-gpg-sign".into(),
                    tree,
                    "-F".into(),
                    "-".into(),
                ];
                if !plan.head.is_empty() {
                    args.extend(["-p".into(), plan.head.clone()]);
                }
                let commit = command(
                    args,
                    Some(plan.args["message"].as_str().unwrap().as_bytes()),
                    env.clone(),
                )?
                .trim()
                .to_string();
                if !oid(&commit) {
                    return Err(DomainError::Conflict);
                }
                control.check()?;
                if self.state(control)? != (plan.head.clone(), plan.branch.clone()) {
                    return Err(DomainError::Conflict);
                }
                let old = if plan.head.is_empty() {
                    "0".repeat(commit.len())
                } else {
                    plan.head.clone()
                };
                lock.verify()?;
                let transaction =
                    format!("start\nupdate {} {commit} {old}\nprepare\n", plan.branch);
                let guard = |_: &crate::process::ProcessIdentity| {
                    control.check()?;
                    self.authority(true)?;
                    self.storage_check(control)?;
                    lock.verify()?;
                    if self.state(control)? != (plan.head.clone(), plan.branch.clone()) {
                        return Err(DomainError::Conflict);
                    }
                    Ok(())
                };
                let (_, ok) = self
                    .command_guarded(
                        &[
                            "update-ref".into(),
                            "-m".into(),
                            "AG1 reviewed commit".into(),
                            "--no-deref".into(),
                            "--stdin".into(),
                        ],
                        Some(transaction.as_bytes()),
                        Some(&private),
                        env,
                        control,
                        Some(&guard),
                        false,
                    )
                    .map_err(|_| DomainError::DurabilityUnknown)?;
                if !ok {
                    return Err(DomainError::DurabilityUnknown);
                }
                return Ok(
                    json!({"commit":commit,"branch":plan.branch,"working_files_preserved":true}),
                );
            }
            "branch_create" => {
                let branch = format!("refs/heads/{}", plan.args["branch"].as_str().unwrap());
                lock.verify()?;
                let transaction = format!(
                    "start\nverify {} {}\ncreate {branch} {}\nprepare\n",
                    plan.branch, plan.head, plan.head
                );
                let guard = |_: &crate::process::ProcessIdentity| {
                    control.check()?;
                    self.authority(true)?;
                    self.storage_check(control)?;
                    lock.verify()?;
                    if self.state(control)? != (plan.head.clone(), plan.branch.clone()) {
                        return Err(DomainError::Conflict);
                    }
                    Ok(())
                };
                let (_, ok) = self
                    .command_guarded(
                        &["update-ref".into(), "--no-deref".into(), "--stdin".into()],
                        Some(transaction.as_bytes()),
                        Some(&private),
                        env,
                        control,
                        Some(&guard),
                        false,
                    )
                    .map_err(|_| DomainError::DurabilityUnknown)?;
                if !ok {
                    return Err(DomainError::DurabilityUnknown);
                }
                return Ok(json!({"created":branch,"working_files_preserved":true}));
            }
            _ => return Err(DomainError::PermissionDenied),
        }
        control.check()?;
        let candidate = std::fs::read(temp.path().join("index")).map_err(io_error)?;
        if candidate.len() > 8 * 1024 * 1024 {
            return Err(DomainError::LimitExceeded("Git index".into()));
        }
        for (path, snapshot) in &plan.sources {
            if self.workspace.git_snapshot(path)?.as_ref() != snapshot.as_ref() {
                return Err(DomainError::Conflict);
            }
        }
        // Native compare-and-publish preserves a concurrent index update.
        let check = || {
            control.check()?;
            self.authority(true)?;
            self.storage_check(control)?;
            lock.verify()?;
            for (path, snapshot) in &plan.sources {
                if self.workspace.git_snapshot(path)?.as_ref() != snapshot.as_ref() {
                    return Err(DomainError::Conflict);
                }
            }
            if Some(self.metadata_snapshot(".git/config", 1024 * 1024)?.revision)
                != plan.config_revision
            {
                return Err(DomainError::Conflict);
            }
            if self.state(control)? != (plan.head.clone(), plan.branch.clone()) {
                return Err(DomainError::Conflict);
            }
            Ok(())
        };
        // Keep the exact preimage in private host state before index publication.
        let private_storage = self.git_private_storage()?;
        let private_pin = private_storage.pin_directory("")?;
        let recovery = private_pin
            .process_cwd()
            .join(format!("git-index-recovery-{}", crate::client::nonce()?));
        if let Some((bytes, _)) = &plan.index {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&recovery)
                .map_err(io_error)?;
            file.write_all(bytes).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            std::fs::File::open(private_pin.process_cwd())
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
        }
        match &plan.index {
            Some((_, revision)) => {
                self.metadata()?
                    .git
                    .replace_checked("index", revision, &candidate, check)?;
            }
            None => {
                self.metadata()?
                    .git
                    .create_checked("index", &candidate, false, check)?;
            }
        }
        lock.verify().map_err(|_| DomainError::DurabilityUnknown)?;
        Ok(json!({"action":plan.action,"working_files_preserved":true,"paths":plan.args["paths"]}))
    }
}
fn oid(value: &str) -> bool {
    [40, 64].contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn exact_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 2048
        && !path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .all(|s| !s.is_empty() && ![".", "..", ".git"].contains(&s))
}

fn abort_reference_transaction(child: &mut crate::process::OwnedProcess) -> bool {
    let until = Instant::now() + Duration::from_millis(500);
    let abort = b"abort\n";
    let mut offset = 0;
    while offset < abort.len() && Instant::now() < until {
        match child.try_write_input(&abort[offset..]) {
            Ok(Some(n)) if n > 0 => offset += n,
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            _ => break,
        }
    }
    child.close_input();
    while Instant::now() < until {
        match child.poll_stdout(32768) {
            Ok((_, Some(status))) => return status.success(),
            Ok(_) => std::thread::sleep(Duration::from_millis(2)),
            Err(_) => break,
        }
    }
    false
}

/// Git-compatible exclusive lock, pinned to the native metadata directory.
/// Never removes a lock replaced by another process.
struct IndexLock {
    pin: crate::workspace::PinnedDirectory,
    directory: std::fs::File,
    file: std::fs::File,
}
impl IndexLock {
    fn acquire(workspace: &Workspace) -> Result<Self> {
        let pin = workspace.pin_directory("")?;
        let directory = std::fs::File::open(pin.process_cwd()).map_err(io_error)?;
        let fd = rustix::fs::openat(
            &directory,
            "index.lock",
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .map_err(|_| DomainError::Conflict)?;
        let lock = Self {
            pin,
            directory,
            file: std::fs::File::from(fd),
        };
        lock.verify()?;
        Ok(lock)
    }
    fn verify(&self) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        self.pin.verify_identity()?;
        let current = rustix::fs::statat(
            &self.directory,
            "index.lock",
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|_| DomainError::Conflict)?;
        let ours = self.file.metadata().map_err(io_error)?;
        if current.st_dev != ours.dev() || current.st_ino != ours.ino() {
            return Err(DomainError::Conflict);
        }
        Ok(())
    }
}
impl Drop for IndexLock {
    fn drop(&mut self) {
        if self.verify().is_ok() {
            let _ =
                rustix::fs::unlinkat(&self.directory, "index.lock", rustix::fs::AtFlags::empty());
        }
    }
}
