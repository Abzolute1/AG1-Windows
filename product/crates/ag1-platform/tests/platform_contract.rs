#![cfg(target_os = "linux")]
use ag1_core::{
    DomainError, MutationPort, NetworkPort, NetworkRequest, PtyPort, PtySize, SecretPort,
    WorkspacePort,
};
use ag1_platform::{
    diagnostics::*, executor::BlockingPool, git::GitService, ports::*, process::*, workspace::*,
};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, mpsc},
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant},
};

const HELPER: &str = env!("CARGO_BIN_EXE_ag1-process-host");
const FIXTURE: &str = env!("CARGO_BIN_EXE_ag1-platform-fixture");
struct ThreadWake(thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park_timeout(Duration::from_millis(100)),
        }
    }
}
fn setup() -> (tempfile::TempDir, WorkspaceService, Arc<Workspace>) {
    let directory = tempfile::tempdir().unwrap();
    let service = WorkspaceService::new().unwrap();
    let workspace = service.register(directory.path()).unwrap();
    (directory, service, workspace)
}
fn spec(cwd: &Path, args: &[&str]) -> ProcessSpec {
    ProcessSpec {
        executable: PathBuf::from(FIXTURE),
        argv: args.iter().map(|s| s.to_string()).collect(),
        cwd: cwd.to_path_buf(),
        env: BTreeMap::from([("FIXTURE_TOKEN".into(), "explicit".into())]),
        timeout_ms: 10_000,
        grace_ms: 100,
    }
}
fn supervisor() -> ProcessSupervisor {
    ProcessSupervisor::new(Path::new(HELPER)).unwrap()
}
fn pid_file(path: &Path) -> u32 {
    let start = Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(pid) = text.parse()
        {
            return pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "fixture PID file missing"
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn gone(pid: u32) {
    assert!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "owned child {pid} remains"
    );
}
fn git(path: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(path)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn git_setup(path: &Path) -> String {
    git(path, &["init", "-q", "--initial-branch=main"]);
    fs::write(path.join("a.txt"), b"first\n").unwrap();
    fs::write(path.join(".gitignore"), b"ignored\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-qm", "initial"]);
    git(path, &["rev-parse", "HEAD"])
}

#[test]
fn explicit_workspaces_do_not_share_cwd_or_identity() {
    let (a, service, first) = setup();
    let b = tempfile::tempdir().unwrap();
    let second = service.register(b.path()).unwrap();
    fs::write(a.path().join("same"), b"a").unwrap();
    fs::write(b.path().join("same"), b"b").unwrap();
    let cwd = std::env::current_dir().unwrap();
    assert_ne!(first.id(), second.id());
    assert_eq!(
        block_on(service.read(first.id(), "same", 10)).unwrap(),
        b"a"
    );
    assert_eq!(
        block_on(service.read(second.id(), "same", 10)).unwrap(),
        b"b"
    );
    assert_eq!(std::env::current_dir().unwrap(), cwd);
    let alias = a.path().join("alias");
    symlink(a.path(), &alias).unwrap();
    assert_eq!(service.register(&alias).unwrap().id(), first.id());
}

#[test]
fn contained_open_rejects_symlink_swap_traversal_and_special_files() {
    let (dir, _, workspace) = setup();
    let foreign = tempfile::tempdir().unwrap();
    fs::write(foreign.path().join("secret"), b"foreign").unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("nested/file"), b"inside").unwrap();
    let prior = workspace.snapshot("nested/file", 100).unwrap();
    fs::rename(dir.path().join("nested"), dir.path().join("old")).unwrap();
    symlink(foreign.path(), dir.path().join("nested")).unwrap();
    for name in ["../secret", "/etc/passwd", "nested/secret", "nested/file"] {
        assert!(workspace.snapshot(name, 100).is_err());
    }
    assert!(
        workspace
            .replace("nested/file", &prior.revision, b"bad")
            .is_err()
    );
    assert_eq!(fs::read(foreign.path().join("secret")).unwrap(), b"foreign");
    let fifo = std::ffi::CString::new(dir.path().join("fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let start = Instant::now();
    assert!(workspace.snapshot("fifo", 100).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn snapshot_bytes_and_digest_detect_same_size_external_edit() {
    let (dir, _, workspace) = setup();
    let path = dir.path().join("file");
    fs::write(&path, b"aaaa").unwrap();
    let before = workspace.snapshot("file", 100).unwrap();
    fs::write(&path, b"bbbb").unwrap();
    let after = workspace.snapshot("file", 100).unwrap();
    assert_ne!(before.revision.sha256, after.revision.sha256);
    assert_eq!(after.bytes, b"bbbb");
    assert_eq!(
        workspace.replace("file", &before.revision, b"cccc"),
        Err(DomainError::Conflict)
    );
    assert_eq!(fs::read(path).unwrap(), b"bbbb");
}

#[test]
fn cas_replacement_preserves_mode_bom_crlf_and_rejects_stale_retry() {
    let (dir, service, workspace) = setup();
    let path = dir.path().join("script");
    fs::write(&path, b"\xef\xbb\xbfline\r\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o751)).unwrap();
    let before = workspace.snapshot("script", 100).unwrap();
    let bytes = preserve_text(&before, "changed\n").unwrap();
    let next =
        block_on(service.replace(workspace.id(), "script", &before.revision, &bytes)).unwrap();
    assert_eq!(next.mode, 0o751);
    assert_eq!(fs::read(path).unwrap(), b"\xef\xbb\xbfchanged\r\n");
    assert_eq!(
        workspace.replace("script", &before.revision, b"stale"),
        Err(DomainError::Conflict)
    );
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ag1-write-")
    }));
}

#[test]
fn rename_requires_revision_and_never_overwrites_existing_target() {
    let (dir, _, workspace) = setup();
    fs::write(dir.path().join("from"), b"data").unwrap();
    fs::write(dir.path().join("occupied"), b"keep").unwrap();
    let snap = workspace.snapshot("from", 100).unwrap();
    assert!(
        workspace
            .rename("from", "occupied", &snap.revision)
            .is_err()
    );
    workspace.rename("from", "to", &snap.revision).unwrap();
    assert!(!dir.path().join("from").exists());
    assert_eq!(workspace.snapshot("to", 100).unwrap().bytes, b"data");
    assert_eq!(fs::read(dir.path().join("occupied")).unwrap(), b"keep");
}

#[test]
fn permissions_and_failed_mutation_never_report_success() {
    let (dir, _, workspace) = setup();
    let path = dir.path().join("private");
    fs::write(&path, b"keep").unwrap();
    let snap = workspace.snapshot("private", 100).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let mut child = supervisor()
        .spawn(
            spec(dir.path(), &["deny-read", path.to_str().unwrap()]),
            &dir.path().join("deny.log"),
        )
        .unwrap();
    assert!(child.wait(Duration::from_secs(3)).unwrap().success());
    assert!(
        workspace
            .replace("private", &snap.revision, b"bad")
            .is_err()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(fs::read(path).unwrap(), b"keep");
}

#[test]
fn bounded_index_search_and_watcher_rebuild_on_external_rename() {
    let (dir, _, workspace) = setup();
    fs::write(dir.path().join("a"), b"needle").unwrap();
    fs::create_dir(dir.path().join("node_modules")).unwrap();
    fs::write(dir.path().join("node_modules/excluded"), b"skip").unwrap();
    let paths = workspace.scan(10).unwrap();
    assert_eq!(paths, ["a"]);
    let index = workspace.index_paths(&paths, 100).unwrap();
    assert_eq!(index.bytes_read, 6);
    assert_eq!(
        workspace.search(&paths, b"needle", 10).unwrap(),
        [("a".into(), 0)]
    );
    assert!(workspace.index_paths(&paths, 2).is_err());
    assert!(workspace.snapshot("a", 2).is_err());
    let mut watch = WatchHints::new(&workspace, &paths).unwrap();
    watch.rebuilt();
    fs::rename(dir.path().join("a"), dir.path().join("b")).unwrap();
    for _ in 0..100 {
        if watch.poll().unwrap() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(watch.rebuild_required);
    assert_eq!(workspace.scan(10).unwrap(), ["b"]);
    watch.rebuilt();
    watch.invalidate_overflow();
    assert!(watch.rebuild_required);
}

#[test]
fn process_argv_cwd_env_input_exit_and_identity_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = supervisor()
        .spawn(spec(dir.path(), &["echo"]), &dir.path().join("echo.log"))
        .unwrap();
    let pid = child.identity().pid;
    assert!(child.wait(Duration::from_secs(3)).unwrap().success());
    let output = String::from_utf8(child.tail(4096).unwrap().bytes).unwrap();
    assert!(output.contains(&format!("cwd={}", dir.path().display())));
    assert!(output.contains("token=explicit"));
    assert!(output.contains("ambient=false"));
    gone(pid);
    let mut input = supervisor()
        .spawn(spec(dir.path(), &["input"]), &dir.path().join("input.log"))
        .unwrap();
    input.write_input(b"bytes\0input").unwrap();
    input.close_input();
    assert!(input.wait(Duration::from_secs(3)).unwrap().success());
    assert_eq!(input.tail(100).unwrap().bytes, b"bytes\0input");
}

#[test]
fn parent_exit_with_descendant_holding_stdout_is_reaped_without_hang() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("desc.pid");
    let mut child = supervisor()
        .spawn(
            spec(dir.path(), &["orphan", pid_path.to_str().unwrap()]),
            &dir.path().join("orphan.log"),
        )
        .unwrap();
    let descendant = pid_file(&pid_path);
    let helper = child.identity().pid;
    let start = Instant::now();
    assert!(child.wait(Duration::from_secs(3)).unwrap().success());
    assert!(start.elapsed() < Duration::from_secs(3));
    gone(descendant);
    gone(helper);
}

#[test]
fn cancel_escalates_reaps_descendants_and_is_repeatable() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("desc.pid");
    let mut child = supervisor()
        .spawn(
            spec(dir.path(), &["descendant", pid_path.to_str().unwrap()]),
            &dir.path().join("cancel.log"),
        )
        .unwrap();
    let descendant = pid_file(&pid_path);
    let helper = child.identity().pid;
    child.stop().unwrap();
    child.stop().unwrap();
    gone(descendant);
    gone(helper);
}

#[test]
fn wrong_start_token_or_executable_cannot_cancel_owned_or_foreign_process() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("hold.pid");
    let mut child = supervisor()
        .spawn(
            spec(dir.path(), &["hold", pid_path.to_str().unwrap()]),
            &dir.path().join("identity.log"),
        )
        .unwrap();
    let pid = pid_file(&pid_path);
    let mut wrong = child.identity().clone();
    wrong.start_ticks += 1;
    assert_eq!(child.stop_if_identity(&wrong), Err(DomainError::Conflict));
    wrong = child.identity().clone();
    wrong.executable = "/unrelated".into();
    assert_eq!(child.stop_if_identity(&wrong), Err(DomainError::Conflict));
    assert!(Path::new(&format!("/proc/{pid}")).exists());
    child.stop().unwrap();
    gone(pid);
}

