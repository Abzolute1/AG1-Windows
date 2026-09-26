use ag1_platform::projects::{Toolchains, environment::trust_bundles};
use std::{fs, path::Path};

#[cfg(target_os = "linux")]
#[path = "support/p14_tls.rs"]
mod tls;

#[test]
fn shared_environment_retains_tool_paths_and_project_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("tool chain/bin");
    fs::create_dir_all(&bin).unwrap();
    for name in ["cargo", "rustc"] {
        fs::write(bin.join(name), "fixture").unwrap();
    }
    let project = tmp.path().join("project");
    fs::create_dir(&project).unwrap();
    let tools = Toolchains {
        node: None,
        npm: None,
        cargo: Some(bin.join("cargo")),
        rustc: Some(bin.join("rustc")),
        godot: None,
        mounts: vec![],
    };
    let env = tools.environment(&project).unwrap();
    assert!(std::env::split_paths(&env["PATH"]).any(|p| p == bin));
    assert_eq!(
        env["CARGO_HOME"],
        project.join("target/ag1-cargo").display().to_string()
    );
    assert_eq!(env["RUSTUP_AUTO_INSTALL"], "0");
    assert!(!env.contains_key("HOME"));
    assert_eq!(env, tools.environment(&project).unwrap());
    fs::remove_file(bin.join("cargo")).unwrap();
    assert!(tools.environment(&project).is_err());
}

