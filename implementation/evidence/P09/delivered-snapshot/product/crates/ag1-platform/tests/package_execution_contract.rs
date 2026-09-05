#![cfg(target_os = "linux")]
use ag1_core::{
    Clock, DomainError, SessionId,
    context::{ContextArtifact, ContextArtifactPort, features::Features},
    resources::{package_lifecycle, packages},
    tools::{
        approval::{AuthorizedCall, Authorizer, ExecutionFence, Preflight, PreflightContext},
        executor::{self, Effect, ExecutionArtifacts, NoHooks, Status},
        policy::Policy,
        registry::{Registry, Scope},
        result::{ResultArtifacts, ResultBudget},
    },
};
use ag1_platform::{
    package_execution::{self, Binary, PackageService, PreparedPackage},
    process::ProcessSupervisor,
    resource_fs::ScanControl,
    workspace::{Workspace, WorkspaceService},
};
use ag1_storage::{Store, context::ContextStore};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
const NOW: &str = "2026-09-05T00:00:00.000Z";
struct TestClock;
impl Clock for TestClock {
    fn now_ms(&self) -> u64 {
        12
    }
}
struct Fixture {
    dir: tempfile::TempDir,
    ws: Arc<Workspace>,
    service: PackageService,
    registry: Registry,
    scope: Scope,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        for p in [
            "project/.ag1/npm",
            "project/.ag1/git",
            "agent/npm",
            "agent/git",
            "temporary",
            "spools",
        ] {
            fs::create_dir_all(dir.path().join(p)).unwrap();
        }
        let workspaces = WorkspaceService::new().unwrap();
        let ws = workspaces.register(&dir.path().join("project")).unwrap();
        let roots = [
            "project/.ag1/npm",
            "project/.ag1/git",
            "agent/npm",
            "agent/git",
            "temporary",
        ]
        .map(|p| workspaces.register(&dir.path().join(p)).unwrap());
        let executable = dir.path().join("fixture-bin");
        fs::copy(env!("CARGO_BIN_EXE_ag1-platform-fixture"), &executable).unwrap();
        let binaries = ["npm", "git"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    Binary {
                        executable: executable.clone(),
                        arguments: vec!["package".into(), name.into()],
                    },
                )
            })
            .collect();
        let service = PackageService::new(
            ws.clone(),
            Arc::new(
                ProcessSupervisor::new(Path::new(env!("CARGO_BIN_EXE_ag1-process-host"))).unwrap(),
            ),
            package_execution::Config {
                manager: package_lifecycle::Config {
                    cwd: dir.path().join("project").to_str().unwrap().into(),
                    agent_dir: dir.path().join("agent").to_str().unwrap().into(),
                    home: dir.path().to_str().unwrap().into(),
                    temporary_dir: dir.path().join("temporary").to_str().unwrap().into(),
                    npm: packages::Command {
                        command: "npm".into(),
                        args: vec![],
                    },
                    offline: false,
                },
                writable_roots: roots.to_vec(),
                readable_roots: vec![],
                npm_prefix: roots[2].clone(),
                binaries,
                env: BTreeMap::from([(
                    "AG1_PACKAGE_FIXTURE_ROOT".into(),
                    dir.path().to_str().unwrap().into(),
                )]),
                spools: dir.path().join("spools"),
            },
        )
        .unwrap();
        let mut registry = Registry::builtins().unwrap();
        PackageService::register(&mut registry).unwrap();
        registry
            .activate(&["managePackage".into()], &["managePackage".into()].into())
            .unwrap();
        let scope = Scope {
            workspace: ws.id().as_str().into(),
            session: "session".into(),
            epoch: 1,
            revision: 0,
        };
        let store = Store::open(&dir.path().join("store")).unwrap();
        store.create_session("session", json!({"type":"session","version":3,"id":"session","cwd":ws.root(),"timestamp":NOW}).to_string().as_bytes()).unwrap();
        store
            .register_origin("origin", "session", &[4; 32])
            .unwrap();
        Self {
            dir,
            ws,
            service,
            registry,
            scope,
            store,
        }
    }
    fn prepare(
        &self,
        operation: &str,
        action: &str,
        source: &str,
        scope: &str,
    ) -> Result<PreparedPackage, DomainError> {
        let call = self.registry.prepare(
            operation,
            operation,
            "managePackage",
            &json!({"action":action,"source":source,"scope":scope}),
            true,
        )?;
        self.service.prepare(&call, &self.scope)
    }
    fn ticket(&self, prepared: &PreparedPackage) -> AuthorizedCall {
        let mut authorizer = Authorizer::new("package-fixture").unwrap();
        let policy: Policy =
            serde_json::from_value(json!({"mode":"interactive","cwd":self.ws.root()})).unwrap();
        let Preflight::Approval(request) = authorizer
            .preflight(
                prepared.call().clone(),
                PreflightContext {
                    scope: &self.scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: prepared.target_revision(),
                    now: 10,
                    ttl: 1000,
                    untrusted_extensions: false,
                },
            )
            .unwrap()
        else {
            panic!("package mutation must request approval")
        };
        authorizer
            .answer(
                &request.binding,
                true,
                &self.registry,
                ExecutionFence {
                    scope: &self.scope,
                    policy_revision: 1,
                    target_revision: prepared.target_revision(),
                    now: 11,
                },
            )
            .unwrap()
            .unwrap()
    }
    fn artifacts(&self, operation: &str) -> ContextStore<'_> {
        ContextStore::new(
            &self.store,
            SessionId::new("session").unwrap(),
            "origin",
            operation,
            NOW,
            Features::default(),
        )
        .unwrap()
    }
    fn execute<A: ExecutionArtifacts>(
        &self,
        prepared: PreparedPackage,
        ticket: AuthorizedCall,
        artifacts: &mut A,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<executor::Outcome, DomainError> {
        let target = prepared.target_revision().to_owned();
        self.service.execute(
            &self.store,
            artifacts,
            prepared,
            ticket,
            package_execution::Execution {
                lifecycle: executor::Execution {
                    registry: &self.registry,
                    fence: ExecutionFence {
                        scope: &self.scope,
                        policy_revision: 1,
                        target_revision: &target,
                        now: 12,
                    },
                    budget: ResultBudget::default(),
                    clock: &TestClock,
                },
                control: ScanControl { cancel, deadline },
            },
            &mut NoHooks,
        )
    }
    fn run(&self, operation: &str, action: &str, source: &str, scope: &str) -> executor::Outcome {
        let prepared = self.prepare(operation, action, source, scope).unwrap();
        let ticket = self.ticket(&prepared);
        self.execute(
            prepared,
            ticket,
            &mut self.artifacts(operation),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap()
    }
    fn commands(&self) -> Vec<Value> {
        match fs::read_to_string(self.dir.path().join("commands.jsonl")) {
            Ok(s) => s
                .lines()
                .map(|s| serde_json::from_str(s).unwrap())
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => panic!("{e}"),
        }
    }
}
#[test]
fn captured_package_selection_and_pin_resolution_do_not_start_commands_before_approved_install() {
    use package_lifecycle::{ConfiguredPackage, Scope as PackageScope};
    let f = Fixture::new();
    let cancel = AtomicBool::new(false);
    let control = || ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(2),
    };
    let entries = vec![
        ConfiguredPackage {
            source: "npm:fixture".into(),
            scope: PackageScope::User,
        },
        ConfiguredPackage {
            source: "npm:fixture@2.0.0".into(),
            scope: PackageScope::Project,
        },
        ConfiguredPackage {
            source: "npm:other".into(),
            scope: PackageScope::User,
        },
    ];
    assert_eq!(
        f.service
            .update_check_queries(&entries, control())
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.service
            .needs_install("npm:fixture@1.0.0", PackageScope::Project, control())
            .unwrap()
    );
    assert!(f.commands().is_empty());
    assert_eq!(
        f.run("first-pin", "install", "npm:fixture@1.0.0", "project")
            .result["isError"],
        false
    );
    let count = f.commands().len();
    assert!(
        !f.service
            .needs_install("npm:fixture", PackageScope::Project, control())
            .unwrap()
    );
    assert!(
        !f.service
            .needs_install("npm:fixture@1.0.0", PackageScope::Project, control())
            .unwrap()
    );
    assert!(
        f.service
            .needs_install("npm:fixture@2.0.0", PackageScope::Project, control())
            .unwrap()
    );
    assert_eq!(f.commands().len(), count);
    assert_eq!(
        f.run("refresh-pin", "install", "npm:fixture@2.0.0", "project")
            .result["isError"],
        false
    );
    assert!(
        !f.service
            .needs_install("npm:fixture@2.0.0", PackageScope::Project, control())
            .unwrap()
    );
    let manifest: Value = serde_json::from_slice(
        &fs::read(
            f.ws.root()
                .join(".ag1/npm/node_modules/fixture/package.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["version"], "2.0.0");
}

#[test]
fn actual_owned_npm_git_commands_settle_with_artifacts_and_replay_without_reexecution() {
    let f = Fixture::new();
    let prepared = f
        .prepare("install", "install", "npm:fixture", "project")
        .unwrap();
    assert!(f.commands().is_empty());
    assert!(!f.ws.root().join(".ag1/npm/package.json").exists());
    let ticket = f.ticket(&prepared);
    assert!(f.commands().is_empty());
    let mut artifacts = f.artifacts("install");
    let outcome = f
        .execute(
            prepared,
            ticket,
            &mut artifacts,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(outcome.record.status, Status::Completed);
    assert_eq!(outcome.record.effect, Effect::Committed);
    assert_eq!(outcome.result["isError"], false, "{}", outcome.result);
    let receipt: ContextArtifact =
        serde_json::from_value(outcome.result["details"]["commands"][0]["artifact"].clone())
            .unwrap();
    let bytes = artifacts.read_json(&receipt).unwrap();
    assert_eq!(
        STANDARD
            .decode(bytes["stdoutBase64"].as_str().unwrap())
            .unwrap(),
        b"fixture npm complete\n"
    );
    assert_eq!(
        STANDARD
            .decode(bytes["stderrBase64"].as_str().unwrap())
            .unwrap(),
        b"fixture diagnostic\n"
    );
    assert!(
        f.ws.root()
            .join(".ag1/npm/node_modules/fixture/package.json")
            .exists()
    );
    assert_eq!(f.commands().len(), 1);
    let replay = f.run("install", "install", "npm:fixture", "project");
    assert!(replay.replayed);
    assert_eq!(f.commands().len(), 1);
    assert_eq!(
        f.run("global", "install", "npm:fixture", "user").result["isError"],
        false
    );
    assert!(
        f.dir
            .path()
            .join("agent/npm/lib/node_modules/fixture/package.json")
            .exists()
    );
    let global = f.commands().last().unwrap().clone();
    assert_eq!(
        global["args"].as_array().unwrap().last().unwrap(),
        f.dir.path().join("agent/npm").to_str().unwrap()
    );
    assert_eq!(global["userconfig"], "/dev/null");
    assert_eq!(
        f.run("temporary", "install", "npm:fixture", "temporary")
            .result["isError"],
        false
    );
    let temp_command = f.commands().last().unwrap().clone();
    let temp_prefix = Path::new(
        temp_command["args"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .as_str()
            .unwrap(),
    );
    assert!(temp_prefix.starts_with(f.dir.path().join("temporary")));
    assert!(
        temp_prefix
            .join("node_modules/fixture/package.json")
            .exists()
    );
    assert_eq!(
        f.run("temporary-update", "update", "npm:fixture", "temporary")
            .result["isError"],
        false
    );
    assert_eq!(
        f.commands().last().unwrap()["args"]
            .as_array()
            .unwrap()
            .last()
            .unwrap(),
        temp_prefix.to_str().unwrap()
    );
    assert_eq!(
        f.run("temporary-remove", "remove", "npm:fixture", "temporary")
            .result["isError"],
        false
    );
    assert!(!temp_prefix.join("node_modules/fixture").exists());
    assert!(
        f.dir
            .path()
            .join("agent/npm/lib/node_modules/fixture/package.json")
            .exists()
    );
    assert!(
        global["home"].is_null(),
        "ambient HOME must not be inherited"
    );
    assert_eq!(
        f.run(
            "git",
            "install",
            "git:github.com/acme/fixture@v1",
            "project"
        )
        .result["isError"],
        false
    );
    assert!(
        f.ws.root()
            .join(".ag1/git/github.com/acme/fixture/package.json")
            .exists()
    );
    assert_eq!(
        f.run(
            "remove",
            "remove",
            "git:github.com/acme/fixture@v1",
            "project"
        )
        .result["isError"],
        false
    );
    assert!(
        !f.ws.root().join(".ag1/git/github.com").exists(),
        "empty ancestors must be pruned"
    );
    assert!(f.ws.root().join(".ag1/git/.gitignore").exists());
    assert_eq!(
        fs::read_dir(f.dir.path().join("spools")).unwrap().count(),
        0
    );
}
#[test]
fn failures_output_limits_and_cancellation_preserve_known_results_and_stop_owned_children() {
    let f = Fixture::new();
    let failure = f.run("failure", "install", "npm:fail", "project");
    assert_eq!(failure.record.status, Status::Completed);
    assert_eq!(failure.record.effect, Effect::Unknown);
    assert_eq!(failure.result["isError"], true);
    assert!(
        failure.result["details"]["error"]
            .as_str()
            .unwrap()
            .contains("Exited(17)")
    );
    let flood = f.run("flood", "install", "npm:flood", "project");
    assert_eq!(flood.result["isError"], true);
    assert_eq!(flood.result["details"]["commands"][0]["truncated"], true);
    let cancel = AtomicBool::new(false);
    let prepared = f
        .prepare("cancel", "install", "npm:slow", "project")
        .unwrap();
    let ticket = f.ticket(&prepared);
    std::thread::scope(|threads| {
        let marker = f.dir.path().join("started");
        let cancellation = &cancel;
        threads.spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !marker.exists() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            cancellation.store(true, Ordering::Release);
        });
        let result = f
            .execute(
                prepared,
                ticket,
                &mut f.artifacts("cancel"),
                &cancel,
                Instant::now() + Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(result.record.status, Status::Completed);
        assert_eq!(result.record.effect, Effect::Unknown);
        assert_eq!(result.result["details"]["error"], "Cancelled");
    });
    let pid = fs::read_to_string(f.dir.path().join("started")).unwrap();
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let count = f.commands().len();
    assert!(f.run("cancel", "install", "npm:slow", "project").replayed);
    assert_eq!(f.commands().len(), count);
    fs::remove_file(f.dir.path().join("started")).unwrap();
    let prepared = f
        .prepare("timeout", "install", "npm:slow", "project")
        .unwrap();
    let ticket = f.ticket(&prepared);
    let timed = f
        .execute(
            prepared,
            ticket,
            &mut f.artifacts("timeout"),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_millis(500),
        )
        .unwrap();
    assert_eq!(timed.result["details"]["error"], "TimedOut");
    assert_eq!(timed.record.effect, Effect::Unknown);
    let pid = fs::read_to_string(f.dir.path().join("started")).unwrap();
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    assert_eq!(
        fs::read_dir(f.dir.path().join("spools")).unwrap().count(),
        0
    );
}
#[test]
fn scope_sources_replaced_binaries_and_stale_packages_fail_before_child_launch() {
    let f = Fixture::new();
    for input in [
        "npm:--help",
        "git:github.com/acme/repo@--orphan",
        "git:https://user:secret@example.com/acme/repo",
    ] {
        assert!(
            f.prepare("invalid", "install", input, "project").is_err(),
            "{input}"
        );
    }
    assert!(f.commands().is_empty());
    let stale = f
        .prepare("stale", "install", "npm:fixture", "project")
        .unwrap();
    let ticket = f.ticket(&stale);
    f.run("advance", "install", "npm:fixture", "project");
    let rejected = f
        .execute(
            stale,
            ticket,
            &mut f.artifacts("stale"),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(rejected.record.status, Status::Completed);
    assert_eq!(rejected.record.effect, Effect::None);
    assert_eq!(rejected.result["details"]["error"], "Conflict");
    assert_eq!(f.commands().len(), 1);
    std::os::unix::fs::symlink(f.dir.path(), f.ws.root().join(".ag1/git/github.com")).unwrap();
    let blocked = f.run("escape", "install", "git:github.com/acme/repo", "project");
    assert_eq!(blocked.result["isError"], true);
    assert_eq!(f.commands().len(), 1);
    let binary = f.dir.path().join("fixture-bin");
    fs::rename(&binary, f.dir.path().join("fixture-bin-old")).unwrap();
    fs::copy(f.dir.path().join("fixture-bin-old"), &binary).unwrap();
    let replaced = f.run("replaced", "install", "npm:fixture", "project");
    assert_eq!(replaced.result["isError"], true);
    assert_eq!(replaced.result["details"]["error"], "Conflict");
    assert_eq!(f.commands().len(), 1);
}
struct FailOutput<'a> {
    inner: ContextStore<'a>,
}
impl ContextArtifactPort for FailOutput<'_> {
    fn put_evidence(
        &mut self,
        text: &str,
        kind: &str,
        producer: &Value,
        salient: &[String],
        parent: Option<&str>,
    ) -> Result<ContextArtifact, DomainError> {
        if kind == "package-command-output" {
            Err(DomainError::Io("fixture artifact failure".into()))
        } else {
            self.inner
                .put_evidence(text, kind, producer, salient, parent)
        }
    }
    fn put_text(
        &mut self,
        text: &str,
        kind: &str,
        tool: &str,
        call_id: &str,
        salient: &[String],
    ) -> Result<ContextArtifact, DomainError> {
        self.inner.put_text(text, kind, tool, call_id, salient)
    }
    fn get(&self, id: &str) -> Result<ContextArtifact, DomainError> {
        self.inner.get(id)
    }
    fn leased_ids(&self) -> Result<Vec<String>, DomainError> {
        self.inner.leased_ids()
    }
}
impl ResultArtifacts for FailOutput<'_> {
    fn read_call(&self, id: &str, offset: u64, max_bytes: u32) -> Result<Value, DomainError> {
        self.inner.read_call(id, offset, max_bytes)
    }
}
impl ExecutionArtifacts for FailOutput<'_> {
    fn read_json(&self, a: &ContextArtifact) -> Result<Value, DomainError> {
        self.inner.read_json(a)
    }
}
#[test]
fn output_acknowledgement_failure_leaves_intent_and_owned_spools_for_reconciliation() {
    let f = Fixture::new();
    let prepared = f
        .prepare("uncertain", "install", "npm:fixture", "project")
        .unwrap();
    let ticket = f.ticket(&prepared);
    let mut artifacts = FailOutput {
        inner: f.artifacts("uncertain"),
    };
    assert!(matches!(
        f.execute(
            prepared,
            ticket,
            &mut artifacts,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10)
        ),
        Err(DomainError::DurabilityUnknown)
    ));
    let paths: Vec<_> = fs::read_dir(f.dir.path().join("spools"))
        .unwrap()
        .map(|p| p.unwrap().path())
        .collect();
    assert_eq!(paths.len(), 3);
    assert_eq!(
        fs::read(
            paths
                .iter()
                .find(|p| p.extension().unwrap() == "stdout")
                .unwrap()
        )
        .unwrap(),
        b"fixture npm complete\n"
    );
    assert_eq!(f.commands().len(), 1);
    let prepared = f
        .prepare("uncertain", "install", "npm:fixture", "project")
        .unwrap();
    let ticket = f.ticket(&prepared);
    assert!(matches!(
        f.execute(
            prepared,
            ticket,
            &mut f.artifacts("uncertain"),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(10)
        ),
        Err(DomainError::DurabilityUnknown)
    ));
    assert_eq!(
        f.commands().len(),
        1,
        "uncertain install must never be replayed"
    );
}