#[test]
fn foreign_port_conflict_does_not_stop_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    assert!(matches!(
        PortReservation::bind(address),
        Err(DomainError::Conflict)
    ));
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"alive").unwrap();
    });
    let mut response = String::new();
    TcpStream::connect(address)
        .unwrap()
        .read_to_string(&mut response)
        .unwrap();
    server.join().unwrap();
    assert_eq!(response, "alive");
}

#[test]
fn timeout_and_drop_cancel_only_owned_fixture_tree() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("timeout.pid");
    let mut request = spec(dir.path(), &["hold", pid_path.to_str().unwrap()]);
    request.timeout_ms = 200;
    let mut child = supervisor()
        .spawn(request, &dir.path().join("timeout.log"))
        .unwrap();
    let pid = pid_file(&pid_path);
    assert!(!child.wait(Duration::from_secs(3)).unwrap().success());
    gone(pid);
    let pid_path = dir.path().join("drop.pid");
    let child = supervisor()
        .spawn(
            spec(dir.path(), &["hold", pid_path.to_str().unwrap()]),
            &dir.path().join("drop.log"),
        )
        .unwrap();
    let pid = pid_file(&pid_path);
    drop(child);
    gone(pid);
}

#[test]
fn log_spool_memory_disk_and_tail_cost_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = supervisor()
        .spawn(spec(dir.path(), &["flood"]), &dir.path().join("flood.log"))
        .unwrap();
    assert!(child.wait(Duration::from_secs(5)).unwrap().success());
    let start = Instant::now();
    let view = child.tail(100).unwrap();
    assert!(view.bytes.ends_with(b"TAIL-END"));
    assert!(view.truncated);
    assert_eq!(view.memory_bytes, MAX_LOG_MEMORY);
    assert_eq!(view.disk_bytes, MAX_LOG_DISK);
    assert_eq!(child.log_page(0, 10).unwrap(), [b'x'; 10]);
    assert!(child.log_page(0, 32769).is_err());
    println!(
        "P03_LOG {}",
        serde_json::json!({"seen":view.total_seen,"memory_bytes":view.memory_bytes,"disk_bytes":view.disk_bytes,"tail_ns":start.elapsed().as_nanos()})
    );
}

