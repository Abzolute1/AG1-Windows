#![cfg(target_os = "linux")]
use ag1_core::{
    DomainError,
    projects::{Kind, Lifecycle},
};
use ag1_platform::{
    projects::{Command, Launch, Runtime, Toolchains},
    resource_fs::ScanControl,
    workspace::WorkspaceService,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
struct Fixture {
    _dir: tempfile::TempDir,
    runtime: Runtime,
}
fn control(cancel: &AtomicBool) -> ScanControl<'_> {
    ScanControl {
        cancel,
        deadline: Instant::now() + Duration::from_secs(20),
    }
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let state = dir.path().join("state");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&state).unwrap();
        let service = WorkspaceService::new().unwrap();
        let runtime = Runtime::load(
            service.register(&root).unwrap(),
            service.register(&state).unwrap(),
            std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            Toolchains {
                node: None,
                npm: None,
                cargo: None,
                rustc: None,
                godot: None,
                mounts: vec![],
            },
        )
        .unwrap();
        Self { _dir: dir, runtime }
    }
    fn run(&mut self, script: &str) -> String {
        let target = self
            .runtime
            .project
            .targets
            .iter()
            .find(|t| t.kind == Kind::Native)
            .unwrap()
            .id
            .clone();
        let cancel = AtomicBool::new(false);
        self.runtime
            .start(
                &target,
                "run",
                Launch {
                    commands: vec![Command {
                        executable: PathBuf::from("/bin/sh"),
                        argv: vec!["-c".into(), script.into()],
                        cwd: "".into(),
                    }],
                    readiness: None,
                    timeout_ms: 10000,
                },
                &control(&cancel),
            )
            .unwrap()
    }
}
#[test]
fn native_runs_retains_streams_and_exit() {
    let mut f = Fixture::new();
    let id = f.run("echo stdout; echo stderr >&2");
    let c = AtomicBool::new(false);
    let result = f.runtime.finish(&control(&c)).unwrap();
    assert_eq!(result.lifecycle, Lifecycle::Exited);
    assert_eq!(
        f.runtime
            .project
            .targets
            .iter()
            .find(|t| t.id == result.target)
            .unwrap()
            .lifecycle,
        Lifecycle::Exited
    );
    assert_eq!(result.exits, vec![Some(0)]);
    assert_eq!(f.runtime.log(&id, 0, 0, 32768).unwrap(), b"stdout\n");
    assert_eq!(f.runtime.log(&id, 1, 0, 32768).unwrap(), b"stderr\n");
}
#[test]
fn failure_is_not_runtime_or_compile_success() {
    let mut f = Fixture::new();
    f.run("echo 'error: deliberate failure' >&2; exit 7");
    let c = AtomicBool::new(false);
    let result = f.runtime.finish(&control(&c)).unwrap();
    assert_eq!(result.lifecycle, Lifecycle::Failed);
    assert_eq!(result.exits, vec![Some(7)]);
    assert!(!result.diagnostics.is_empty());
}
#[test]
fn repeated_stop_is_idempotent_and_no_host_secrets() {
    let mut f = Fixture::new();
    for _ in 0..3 {
        f.run("test ! -e /home/alex/.ag1; test -z \"$AG1_LLAMA_SERVER\"; echo isolated; sleep 30 & wait");
        let c = AtomicBool::new(false);
        for _ in 0..10 {
            f.runtime.poll(&control(&c)).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(f.runtime.stop().unwrap().is_some());
        assert!(f.runtime.stop().unwrap().is_none());
    }
}
#[test]
fn cancellation_stops_owned_group() {
    let mut f = Fixture::new();
    f.run("sleep 30 & wait");
    let c = AtomicBool::new(true);
    assert!(matches!(
        f.runtime.poll(&control(&c)),
        Err(DomainError::Cancelled)
    ));
    assert!(f.runtime.stop().unwrap().is_none());
}
#[test]
fn rename_and_delete_stop_without_adopting_new_path() {
    for delete in [false, true] {
        let mut f = Fixture::new();
        f.run("sleep 30 & wait");
        let path = f.runtime.workspace.root().to_path_buf();
        if delete {
            std::fs::remove_dir_all(&path).unwrap();
        } else {
            std::fs::rename(&path, path.with_file_name("renamed")).unwrap();
        }
        std::fs::create_dir(&path).unwrap();
        let c = AtomicBool::new(false);
        assert!(f.runtime.poll(&control(&c)).is_err());
        assert!(f.runtime.stop().unwrap().is_none());
    }
}
#[test]
fn create_failure_cleans_only_owned_destination() {
    let dir = tempfile::tempdir().unwrap();
    let parent = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let c = AtomicBool::new(false);
    assert!(
        Runtime::create(
            &parent,
            "new",
            &BTreeMap::from([("../escape".into(), b"bad".to_vec())]),
            &control(&c)
        )
        .is_err()
    );
    assert!(!dir.path().join("new").exists());
    std::fs::create_dir(dir.path().join("existing")).unwrap();
    assert!(
        Runtime::create(
            &parent,
            "existing",
            &BTreeMap::from([("a".into(), vec![1])]),
            &control(&c)
        )
        .is_err()
    );
    assert!(dir.path().join("existing").is_dir());
}
#[test]
fn target_discovery_does_not_scan_unrelated_archived_environments() {
    let mut f = Fixture::new();
    let root = f.runtime.workspace.root();
    std::fs::write(
        root.join("package.json"),
        r#"{"scripts":{"build":"build"}}"#,
    )
    .unwrap();
    std::fs::create_dir(root.join("archive")).unwrap();
    for n in 0..1001 {
        std::fs::write(root.join(format!("archive/{n}")), "retained").unwrap();
    }
    assert!(f.runtime.workspace.scan(1000).is_err());
    f.runtime.discover().unwrap();
    assert!(
        f.runtime
            .project
            .targets
            .iter()
            .any(|t| t.kind == Kind::Web)
    );
    // Discovery does not weaken the existing execution revision boundary.
    assert!(f.runtime.revision().is_err());
    let root = f.runtime.workspace.root().to_path_buf();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join(".gitignore"), "archive/\n").unwrap();
    std::fs::write(root.join("main.rs"), "original source").unwrap();
    std::fs::create_dir(root.join("source")).unwrap();
    for n in 0..1001 {
        std::fs::write(root.join(format!("source/{n}")), "source").unwrap();
    }
    std::fs::write(root.join("large-source"), vec![7; 17 * 1024 * 1024]).unwrap();
    std::fs::write(root.join("deleted-source"), "tracked").unwrap();
    assert!(
        std::process::Command::new("git")
            .args(["add", "deleted-source"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::remove_file(root.join("deleted-source")).unwrap();
    f.runtime
        .bind_git_source(
            std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            std::path::Path::new("/usr/bin/git"),
        )
        .unwrap();
    let before = f.runtime.revision().unwrap();
    std::fs::write(root.join("archive/0"), "ignored cache change").unwrap();
    assert_eq!(f.runtime.revision().unwrap(), before);
    std::fs::write(root.join("main.rs"), "changed source").unwrap();
    assert_ne!(f.runtime.revision().unwrap(), before);
    let before = f.runtime.revision().unwrap();
    std::fs::write(root.join("deleted-source"), "restored").unwrap();
    assert_ne!(f.runtime.revision().unwrap(), before);
}

#[test]
fn unavailable_toolchain_and_preview_policy_are_actionable() {
    let mut f = Fixture::new();
    f.runtime
        .workspace
        .create_checked(
            "package.json",
            br#"{"scripts":{"dev":"vite","build":"vite build"}}"#,
            false,
            || Ok(()),
        )
        .unwrap();
    f.runtime.discover().unwrap();
    assert!(
        !f.runtime
            .project
            .targets
            .iter()
            .find(|t| t.kind == Kind::Web)
            .unwrap()
            .capabilities["run"]
            .available
    );
    assert!(matches!(
        f.runtime.command(Kind::Web, "run"),
        Err(DomainError::Unavailable(_))
    ));
    let mut grant =
        ag1_platform::projects::preview::Grant::new("run1", "session1", "http://127.0.0.1:38123/")
            .unwrap();
    assert!(grant.allows("run1", "http://127.0.0.1:38123/route"));
    for url in [
        "file:///etc/passwd",
        "http://127.0.0.1:38124/",
        "https://evil.test/",
        "http://user@127.0.0.1:38123/",
    ] {
        assert!(!grant.allows("run1", url));
    }
    grant.close();
    assert!(!grant.allows("run1", "http://127.0.0.1:38123/"));
}
#[test]
fn foreign_listener_is_rejected_and_survives() {
    let mut f = Fixture::new();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let c = AtomicBool::new(false);
    let target = f.runtime.project.targets[0].id.clone();
    let result = f.runtime.start(
        &target,
        "run",
        Launch {
            commands: vec![Command {
                executable: "/bin/true".into(),
                argv: vec![],
                cwd: "".into(),
            }],
            timeout_ms: 1000,
            readiness: Some(ag1_platform::projects::ReadyHttp {
                address: listener.local_addr().unwrap(),
                path: "/".into(),
                expected_body: "nonce".into(),
            }),
        },
        &control(&c),
    );
    assert!(matches!(result, Err(DomainError::Conflict)));
    assert!(std::net::TcpStream::connect(listener.local_addr().unwrap()).is_ok());
}
#[test]
fn stale_readiness_never_certifies_an_owned_run() {
    let mut f = Fixture::new();
    f.runtime.workspace.create_checked("serve.py",b"import http.server\nclass H(http.server.BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200);self.end_headers();self.wfile.write(b'old-nonce')\nhttp.server.HTTPServer(('127.0.0.1',int(__import__('sys').argv[1])),H).serve_forever()\n",false,||Ok(())).unwrap();
    let c = AtomicBool::new(false);
    let target = f.runtime.project.targets[0].id.clone();
    // The OS may reuse a released ephemeral port for another concurrent
    // fixture. Retry only rejected, unadmitted launches on a new port.
    let address = (0..8)
        .find_map(|_| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            let result = f.runtime.start(
                &target,
                "run",
                Launch {
                    commands: vec![Command {
                        executable: "/usr/bin/python3".into(),
                        argv: vec!["serve.py".into(), address.port().to_string()],
                        cwd: "".into(),
                    }],
                    timeout_ms: 10000,
                    readiness: Some(ag1_platform::projects::ReadyHttp {
                        address,
                        path: "/".into(),
                        expected_body: "new-nonce".into(),
                    }),
                },
                &control(&c),
            );
            match result {
                Ok(_) => Some(address),
                Err(DomainError::Conflict) => {
                    assert!(
                        f.runtime.history().unwrap().is_empty(),
                        "An admitted operation must never be retried"
                    );
                    None
                }
                Err(error) => panic!("{error:?}"),
            }
        })
        .expect("Could not reserve an unused fixture port");

    for _ in 0..15 {
        let result = f.runtime.poll(&control(&c)).unwrap();
        assert_ne!(result.lifecycle, Lifecycle::Ready);
        std::thread::sleep(Duration::from_millis(10));
    }
    f.runtime.stop().unwrap();
    assert!(std::net::TcpListener::bind(address).is_ok());
}
#[test]
fn load_preserves_identity_and_reports_uncertain_history() {
    let mut f = Fixture::new();
    let project = f.runtime.project.id.clone();
    let session = f.runtime.project.session.clone();
    f.run("sleep 30");
    let state = WorkspaceService::new()
        .unwrap()
        .register(&f._dir.path().join("state"))
        .unwrap();
    let workspace = f.runtime.workspace.clone();
    let tools = f.runtime.toolchains.clone();
    assert!(matches!(
        Runtime::load(
            workspace.clone(),
            state.clone(),
            std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            tools.clone()
        ),
        Err(DomainError::Conflict)
    ));
    drop(f.runtime);
    let runtime = Runtime::load(
        workspace,
        state,
        std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        tools,
    )
    .unwrap();
    assert_eq!(runtime.project.id, project);
    assert_eq!(runtime.project.session, session);
    assert_eq!(
        runtime.history().unwrap()[0].lifecycle,
        Lifecycle::RecoveryRequired
    );
    assert!(
        runtime
            .project
            .targets
            .iter()
            .any(|t| t.lifecycle == Lifecycle::RecoveryRequired)
    );
}

#[test]
fn project_writer_release_is_not_delayed_by_inherited_file_descriptions() {
    use std::os::fd::{FromRawFd, OwnedFd};
    let f = Fixture::new();
    let state = WorkspaceService::new()
        .unwrap()
        .register(&f._dir.path().join("state"))
        .unwrap();
    let lock_path = state.root().join("project-writer.lock");
    let descriptor = std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| std::fs::read_link(entry.path()).ok().as_ref() == Some(&lock_path))
        .unwrap();
    let descriptor: i32 = descriptor.file_name().to_str().unwrap().parse().unwrap();
    // dup and fork retain the same open-file description. Keep a duplicate to
    // model another thread's pre-exec child without timing-dependent sleeps.
    let duplicate = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 3) };
    assert!(duplicate >= 0);
    let inherited = unsafe { OwnedFd::from_raw_fd(duplicate) };
    let workspace = f.runtime.workspace.clone();
    let tools = f.runtime.toolchains.clone();
    assert!(matches!(
        Runtime::load(
            workspace.clone(),
            state.clone(),
            std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            tools.clone()
        ),
        Err(DomainError::Conflict)
    ));
    drop(f.runtime);
    let resumed = Runtime::load(
        workspace,
        state,
        std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        tools,
    );
    assert!(
        resumed.is_ok(),
        "A dropped writer must release its flock even while an inherited descriptor remains open"
    );
    drop(inherited);
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}
fn long_control(cancel: &AtomicBool) -> ScanControl<'_> {
    ScanControl {
        cancel,
        deadline: Instant::now() + Duration::from_secs(180),
    }
}
fn node_tools(runtime: &mut Runtime) {
    let root = repo().join(".local/toolchains/node-v24.14.0-linux-x64");
    runtime.toolchains.node = Some(root.join("bin/node"));
    runtime.toolchains.npm = Some(root.join("lib/node_modules/npm/bin/npm-cli.js"));
    runtime.toolchains.mounts.push(root);
}
fn fixture_files(runtime: &Runtime, fixture: &str, paths: &[&str]) {
    for p in paths {
        let bytes = std::fs::read(repo().join("fixtures/projects").join(fixture).join(p)).unwrap();
        runtime
            .workspace
            .create_checked(p, &bytes, true, || Ok(()))
            .unwrap();
    }
}
#[test]
fn real_rust_build_run_compile_error_and_crash() {
    let mut f = Fixture::new();
    fixture_files(&f.runtime, "rust", &["Cargo.toml", "src/main.rs"]);
    let cargo = PathBuf::from(env!("CARGO")).canonicalize().unwrap();
    let root = cargo.parent().unwrap().parent().unwrap().to_path_buf();
    f.runtime.toolchains.cargo = Some(cargo);
    f.runtime.toolchains.rustc = Some(root.join("bin/rustc"));
    f.runtime.toolchains.mounts.push(root);
    f.runtime.discover().unwrap();
    let target = f
        .runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == Kind::Rust)
        .unwrap()
        .id
        .clone();
    let c = AtomicBool::new(false);
    for action in ["install", "build", "run"] {
        f.runtime
            .start(
                &target,
                action,
                Launch {
                    commands: f.runtime.command(Kind::Rust, action).unwrap(),
                    readiness: None,
                    timeout_ms: 60000,
                },
                &long_control(&c),
            )
            .unwrap();
        let r = f.runtime.finish(&long_control(&c)).unwrap();
        capture_operation(&f.runtime, &r, &format!("rust-{action}"));
        assert_eq!(r.exits, vec![Some(0)], "{action}: {:?}", r.diagnostics);
        assert_eq!(r.lifecycle, Lifecycle::Exited);
    }
    let mut command = f.runtime.command(Kind::Rust, "run").unwrap();
    command[0].argv.extend(["--".into(), "--crash".into()]);
    f.runtime
        .start(
            &target,
            "run",
            Launch {
                commands: command,
                readiness: None,
                timeout_ms: 60000,
            },
            &long_control(&c),
        )
        .unwrap();
    assert_eq!(
        f.runtime.finish(&long_control(&c)).unwrap().lifecycle,
        Lifecycle::Failed
    );
    let old = f.runtime.workspace.snapshot("src/main.rs", 65536).unwrap();
    f.runtime
        .workspace
        .replace(
            "src/main.rs",
            &old.revision,
            b"fn main() { deliberate syntax failure }",
        )
        .unwrap();
    f.runtime
        .start(
            &target,
            "check",
            Launch {
                commands: f.runtime.command(Kind::Rust, "check").unwrap(),
                readiness: None,
                timeout_ms: 60000,
            },
            &long_control(&c),
        )
        .unwrap();
    let r = f.runtime.finish(&long_control(&c)).unwrap();
    assert_eq!(r.lifecycle, Lifecycle::Failed);
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.file.as_deref() == Some("src/main.rs"))
    );
}
#[test]
fn real_vite_install_build_interactive_browser_and_failures() {
    let mut f = Fixture::new();
    fixture_files(
        &f.runtime,
        "web",
        &["package.json", "package-lock.json", "index.html", "main.js"],
    );
    node_tools(&mut f.runtime);
    f.runtime.discover().unwrap();
    let target = f
        .runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == Kind::Web)
        .unwrap()
        .id
        .clone();
    let c = AtomicBool::new(false);
    for action in ["install", "build"] {
        f.runtime
            .start(
                &target,
                action,
                Launch {
                    commands: f.runtime.command(Kind::Web, action).unwrap(),
                    readiness: None,
                    timeout_ms: 120000,
                },
                &long_control(&c),
            )
            .unwrap();
        let r = f.runtime.finish(&long_control(&c)).unwrap();
        capture_operation(&f.runtime, &r, &format!("web-{action}"));
        assert_eq!(
            r.lifecycle,
            Lifecycle::Exited,
            "{action}: {:?}",
            r.diagnostics
        );
    }
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);
    let nonce = format!("p11-{}", f.runtime.project.id);
    f.runtime
        .workspace
        .create_checked("public/ready.txt", nonce.as_bytes(), true, || Ok(()))
        .unwrap();
    let mut commands = f.runtime.command(Kind::Web, "run").unwrap();
    commands[0]
        .argv
        .extend(["--".into(), "--port".into(), address.port().to_string()]);
    f.runtime
        .start(
            &target,
            "run",
            Launch {
                commands,
                readiness: Some(ag1_platform::projects::ReadyHttp {
                    address,
                    path: "/ready.txt".into(),
                    expected_body: nonce,
                }),
                timeout_ms: 120000,
            },
            &long_control(&c),
        )
        .unwrap();
    let ready = loop {
        let r = f.runtime.poll(&long_control(&c)).unwrap();
        assert!(
            !matches!(r.lifecycle, Lifecycle::Failed | Lifecycle::Exited),
            "{:?}",
            r.diagnostics
        );
        if r.lifecycle == Lifecycle::Ready {
            break r;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    capture_operation(&f.runtime, &ready, "web-ready");
    assert!(ready.ready_ms.is_some());
    let grant = f.runtime.preview_grant().unwrap();
    let browser = ag1_platform::projects::preview::Browser {
        node: f.runtime.toolchains.node.clone().unwrap(),
        driver: repo().join("product/compat/project-browser/driver.mjs"),
        playwright: repo().join("reference/ag1-executable/node_modules/playwright/index.js"),
        executable: "/home/alex/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome".into(),
        helper: env!("CARGO_BIN_EXE_ag1-process-host").into(),
        state: f._dir.path().join("state"),
    };
    let observation=browser.observe(&grant,&serde_json::json!({"url":format!("{}/",grant.origin),"actions":[{"type":"click","selector":"#increment"},{"type":"click","selector":"#crash"}]}),&long_control(&c)).unwrap();
    assert!(observation.result["text"].as_str().unwrap().contains('1'));
    assert!(
        observation.result["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e
                .as_str()
                .unwrap()
                .contains("deliberate frontend runtime failure"))
    );
    if let Some(directory) = std::env::var_os("AG1_P11_EVIDENCE") {
        let directory = PathBuf::from(directory);
        std::fs::write(directory.join("browser.png"), &observation.png).unwrap();
        std::fs::write(directory.join("browser.json"),serde_json::to_vec_pretty(&serde_json::json!({"observation":observation.result,"elapsed_ms":observation.elapsed_ms,"browser_rss_bytes":observation.rss_bytes,"target_rss_bytes":ready.target_rss_bytes,"total_rss_bytes":observation.rss_bytes+ready.target_rss_bytes})).unwrap()).unwrap();
    }
    assert!(!observation.png.is_empty());
    assert!(observation.rss_bytes > 0);
    let stop = f.runtime.stop().unwrap().unwrap();
    capture_operation(&f.runtime, &stop, "web-stopped");
    assert!(stop.stop_ms.is_some());
    assert!(f.runtime.verify_preview(&grant).is_err());
    assert!(std::net::TcpListener::bind(address).is_ok());
    let old = f.runtime.workspace.snapshot("main.js", 65536).unwrap();
    f.runtime
        .workspace
        .replace(
            "main.js",
            &old.revision,
            b"const = deliberate syntax failure",
        )
        .unwrap();
    f.runtime
        .start(
            &target,
            "build",
            Launch {
                commands: f.runtime.command(Kind::Web, "build").unwrap(),
                readiness: None,
                timeout_ms: 60000,
            },
            &long_control(&c),
        )
        .unwrap();
    assert_eq!(
        f.runtime.finish(&long_control(&c)).unwrap().lifecycle,
        Lifecycle::Failed
    );
}
#[test]
fn git_version_preview_guards_dirty_head_conflict_and_preserves_session() {
    let mut f = Fixture::new();
    let git_root = f.runtime.workspace.root().to_path_buf();
    let run = |args: &[&str]| {
        let output = std::process::Command::new("/usr/bin/git")
            .arg("-c")
            .arg("core.hooksPath=/dev/null")
            .args(args)
            .current_dir(&git_root)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    run(&["init", "-q"]);
    run(&["config", "user.name", "Fixture"]);
    run(&["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(f.runtime.workspace.root().join("file"), "old").unwrap();
    run(&["add", "file"]);
    run(&["commit", "-qm", "old"]);
    let old = run(&["rev-parse", "HEAD"]);
    std::fs::write(f.runtime.workspace.root().join("file"), "new").unwrap();
    run(&["commit", "-qam", "new"]);
    let head = run(&["rev-parse", "HEAD"]);
    let git = ag1_platform::git::GitService::new(
        std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        std::path::Path::new("/usr/bin/git"),
        &f._dir.path().join("state"),
    )
    .unwrap();
    let preview = git
        .preview_version(&f.runtime.workspace, "file", &head, &old)
        .unwrap();
    assert_eq!(preview.candidate, b"old");
    assert_eq!(git.history(&f.runtime.workspace, 10).unwrap().len(), 2);
    std::fs::write(f.runtime.workspace.root().join("other"), "dirty").unwrap();
    assert!(matches!(
        git.apply_version(&f.runtime.workspace, &preview),
        Err(DomainError::Conflict)
    ));
    std::fs::remove_file(f.runtime.workspace.root().join("other")).unwrap();
    f.runtime.restore_version(&git, &preview).unwrap();
    assert!(f._dir.path().join("state/git-recovery-1.json").is_file());
    assert_eq!(run(&["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read(f.runtime.workspace.root().join("file")).unwrap(),
        b"old"
    );
    assert!(
        git.preview_version(&f.runtime.workspace, "file", &head, &old)
            .is_err()
    );
}
#[test]
fn real_tauri_webkit_render_interaction_and_native_crash() {
    let mut f = Fixture::new();
    let c = AtomicBool::new(false);
    f.runtime
        .open_private_display(
            &repo().join(".local/p11/xvfb/usr/bin/Xvfb"),
            &long_control(&c),
        )
        .unwrap();
    let executable = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/project-native-fixture");
    assert!(
        executable.is_file(),
        "Build project-native-fixture with --features visual-test first"
    );
    std::fs::create_dir_all(f.runtime.workspace.root().join("target")).unwrap();
    std::fs::copy(
        &executable,
        f.runtime.workspace.root().join("target/native-fixture"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        f.runtime.workspace.root().join("target/native-fixture"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let target = f
        .runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == Kind::Native)
        .unwrap()
        .id
        .clone();
    for crash in [false, true] {
        let id = f
            .runtime
            .start(
                &target,
                "run",
                Launch {
                    commands: vec![Command {
                        executable: f.runtime.workspace.root().join("target/native-fixture"),
                        argv: if crash {
                            vec!["--crash".into()]
                        } else {
                            vec![]
                        },
                        cwd: "".into(),
                    }],
                    readiness: None,
                    timeout_ms: 30000,
                },
                &long_control(&c),
            )
            .unwrap();
        let result = f.runtime.finish(&long_control(&c)).unwrap();
        capture_operation(
            &f.runtime,
            &result,
            if crash {
                "tauri-crash"
            } else {
                "tauri-rendered"
            },
        );
        if let Some(directory) = std::env::var_os("AG1_P11_EVIDENCE") {
            std::fs::copy(
                f.runtime.workspace.root().join("native-capture.png"),
                PathBuf::from(directory).join(if crash {
                    "tauri-crash.png"
                } else {
                    "tauri.png"
                }),
            )
            .unwrap();
        }
        let output = String::from_utf8(f.runtime.log(&id, 0, 0, 32768).unwrap()).unwrap();
        let stderr = String::from_utf8(f.runtime.log(&id, 1, 0, 32768).unwrap()).unwrap();
        assert!(
            output.contains("P11_NATIVE_RENDERED"),
            "stdout={output} stderr={stderr}"
        );
        assert!(output.contains("\"count\":\"1\""));
        assert!(output.contains("P11_NATIVE_CAPTURE_RETAINED"));
        assert_eq!(
            result.lifecycle,
            if crash {
                Lifecycle::Failed
            } else {
                Lifecycle::Exited
            }
        );
        assert!(
            f.runtime
                .workspace
                .snapshot("native-capture.png", 8 * 1024 * 1024)
                .unwrap()
                .bytes
                .len()
                > 100
        );
    }
    f.runtime.close_private_display().unwrap();
}

fn capture_operation(runtime: &Runtime, record: &ag1_core::projects::Operation, name: &str) {
    if let Some(directory) = std::env::var_os("AG1_P11_EVIDENCE") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join(format!("{name}.json")),
            serde_json::to_vec_pretty(record).unwrap(),
        )
        .unwrap();
        for n in 0..record.logs.len() {
            let mut bytes = vec![];
            loop {
                let page = runtime
                    .log(&record.id, n, bytes.len() as u64, 32768)
                    .unwrap();
                if page.is_empty() {
                    break;
                }
                bytes.extend(page);
            }
            std::fs::write(directory.join(format!("{name}-{n}.log")), bytes).unwrap();
        }
    }
}
#[test]
fn tauri_discovery_binds_frontend_cargo_config_and_group_failure() {
    let mut f = Fixture::new();
    fixture_files(
        &f.runtime,
        "tauri",
        &[
            "src-tauri/Cargo.toml",
            "src-tauri/Cargo.lock",
            "src-tauri/tauri.conf.json",
            "src-tauri/src/main.rs",
            "src-tauri/build.rs",
            "src-tauri/icons/icon.png",
        ],
    );
    fixture_files(&f.runtime, "web", &["package.json"]);
    node_tools(&mut f.runtime);
    f.runtime.toolchains.cargo = Some(PathBuf::from(env!("CARGO")).canonicalize().unwrap());
    f.runtime.discover().unwrap();
    let target = f
        .runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == Kind::Tauri)
        .unwrap();
    assert!(target.manifests.contains(&"src-tauri/Cargo.toml".into()));
    assert!(target.manifests.contains(&"package.json".into()));
    let target = target.id.clone();
    let commands = f.runtime.command(Kind::Tauri, "run").unwrap();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[1].cwd, "src-tauri");
    let c = AtomicBool::new(false);
    f.runtime
        .start(
            &target,
            "run",
            Launch {
                commands: vec![
                    Command {
                        executable: "/bin/sh".into(),
                        argv: vec!["-c".into(), "sleep 30 & wait".into()],
                        cwd: "".into(),
                    },
                    Command {
                        executable: "/bin/sh".into(),
                        argv: vec![
                            "-c".into(),
                            "sleep .1; echo frontend-error >&2; exit 3".into(),
                        ],
                        cwd: "".into(),
                    },
                ],
                readiness: None,
                timeout_ms: 10000,
            },
            &control(&c),
        )
        .unwrap();
    let pids = f.runtime.observed_pids();
    assert_eq!(
        f.runtime.finish(&control(&c)).unwrap().lifecycle,
        Lifecycle::Failed
    );
    for pid in pids {
        assert!(ag1_platform::process::observe(pid).is_err());
    }
}
#[test]
fn output_quota_fails_and_stops_owned_children() {
    let mut f = Fixture::new();
    f.run("head -c 9000000 /dev/zero; sleep 30");
    let c = AtomicBool::new(false);
    let pids = f.runtime.observed_pids();
    assert!(f.runtime.finish(&control(&c)).is_err());
    for pid in pids {
        assert!(ag1_platform::process::observe(pid).is_err());
    }
    assert!(f.runtime.stop().unwrap().is_none());
}
#[test]
fn compiler_detection_and_generic_diagnostics_are_explicit() {
    let f = Fixture::new();
    assert!(
        ag1_platform::projects::checks::detect(&f.runtime.workspace, &f.runtime.toolchains)
            .is_err()
    );
    f.runtime
        .workspace
        .create_checked("CMakeLists.txt", b"project(fixture)", false, || Ok(()))
        .unwrap();
    let error = ag1_platform::projects::checks::detect(&f.runtime.workspace, &f.runtime.toolchains)
        .unwrap_err();
    assert!(matches!(error,DomainError::Unavailable(ref s) if s.contains("CMake build directory")));
    assert!(!ag1_core::projects::platform_capability("windows").available);
    assert!(!ag1_core::projects::platform_capability("macos").available);
    assert!(ag1_core::projects::platform_capability("linux").available);
}

#[test]
fn real_tauri_frontend_cargo_install_build_and_dev_group() {
    let mut f = Fixture::new();
    let c = AtomicBool::new(false);
    node_tools(&mut f.runtime);
    fixture_files(
        &f.runtime,
        "tauri",
        &[
            "src-tauri/Cargo.toml",
            "src-tauri/Cargo.lock",
            "src-tauri/build.rs",
            "src-tauri/icons/icon.png",
            "src-tauri/src/main.rs",
            "src-tauri/tauri.conf.json",
        ],
    );
    fixture_files(
        &f.runtime,
        "web",
        &["package.json", "package-lock.json", "index.html", "main.js"],
    );
    let source = f.runtime.workspace.snapshot("main.js", 32768).unwrap();
    let mut script = source.bytes.clone();
    script.extend_from_slice(b"\nsetTimeout(()=>{document.querySelector('button').click();window.__TAURI__.core.invoke('fixture_done',{count:document.querySelector('#count').textContent.trim()})},1500);\n");
    f.runtime
        .workspace
        .replace_checked("main.js", &source.revision, &script, || Ok(()))
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let snapshot = f
        .runtime
        .workspace
        .snapshot("src-tauri/tauri.conf.json", 32768)
        .unwrap();
    let mut config: serde_json::Value = serde_json::from_slice(&snapshot.bytes).unwrap();
    config["build"]["devUrl"] = format!("http://127.0.0.1:{port}").into();
    f.runtime
        .workspace
        .replace_checked(
            "src-tauri/tauri.conf.json",
            &snapshot.revision,
            &serde_json::to_vec(&config).unwrap(),
            || Ok(()),
        )
        .unwrap();
    let cargo = PathBuf::from(env!("CARGO")).canonicalize().unwrap();
    let root = cargo.parent().unwrap().parent().unwrap().to_path_buf();
    f.runtime.toolchains.cargo = Some(cargo);
    f.runtime.toolchains.rustc = Some(root.join("bin/rustc"));
    f.runtime.toolchains.mounts.push(root);
    // Only public, installed crate material is admitted, never Cargo credentials/config.
    let registry = PathBuf::from("/home/alex/.cargo/registry");
    f.runtime.toolchains.mounts.push(registry.clone());
    std::fs::create_dir_all(f.runtime.workspace.root().join("target/ag1-cargo")).unwrap();
    std::os::unix::fs::symlink(
        registry,
        f.runtime.workspace.root().join("target/ag1-cargo/registry"),
    )
    .unwrap();
    f.runtime.discover().unwrap();
    f.runtime
        .open_private_display(
            &repo().join(".local/p11/xvfb/usr/bin/Xvfb"),
            &long_control(&c),
        )
        .unwrap();
    let target = f
        .runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == Kind::Tauri)
        .unwrap()
        .id
        .clone();
    for action in ["install", "build", "run"] {
        let mut commands = f.runtime.command(Kind::Tauri, action).unwrap();
        commands[1].argv.push("--offline".into());
        let id = f
            .runtime
            .start(
                &target,
                action,
                Launch {
                    commands,
                    readiness: None,
                    timeout_ms: 600000,
                },
                &ScanControl {
                    cancel: &c,
                    deadline: Instant::now() + Duration::from_secs(660),
                },
            )
            .unwrap();
        let result = f
            .runtime
            .finish(&ScanControl {
                cancel: &c,
                deadline: Instant::now() + Duration::from_secs(660),
            })
            .unwrap();
        capture_operation(&f.runtime, &result, &format!("tauri-dev-{action}"));
        let stderr = String::from_utf8_lossy(&f.runtime.log(&id, 3, 0, 32768).unwrap()).to_string();
        assert_eq!(result.lifecycle, Lifecycle::Exited, "{action}: {stderr}");
        assert_eq!(result.exits[1], Some(0), "{action}: {stderr}");
        if action == "run" {
            let mut full = vec![];
            loop {
                let page = f.runtime.log(&id, 2, full.len() as u64, 32768).unwrap();
                if page.is_empty() {
                    break;
                }
                full.extend(page);
            }
            let stdout = String::from_utf8_lossy(&full);
            assert!(
                stdout.contains("P11_TAURI_DEV_INTERACTION count=1"),
                "{stdout} {stderr}"
            );
        }
    }
    f.runtime.close_private_display().unwrap();
}

#[test]
fn structured_and_text_diagnostics_do_not_misclassify_cargo_artifacts() {
    let values = ag1_platform::projects::parse_diagnostics(b"src/file.ts(2,3): error TS2322: wrong type\nmain.go:4:5: missing name\nCompiling thiserror v1.0\n{\"reason\":\"compiler-artifact\",\"package_id\":\"thiserror\"}\n", "compiler");
    assert!(
        ag1_platform::projects::parse_diagnostics(
            br#"127.0.0.1 - - [07/Sep/2026 03:44:44] "GET /error HTTP/1.0" 200 -"#,
            "process"
        )
        .is_empty()
    );
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].file.as_deref(), Some("src/file.ts"));
    assert_eq!(values[0].column, Some(3));
    assert_eq!(values[0].code.as_deref(), Some("TS2322"));
    assert_eq!(values[1].line, Some(4));
}

#[test]
fn model_preview_lifecycle_binds_source_and_operation_and_returns_only_owned_ready_urls() {
    use serde_json::json;
    let mut f = Fixture::new();
    let cancel = AtomicBool::new(false);
    let ctl = long_control(&cancel);
    let status = f
        .runtime
        .prepare_model_preview(&json!({"action":"status"}), &ctl)
        .unwrap();
    assert_eq!(
        f.runtime
            .execute_model_preview(status, &ctl)
            .unwrap()
            .result["details"]["preview"]["ready"],
        false
    );
    assert!(
        f.runtime
            .prepare_model_preview(&json!({"action":"start"}), &ctl)
            .is_err()
    );
    let other = f.run("sleep 1");
    assert!(
        f.runtime
            .prepare_model_preview(&json!({"action":"stop","operation":other}), &ctl)
            .is_err(),
        "preview cannot stop a different kind of owned target"
    );
    f.runtime.stop().unwrap();
    fixture_files(
        &f.runtime,
        "web",
        &["package.json", "package-lock.json", "index.html", "main.js"],
    );
    node_tools(&mut f.runtime);
    f.runtime.discover().unwrap();
    let target = f
        .runtime
        .project
        .targets
        .iter()
        .find(|t| t.kind == Kind::Web)
        .unwrap()
        .id
        .clone();
    f.runtime
        .start(
            &target,
            "install",
            Launch {
                commands: f.runtime.command(Kind::Web, "install").unwrap(),
                readiness: None,
                timeout_ms: 120000,
            },
            &ctl,
        )
        .unwrap();
    assert_eq!(f.runtime.finish(&ctl).unwrap().lifecycle, Lifecycle::Exited);
    let stale = f
        .runtime
        .prepare_model_preview(&json!({"action":"start"}), &ctl)
        .unwrap();
    f.runtime
        .workspace
        .create_checked("new-source.txt", b"changed", false, || Ok(()))
        .unwrap();
    assert!(f.runtime.execute_model_preview(stale, &ctl).is_err());
    let start = f
        .runtime
        .prepare_model_preview(&json!({"action":"start"}), &ctl)
        .unwrap();
    let started = f.runtime.execute_model_preview(start, &ctl).unwrap();
    let operation = started.result["details"]["preview"]["operation"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        f.runtime
            .prepare_model_preview(&json!({"action":"stop","operation":"foreign"}), &ctl)
            .is_err()
    );
    assert!(
        f.runtime
            .prepare_model_preview(&json!({"action":"start"}), &ctl)
            .is_err()
    );
    let ready = loop {
        let status = f
            .runtime
            .prepare_model_preview(&json!({"action":"status"}), &ctl)
            .unwrap();
        let result = f.runtime.execute_model_preview(status, &ctl).unwrap();
        if result.result["details"]["preview"]["ready"] == true {
            break result;
        }
        assert!(Instant::now() < ctl.deadline);
        std::thread::sleep(Duration::from_millis(50));
    };
    let url = ready.result["details"]["preview"]["url"].as_str().unwrap();
    assert!(f.runtime.preview_grant().unwrap().allows(&operation, url));
    let stop = f
        .runtime
        .prepare_model_preview(&json!({"action":"stop","operation":operation}), &ctl)
        .unwrap();
    f.runtime.execute_model_preview(stop, &ctl).unwrap();
    assert!(f.runtime.preview_grant().is_err());
    assert!(
        f.runtime
            .prepare_model_preview(&json!({"action":"stop","operation":operation}), &ctl)
            .is_err()
    );
}