#[test]
fn settled_package_installations_feed_atomic_resource_reload_without_starting_commands() {
    use ag1_platform::{
        resource_fs::ResourceFs,
        resources::{Config, InstalledPackage, Paths, ResourceLoader},
    };
    use package_lifecycle::Scope as PackageScope;
    let f = Fixture::new();
    let cancel = AtomicBool::new(false);
    let control = || ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(3),
    };
    let settings = f.ws.root().join(".ag1/settings.json");
    let settings_bytes = r#"{"packages":["npm:fixture@2.0.0"]}"#;
    fs::write(&settings, settings_bytes).unwrap();
    let root = WorkspaceService::new()
        .unwrap()
        .register(f.dir.path())
        .unwrap();
    let mut loader = ResourceLoader::new(
        ResourceFs::new(vec![root]).unwrap(),
        Config {
            cwd: f.ws.root().to_owned(),
            agent_dir: f.dir.path().join("agent"),
            home: f.dir.path().to_owned(),
            additional: Paths::default(),
            disabled: vec![],
            installed: vec![],
            system: None,
            append_system: None,
        },
    )
    .unwrap();
    loader.reload(0, &control()).unwrap();
    assert!(loader.snapshot().unwrap().catalog.prompts.is_empty());
    assert!(
        f.service
            .installation_record("npm:fixture@2.0.0", PackageScope::Project, control())
            .unwrap()
            .is_none()
    );
    assert!(f.commands().is_empty());
    assert_eq!(
        f.run(
            "resource-install-old",
            "install",
            "npm:fixture@1.0.0",
            "project"
        )
        .result["isError"],
        false
    );
    assert!(
        f.service
            .installation_record("npm:fixture@2.0.0", PackageScope::Project, control())
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.run(
            "resource-install-current",
            "install",
            "npm:fixture@2.0.0",
            "project"
        )
        .result["isError"],
        false
    );
    let before = f.commands().len();
    let record = f
        .service
        .installation_record("npm:fixture@2.0.0", PackageScope::Project, control())
        .unwrap()
        .unwrap();
    assert_eq!(record.identity, "npm:fixture");
    assert_eq!(record.scope, "project");
    // Owned package contents stand in for files shipped by the fixture package.
    fs::create_dir(record.root.join("prompts")).unwrap();
    fs::write(
        record.root.join("prompts/fixture.md"),
        "Installed fixture prompt.",
    )
    .unwrap();
    loader
        .reload_installed(loader.generation(), vec![record], &control())
        .unwrap();
    assert_eq!(loader.snapshot().unwrap().catalog.prompts.len(), 1);
    let generation = loader.generation();
    let fingerprint = loader.snapshot().unwrap().fingerprint.clone();
    fs::write(&settings, "invalid JSON").unwrap();
    assert!(
        loader
            .reload_installed(generation, vec![], &control())
            .is_err()
    );
    assert_eq!(loader.generation(), generation);
    assert_eq!(loader.snapshot().unwrap().fingerprint, fingerprint);
    fs::write(&settings, settings_bytes).unwrap();
    loader.reload(generation, &control()).unwrap();
    assert_eq!(
        loader.snapshot().unwrap().catalog.prompts.len(),
        1,
        "failed reload must retain its installation map"
    );
    let bad = InstalledPackage {
        source: "npm:fixture@2.0.0".into(),
        scope: "project".into(),
        identity: "wrong".into(),
        root: f.ws.root().join(".ag1/npm/node_modules/fixture"),
    };
    assert!(matches!(
        loader.reload_installed(loader.generation(), vec![bad], &control()),
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        loader.reload_installed(generation, vec![], &control()),
        Err(DomainError::Conflict)
    ));
    loader.reload(loader.generation(), &control()).unwrap();
    assert_eq!(loader.snapshot().unwrap().catalog.prompts.len(), 1);
    assert_eq!(
        f.commands().len(),
        before,
        "inspection/reload must never invoke npm or git"
    );
    let local = f
        .service
        .installation_record(
            f.ws.root().to_str().unwrap(),
            PackageScope::Temporary,
            control(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(local.root, f.ws.root());
}