#[test]
fn pty_bytes_resize_exit_and_disconnect_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let request = ProcessSpec {
        executable: "/bin/sh".into(),
        argv: vec![
            "-c".into(),
            "read line; /usr/bin/stty size; printf 'got:%s' \"$line\"".into(),
        ],
        cwd: dir.path().into(),
        env: BTreeMap::new(),
        timeout_ms: 5000,
        grace_ms: 100,
    };
    let mut pty = supervisor()
        .spawn_pty(
            request,
            &dir.path().join("pty.log"),
            PtySize {
                rows: 24,
                columns: 80,
            },
        )
        .unwrap();
    pty.resize(PtySize {
        rows: 40,
        columns: 120,
    })
    .unwrap();
    pty.write_input(b"hello\n").unwrap();
    let mut output = vec![];
    let start = Instant::now();
    loop {
        output.extend(pty.read_output(4096).unwrap());
        if String::from_utf8_lossy(&output).contains("got:hello") {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(5));
    }
    assert!(String::from_utf8_lossy(&output).contains("40 120"));
    assert!(pty.process.wait(Duration::from_secs(3)).unwrap().success());
    pty.disconnect().unwrap();
    assert_eq!(pty.write_input(b"late"), Err(DomainError::Cancelled));
}

#[test]
fn git_worktree_identity_ignore_untracked_diff_and_restore_guards() {
    let (dir, service, workspace) = setup();
    let head = git_setup(dir.path());
    let spools = tempfile::tempdir().unwrap();
    let git_service =
        GitService::new(Path::new(HELPER), Path::new("/usr/bin/git"), spools.path()).unwrap();
    let clean = git_service.status_diff(&workspace, Some(&head)).unwrap();
    assert!(clean.status.is_empty());
    let snap = workspace.snapshot("a.txt", 100).unwrap();
    assert_eq!(
        git_service
            .restore_candidate(&workspace, "a.txt", &head, &snap.revision, &head)
            .unwrap(),
        b"first\n"
    );
    fs::write(dir.path().join("ignored"), b"ignore me").unwrap();
    fs::write(dir.path().join("untracked"), b"new").unwrap();
    fs::write(dir.path().join("a.txt"), b"changed\n").unwrap();
    let dirty = git_service.status_diff(&workspace, Some(&head)).unwrap();
    assert!(String::from_utf8_lossy(&dirty.diff).contains("changed"));
    let files = git_service.files(&workspace).unwrap();
    assert!(files.contains(&"untracked".into()));
    assert!(!files.contains(&"ignored".into()));
    assert!(
        git_service
            .restore_candidate(&workspace, "a.txt", &head, &snap.revision, &head)
            .is_err()
    );
    assert!(matches!(
        git_service.status_diff(&workspace, Some(&"0".repeat(40))),
        Err(DomainError::Conflict)
    ));
    let worktree = tempfile::tempdir().unwrap();
    let target = worktree.path().join("linked");
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "--detach",
            target.to_str().unwrap(),
            &head,
        ],
    );
    let linked = service.register(&target).unwrap();
    let linked_view = git_service.status_diff(&linked, Some(&head)).unwrap();
    assert_ne!(linked.id(), workspace.id());
    assert_ne!(linked_view.worktree, clean.worktree);
}