#[cfg(unix)]
#[test]
fn trust_bundle_resolves_external_symlinks_and_rejects_broken_stores() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("actual.pem");
    let link = tmp.path().join("ssl/cert.pem");
    fs::create_dir(tmp.path().join("ssl")).unwrap();
    fs::write(&bundle, "fixture CA").unwrap();
    symlink("../actual.pem", &link).unwrap();
    assert_eq!(
        trust_bundles(&[&link, &bundle]).unwrap(),
        vec![bundle.canonicalize().unwrap()]
    );
    assert!(
        trust_bundles(&[Path::new("/nonexistent-p14-trust.pem")])
            .unwrap()
            .is_empty()
    );
    fs::remove_file(&bundle).unwrap();
    assert!(trust_bundles(&[&link]).is_err());
    fs::write(&bundle, "").unwrap();
    assert!(trust_bundles(&[&link]).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn ordinary_cargo_works_in_sandbox_shell_jobs_and_native_runner() {
    use ag1_core::{
        Clock, SessionId,
        tools::{
            approval::{Authorizer, Preflight, PreflightContext},
            policy::Policy,
            registry::{Registry, Scope},
        },
    };
    use ag1_platform::{
        job_tools::JobService,
        process::ProcessSupervisor,
        projects::{Launch, Runtime},
        resource_fs::ScanControl,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
        workspace::WorkspaceService,
    };
    use ag1_storage::{Store, context::ContextStore};
    use serde_json::json;
    use std::{
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };
    struct TestClock;
    impl Clock for TestClock {
        fn now_ms(&self) -> u64 {
            1000
        }
    }
    let real = PathBuf::from(env!("CARGO")).canonicalize().unwrap();
    let mut compilers = vec![real.clone()];
    if let Some(home) = std::env::var_os("HOME") {
        let shim = PathBuf::from(home).join(".cargo/bin/cargo");
        if shim.is_file() {
            compilers.push(shim);
        }
    }
    for cargo in compilers {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("project with spaces");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='p14_probe'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        fs::write(
            root.join("src/main.rs"),
            "fn main() { println!(\"p14\"); }\n#[test] fn meaningful() { assert_eq!(2+2,4); }",
        )
        .unwrap();
        let tools = Toolchains {
            node: None,
            npm: None,
            cargo: Some(cargo.clone()),
            rustc: Some(cargo.with_file_name("rustc")),
            godot: None,
            mounts: vec![real.parent().unwrap().parent().unwrap().to_owned()],
        };
        let ws = WorkspaceService::new().unwrap().register(&root).unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(data.path()).unwrap());
        let timestamp = "2026-09-09T00:00:00.000Z";
        store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":root,"timestamp":timestamp}).to_string().as_bytes()).unwrap();
        store
            .register_origin("fixture", "fixture", &[8; 32])
            .unwrap();
        let spools = tempfile::tempdir().unwrap();
        let supervisor = Arc::new(
            ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap(),
        );
        let shell = Arc::new(
            ShellService::new(
                ws.clone(),
                supervisor,
                spools.path(),
                ShellConfig {
                    executable: "/bin/bash".into(),
                    env: tools.environment(&root).unwrap(),
                    command_prefix: None,
                    command_template: None,
                    sandbox: Sandbox::Strict,
                    bubblewrap: Some("/usr/bin/bwrap".into()),
                    output_bytes: 1024 * 1024,
                    address_space: None,
                },
            )
            .unwrap(),
        );
        let scope = Scope {
            workspace: ws.id().as_str().into(),
            session: "fixture".into(),
            epoch: 1,
            revision: 1,
        };
        let mut registry = Registry::builtins().unwrap();
        let names = vec!["bash".into(), "job".into()];
        registry
            .activate(&names, &names.iter().cloned().collect())
            .unwrap();
        let policy: Policy = serde_json::from_value(json!({"mode":"disabled","cwd":root})).unwrap();
        let authorize = |call, target: &str| {
            let mut book = Authorizer::new("p14").unwrap();
            match book
                .preflight(
                    call,
                    PreflightContext {
                        scope: &scope,
                        policy: &policy,
                        policy_revision: 1,
                        target_revision: target,
                        now: 1000,
                        ttl: 300_000,
                        untrusted_extensions: true,
                    },
                )
                .unwrap()
            {
                Preflight::Ready(ticket) => ticket,
                _ => panic!("fixture authorization"),
            }
        };
        let cancel = AtomicBool::new(false);
        let execution = || ShellExecution {
            registry: &registry,
            scope: &scope,
            policy_revision: 1,
            now: || 1000,
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        let mut artifacts = ContextStore::new(
            &store,
            SessionId::new("fixture").unwrap(),
            "fixture",
            "p14",
            timestamp,
            Default::default(),
        )
        .unwrap();
        let call = registry
            .prepare(
                "shell",
                "shell",
                "bash",
                &json!({"command":"cargo test --offline"}),
                true,
            )
            .unwrap();
        let plan = shell.prepare(&call, &registry, &scope).unwrap();
        let ticket = authorize(plan.call().clone(), plan.target_revision());
        let result = shell
            .execute(plan, ticket, execution(), &mut artifacts, |_| {})
            .unwrap();
        assert_ne!(
            result.result["isError"], true,
            "compiler={cargo:?}: {}",
            result.result
        );
        let jobs = JobService::new(
            store.clone(),
            shell.clone(),
            Arc::new(TestClock),
            scope.clone(),
            "p14-jobs",
        )
        .unwrap();
        let call = registry
            .prepare(
                "job",
                "job",
                "job",
                &json!({"action":"start","command":"cargo build --offline"}),
                true,
            )
            .unwrap();
        let plan = shell.prepare_job(&call, &registry, &scope).unwrap();
        let ticket = authorize(plan.call().clone(), jobs.target_revision());
        let result = jobs
            .execute(Some(plan), ticket, execution(), &mut artifacts)
            .unwrap();
        assert_ne!(result.result["isError"], true, "{}", result.result);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            jobs.refresh(&mut artifacts).unwrap();
            let records = jobs.client_records().unwrap();
            if records["jobs"][0]["status"] == "exited" {
                assert_eq!(records["jobs"][0]["exitCode"], 0, "{records}");
                break;
            }
            assert!(Instant::now() < deadline, "{records}");
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::load(
            ws,
            WorkspaceService::new()
                .unwrap()
                .register(state.path())
                .unwrap(),
            env!("CARGO_BIN_EXE_ag1-process-host").as_ref(),
            tools,
        )
        .unwrap();
        let target = runtime
            .project
            .targets
            .iter()
            .find(|t| t.kind == ag1_core::projects::Kind::Rust)
            .unwrap()
            .id
            .clone();
        let mut commands = runtime
            .command(ag1_core::projects::Kind::Rust, "check")
            .unwrap();
        commands[0].argv.push("--offline".into());
        let control = ScanControl {
            cancel: &cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        };
        runtime
            .start(
                &target,
                "check",
                Launch {
                    commands,
                    readiness: None,
                    timeout_ms: 30_000,
                },
                &control,
            )
            .unwrap();
        let record = runtime.finish(&control).unwrap();
        assert_eq!(
            record.exits,
            vec![Some(0)],
            "compiler={cargo:?}: {:?}",
            record.diagnostics
        );
    }
}