#[test]
fn conflicted_git_restore_is_refused() {
    let (dir, _, workspace) = setup();
    git_setup(dir.path());
    git(dir.path(), &["checkout", "-qb", "other"]);
    fs::write(dir.path().join("a.txt"), b"other\n").unwrap();
    git(dir.path(), &["commit", "-am", "other"]);
    git(dir.path(), &["checkout", "main"]);
    fs::write(dir.path().join("a.txt"), b"main\n").unwrap();
    git(dir.path(), &["commit", "-am", "main"]);
    let status = Command::new("/usr/bin/git")
        .args(["-c", "core.hooksPath=/dev/null", "merge", "other"])
        .current_dir(dir.path())
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@invalid")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@invalid")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    let spools = tempfile::tempdir().unwrap();
    let service =
        GitService::new(Path::new(HELPER), Path::new("/usr/bin/git"), spools.path()).unwrap();
    let view = service.status_diff(&workspace, None).unwrap();
    assert!(
        view.conflicted,
        "status={:?}",
        String::from_utf8_lossy(&view.status)
    );
    let snap = workspace.snapshot("a.txt", 1000).unwrap();
    assert!(
        service
            .restore_candidate(&workspace, "a.txt", &view.head, &snap.revision, &view.head)
            .is_err()
    );
}

#[test]
fn executor_saturation_is_nonblocking_and_panics_settle() {
    let pool = BlockingPool::new(1, 1).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first = pool.submit(move || {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        Ok(1)
    });
    started_rx.recv().unwrap();
    let second = pool.submit(|| Ok(2));
    assert_eq!(block_on(pool.submit(|| Ok(3))), Err(DomainError::QueueFull));
    release_tx.send(()).unwrap();
    assert_eq!(block_on(first), Ok(1));
    assert_eq!(block_on(second), Ok(2));
    assert!(block_on(pool.submit::<()>(|| panic!("controlled fixture panic"))).is_err());
    assert_eq!(block_on(pool.submit(|| Ok(4))), Ok(4));
}

#[test]
fn fake_network_secret_namespaces_and_pipe_close_state_are_explicit() {
    let mut secrets = FakeSecretPort::default();
    secrets
        .insert("model-a".into(), "token".into(), b"fixture".to_vec())
        .unwrap();
    assert_eq!(
        block_on(secrets.resolve("model-a", "token"))
            .unwrap()
            .expose(),
        b"fixture"
    );
    assert!(block_on(secrets.resolve("model-b", "token")).is_err());
    let mut network = FakeNetworkPort::default();
    network
        .allow_origin("https://fixture.invalid".into())
        .unwrap();
    network
        .respond(
            "https://fixture.invalid".into(),
            "/data".into(),
            b"body".to_vec(),
        )
        .unwrap();
    let request = NetworkRequest {
        origin: "https://fixture.invalid".into(),
        path: "/data".into(),
        max_bytes: 4,
    };
    assert_eq!(block_on(network.fetch(request.clone())).unwrap(), b"body");
    let mut small = request.clone();
    small.max_bytes = 3;
    assert!(block_on(network.fetch(small)).is_err());
    let mut wrong = request;
    wrong.origin = "http://127.0.0.1".into();
    assert_eq!(
        block_on(network.fetch(wrong)),
        Err(DomainError::PermissionDenied)
    );
    let mut pipe = DrainState {
        exit_seen: true,
        ..Default::default()
    };
    assert!(!pipe.finished());
    pipe.tick(200);
    assert!(pipe.needs_tree_cancel());
    pipe.eof_seen = true;
    assert!(pipe.finished());
}

#[test]
fn compiler_diagnostics_and_target_discovery_are_bounded() {
    let input = br#"{"reason":"compiler-message","message":{"message":"type mismatch","level":"error","spans":[{"is_primary":true,"file_name":"src/main.rs","line_start":7}]}}"#;
    let diagnostics = cargo_diagnostics(input).unwrap();
    assert_eq!(diagnostics[0].line, Some(7));
    assert!(cargo_diagnostics(&vec![b'x'; 1024 * 1024 + 1]).is_err());
    assert_eq!(
        discover_targets(&[
            "package.json".into(),
            "src-tauri/tauri.conf.json".into(),
            "project.godot".into()
        ])
        .unwrap(),
        [TargetKind::Web, TargetKind::Tauri, TargetKind::Godot]
    );
}

#[test]
fn worker_stdout_is_separate_from_diagnostic_spool() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = supervisor()
        .spawn_protocol(
            spec(dir.path(), &["protocol"]),
            &dir.path().join("protocol.log"),
        )
        .unwrap();
    assert!(child.wait(Duration::from_secs(3)).unwrap().success());
    assert_eq!(child.read_stdout(4096).unwrap(), b"{\"type\":\"ready\"}\n");
    assert_eq!(child.tail(4096).unwrap().bytes, b"diagnostic only\n");
    assert!(child.read_stdout(32769).is_err());
}

#[test]
fn failed_write_is_reported_and_original_bytes_survive() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("read-only");
    fs::create_dir(&work).unwrap();
    fs::write(work.join("file"), b"keep").unwrap();
    fs::set_permissions(&work, fs::Permissions::from_mode(0o555)).unwrap();
    let mut child = supervisor()
        .spawn(
            spec(dir.path(), &["deny-mutate", work.to_str().unwrap()]),
            &dir.path().join("write-denied.log"),
        )
        .unwrap();
    let status = child.wait(Duration::from_secs(3)).unwrap();
    fs::set_permissions(&work, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        status.success(),
        "{}",
        String::from_utf8_lossy(&child.tail(4096).unwrap().bytes)
    );
    assert_eq!(fs::read(work.join("file")).unwrap(), b"keep");
    assert_eq!(fs::read_dir(work).unwrap().count(), 1);
}

#[test]
fn process_disappearing_during_stop_and_launch_failures_release_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = supervisor();
    for i in 0..40 {
        let mut bad = spec(dir.path(), &["echo"]);
        bad.executable = "/does-not-exist".into();
        assert!(
            supervisor
                .spawn(bad, &dir.path().join(format!("bad-{i}.log")))
                .is_err()
        );
        let mut child = supervisor
            .spawn(
                spec(dir.path(), &["echo"]),
                &dir.path().join(format!("quick-{i}.log")),
            )
            .unwrap();
        thread::sleep(Duration::from_millis(5));
        child.stop().unwrap();
        child.stop().unwrap();
        gone(child.identity().pid);
    }
}

#[test]
fn manifest_bounds_and_freshness_are_preserved() {
    let (dir, _, workspace) = setup();
    fs::write(
        dir.path().join("package.json"),
        br#"{"main":"index.js","scripts":{"a":"a","b":"b","c":"c","d":"d","e":"e","f":"f"}}"#,
    )
    .unwrap();
    let paths = workspace.scan(100).unwrap();
    let index = workspace.index_paths(&paths, 4096).unwrap();
    let summary = manifest_summary(&workspace, &index).unwrap();
    assert_eq!(summary.entry_points, ["index.js"]);
    assert_eq!(summary.scripts.len(), 5);
    assert!(summary.truncated);
    fs::write(dir.path().join("package.json"), b"{}").unwrap();
    assert!(manifest_summary(&workspace, &index).is_err());
    assert!(
        workspace
            .index_paths(&vec!["package.json".into(); 1001], 4096)
            .is_err()
    );
}

#[test]
fn workspace_claims_serialize_writers_and_root_replacement_conflicts() {
    let (dir, _, workspace) = setup();
    fs::write(dir.path().join("file"), b"first").unwrap();
    let snapshot = workspace.snapshot("file", 100).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut threads = vec![];
    for bytes in [b"aaaaa", b"bbbbb"] {
        let workspace = workspace.clone();
        let revision = snapshot.revision.clone();
        let barrier = barrier.clone();
        threads.push(thread::spawn(move || {
            barrier.wait();
            workspace.replace("file", &revision, bytes)
        }));
    }
    barrier.wait();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let moved = dir.path().with_extension("moved-fixture");
    fs::rename(dir.path(), &moved).unwrap();
    fs::create_dir(dir.path()).unwrap();
    assert!(workspace.snapshot("file", 100).is_err());
    fs::remove_dir(dir.path()).unwrap();
    fs::rename(moved, dir.path()).unwrap();
}

#[test]
fn records_small_large_fixture_latency_and_bounded_bytes() {
    for count in [120, 900] {
        let (dir, _, workspace) = setup();
        let head = git_setup(dir.path());
        for i in 0..count {
            fs::write(
                dir.path().join(format!("file-{i}.ts")),
                format!("export const item{i} = 'needle';\n"),
            )
            .unwrap();
        }
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "files"]);
        let spools = tempfile::tempdir().unwrap();
        let service =
            GitService::new(Path::new(HELPER), Path::new("/usr/bin/git"), spools.path()).unwrap();
        let mut rows = BTreeMap::new();
        for name in ["status_diff", "index", "search", "read", "write"] {
            let mut samples = vec![];
            for _ in 0..5 {
                let paths = workspace.scan(MAX_FILES).unwrap();
                let start = Instant::now();
                match name {
                    "status_diff" => {
                        service.status_diff(&workspace, None).unwrap();
                    }
                    "index" => {
                        workspace.index_paths(&paths, MAX_INDEX_BYTES).unwrap();
                    }
                    "search" => {
                        workspace.search(&paths, b"needle", 1000).unwrap();
                    }
                    "read" => {
                        workspace.snapshot("a.txt", 100).unwrap();
                    }
                    _ => {
                        let snapshot = workspace.snapshot("a.txt", 100).unwrap();
                        workspace
                            .replace("a.txt", &snapshot.revision, &snapshot.bytes)
                            .unwrap();
                    }
                }
                samples.push(start.elapsed().as_nanos());
            }
            samples.sort_unstable();
            rows.insert(name, samples);
        }
        println!(
            "P03_MEASUREMENT {}",
            serde_json::json!({"files":count,"samples_ns":rows,"fixture_initial_head":head,"mode":"warm disposable Git fixture; synchronous call time includes blocking; 5 samples; uncontrolled CPU scheduling"})
        );
    }
}
