use super::*;
use crate::github::{
    service::{Account, ApiError, Reply, Transport},
    transport::GitCredential,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("/usr/bin/git")
        .current_dir(root)
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Owned fixture")
        .env("GIT_COMMITTER_NAME", "Owned fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "owned git: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
struct Auth;
impl Transport for Auth {
    fn account(&self, _: &ScanControl<'_>) -> crate::github::service::Result<Account> {
        Account::bound(1, "owned".into(), "synthetic".into())
    }
    fn execute(
        &self,
        _: &Account,
        _: &crate::github::operations::Operation,
        _: &ScanControl<'_>,
    ) -> crate::github::service::Result<Reply> {
        Err(ApiError::new(
            "fixture",
            "No API calls in Git protocol fixture.",
        ))
    }
    fn git_credential(
        &self,
        _: &Account,
        _: &ScanControl<'_>,
    ) -> crate::github::service::Result<GitCredential> {
        Ok(GitCredential::fixture())
    }
}
type TransferHook = Arc<Mutex<Option<Box<dyn FnOnce() + Send>>>>;
struct Server {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    advance: Arc<Mutex<Option<String>>>,
    transfer_hook: TransferHook,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
fn server(root: &Path, cert: &Path, key: &Path) -> Server {
    let acceptor = native_tls::TlsAcceptor::new(
        native_tls::Identity::from_pkcs8(
            &std::fs::read(cert).unwrap(),
            &std::fs::read(key).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("https://{}/repo.git", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let halt = stop.clone();
    let root = root.to_path_buf();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let advance = Arc::new(Mutex::new(None::<String>));
    let race = advance.clone();
    let transfer_hook: TransferHook = Arc::new(Mutex::new(None));
    let transfer = transfer_hook.clone();
    let worker = thread::spawn(move || {
        while !halt.load(Ordering::Acquire) {
            let socket = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("fixture TLS: {e}"),
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let Ok(mut stream) = acceptor.accept(socket) else {
                continue;
            };
            let mut headers = Vec::new();
            let mut byte = [0];
            while !headers.ends_with(b"\r\n\r\n") && headers.len() < 16384 {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    break;
                }
                headers.push(byte[0]);
            }
            if headers.is_empty() {
                continue;
            }
            let headers = String::from_utf8(headers).unwrap();
            let header = |name: &str| {
                headers.lines().find_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case(name))
                        .map(|(_, v)| v.trim().to_string())
                })
            };
            assert_eq!(
                header("authorization").as_deref(),
                Some("Basic eC1hY2Nlc3MtdG9rZW46b3duZWQtc3ludGhldGljLXRva2Vu")
            );
            let mut words = headers.split_whitespace();
            let method = words.next().unwrap();
            let route = words.next().unwrap();
            let (path, query) = route.split_once('?').unwrap_or((route, ""));
            let len = header("content-length")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0);
            assert!(len <= 1024 * 1024);
            let mut body = vec![0; len];
            if stream.read_exact(&mut body).is_err() {
                continue;
            }
            // Retain only paths, never authentication headers/body in evidence.
            seen.lock().unwrap().push(format!("{method} {route}"));
            assert!(path.starts_with("/repo.git/"));
            if path == "/repo.git/git-upload-pack"
                && let Some(hook) = transfer.lock().unwrap().take()
            {
                hook();
            }
            if query == "service=git-receive-pack"
                && let Some(sha) = race.lock().unwrap().take()
            {
                git(
                    &root.join("repo.git"),
                    &["update-ref", "refs/heads/main", &sha],
                );
            }
            let mut backend = Command::new("/usr/lib/git-core/git-http-backend")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_PROJECT_ROOT", &root)
                .env("GIT_HTTP_EXPORT_ALL", "1")
                .env("PATH_INFO", path)
                .env("QUERY_STRING", query)
                .env("REQUEST_METHOD", method)
                .env("CONTENT_TYPE", header("content-type").unwrap_or_default())
                .env("CONTENT_LENGTH", len.to_string())
                .env("REMOTE_USER", "owned")
                .env("REMOTE_ADDR", "127.0.0.1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            backend.stdin.take().unwrap().write_all(&body).unwrap();
            let response = backend.wait_with_output().unwrap();
            assert!(response.status.success());
            let split = response
                .stdout
                .windows(4)
                .position(|s| s == b"\r\n\r\n")
                .unwrap();
            let cgi = String::from_utf8(response.stdout[..split].to_vec()).unwrap();
            let body = &response.stdout[split + 4..];
            let status = cgi
                .lines()
                .find_map(|l| l.strip_prefix("Status: "))
                .unwrap_or("200 OK");
            let extra = cgi
                .lines()
                .filter(|l| !l.starts_with("Status:"))
                .collect::<Vec<_>>()
                .join("\r\n");
            let _=stream.write_all(format!("HTTP/1.1 {status}\r\n{extra}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes());
            let _ = stream.write_all(body);
            let _ = stream.shutdown();
        }
    });
    Server {
        stop,
        worker: Some(worker),
        url,
        requests,
        advance,
        transfer_hook,
    }
}

#[test]
fn exact_fetch_and_leased_push_use_owned_https_preserving_user_bytes_and_config() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let remote = temp.path().join("repo.git");
    let source = temp.path().join("source");
    for path in [&root, &source] {
        std::fs::create_dir(path).unwrap();
        git(path, &["init", "--quiet", "-b", "main"]);
    }
    git(
        temp.path(),
        &["init", "--bare", "--quiet", remote.to_str().unwrap()],
    );
    git(&remote, &["config", "http.receivepack", "true"]);
    std::fs::write(source.join("notes.txt"), "remote baseline\n").unwrap();
    git(&source, &["add", "notes.txt"]);
    git(&source, &["commit", "--quiet", "-m", "baseline"]);
    let baseline = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    std::fs::write(root.join("dirty.txt"), "untracked user bytes\r\n").unwrap();
    git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/owned/repo.git",
        ],
    );
    git(
        &root,
        &[
            "config",
            "url.https://untrusted.invalid/.insteadOf",
            "https://github.com/",
        ],
    );
    let unexpected = temp.path().join("unexpected-helper");
    git(
        &root,
        &[
            "config",
            "credential.helper",
            &format!("!touch {}", unexpected.display()),
        ],
    );
    let cert = root.join("fixture-cert.pem");
    let key = temp.path().join("key.pem");
    assert!(
        Command::new("/usr/bin/openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=IP:127.0.0.1",
                "-keyout"
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .unwrap()
            .status
            .success()
    );
    let server = server(temp.path(), &cert, &key);
    git(
        &root,
        &[
            "config",
            "--add",
            "url.https://untrusted.invalid/.insteadOf",
            &server.url,
        ],
    );
    let state = temp.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(state.join("github-connected"), b"owned").unwrap();
    let github = Arc::new(crate::github::service::Service::new(
        state.clone(),
        Arc::new(Auth),
    ));
    let helper = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ag1-process-host");
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let mut local = LocalGit::new(
        workspace,
        &helper,
        Path::new("/usr/bin/git"),
        Path::new("/usr/bin/bwrap"),
        &state.join("git"),
    )
    .unwrap()
    .with_github(github);
    local.network_fixture=Some((server.url.clone(),b"\n[http]\nsslCAInfo = /workspace/fixture-cert.pem\nextraHeader = Authorization: Basic eC1hY2Nlc3MtdG9rZW46b3duZWQtc3ludGhldGljLXRva2Vu\n".to_vec()));
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(40),
    };
    let original = std::fs::read(root.join(".git/config")).unwrap();
    let args = json!({"repo":"owned/repo","branch":"main","expected_remote_sha":baseline});
    let plan = local.prepare("fetch", &args, "scope", &control).unwrap();
    assert!(
        server.requests.lock().unwrap().is_empty(),
        "preview cannot send repository traffic"
    );
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap()["commit"],
        baseline
    );
    assert_eq!(
        git(&root, &["rev-parse", "refs/remotes/origin/main"]),
        baseline
    );
    assert!(
        std::fs::read_dir(root.join(".git/objects/pack"))
            .unwrap()
            .any(|e| e.unwrap().path().extension().is_some_and(|s| s == "pack")),
        "Native fetch keeps a bounded pack instead of expanding arbitrary loose files"
    );
    assert!(!root.join(".git/index").exists());
    assert!(!root.join(".git/FETCH_HEAD").exists());
    assert_eq!(std::fs::read(root.join(".git/config")).unwrap(), original);
    assert_eq!(
        std::fs::read(root.join("dirty.txt")).unwrap(),
        b"untracked user bytes\r\n"
    );
    // Adopt baseline only in the owned fixture, then make a local descendant.
    git(&root, &["update-ref", "refs/heads/main", &baseline]);
    git(&root, &["read-tree", "HEAD"]);
    std::fs::write(root.join("notes.txt"), "local descendant\n").unwrap();
    git(&root, &["add", "notes.txt"]);
    git(&root, &["commit", "--quiet", "-m", "descendant"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("notes.txt"), "later user edits\r\n").unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let plan = local.prepare("push", &args, "scope", &control).unwrap();
    local.execute(plan, "scope", &control).unwrap();
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]), head);
    // Reusing a reviewed but now-stale remote SHA cannot overwrite the remote.
    let plan = local.prepare("push", &args, "scope", &control).unwrap();
    assert!(local.execute(plan, "scope", &control).is_err());
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]), head);
    // A competing ref arriving after ls-remote is still stopped by the exact
    // server-side lease, not merely the earlier native check.
    let competing = git(
        &root,
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            &head,
            "-m",
            "owned competing update",
        ],
    );
    git(
        &root,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            &format!("{competing}:refs/heads/competing"),
        ],
    );
    let plan = local
        .prepare(
            "push",
            &json!({"repo":"owned/repo","branch":"main","expected_remote_sha":head}),
            "scope",
            &control,
        )
        .unwrap();
    *server.advance.lock().unwrap() = Some(competing.clone());
    assert!(local.execute(plan, "scope", &control).is_err());
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/main"]), competing);
    assert_eq!(
        std::fs::read(root.join("notes.txt")).unwrap(),
        b"later user edits\r\n"
    );
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(std::fs::read(root.join(".git/config")).unwrap(), original);
    assert!(!root.join(".git/index.lock").exists());
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r == "POST /repo.git/git-upload-pack")
    );
    assert!(
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r == "POST /repo.git/git-receive-pack")
    );
    assert!(
        std::fs::read_dir(state.join("git")).unwrap().all(|p| !p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".log"))
    );
    assert!(!unexpected.exists());
    // Pull-request heads use the base repository's fixed refs/pull/N/head,
    // including commits which are not on a branch in this local checkout.
    std::fs::write(source.join("notes.txt"), b"reviewed pull request\n").unwrap();
    git(&source, &["add", "notes.txt"]);
    git(&source, &["commit", "--quiet", "-m", "owned PR head"]);
    let pr = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            "HEAD:refs/pull/7/head",
        ],
    );
    let pr_args = json!({"repo":"owned/repo","pull_number":7,"expected_remote_sha":pr});
    let before_calls = server.requests.lock().unwrap().len();
    let plan = local
        .prepare("fetch_pr", &pr_args, "scope", &control)
        .unwrap();
    assert_eq!(plan.preview()["remote_branch"], "refs/pull/7/head");
    assert_eq!(server.requests.lock().unwrap().len(), before_calls);
    local.execute(plan, "scope", &control).unwrap();
    assert_eq!(git(&root, &["rev-parse", "refs/remotes/origin/pr/7"]), pr);
    assert_eq!(
        std::fs::read(root.join("notes.txt")).unwrap(),
        b"later user edits\r\n"
    );
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    let checkout = json!({"repo":"owned/repo","branch":"pr/7","target_sha":pr});
    assert!(
        local
            .prepare("checkout", &checkout, "scope", &control)
            .is_err(),
        "PR checkout refuses overlapping dirty work"
    );
    std::fs::write(root.join("notes.txt"), b"local descendant\n").unwrap();
    let plan = local
        .prepare("checkout", &checkout, "scope", &control)
        .unwrap();
    local.execute(plan, "scope", &control).unwrap();
    assert_eq!(git(&root, &["branch", "--show-current"]), "pr/7");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), pr);
    assert_eq!(
        std::fs::read(root.join("notes.txt")).unwrap(),
        b"reviewed pull request\n"
    );
    assert_eq!(
        std::fs::read(root.join("dirty.txt")).unwrap(),
        b"untracked user bytes\r\n"
    );
    assert_eq!(std::fs::read(root.join(".git/config")).unwrap(), original);
    assert!(!unexpected.exists());
    // The same actual HTTPS transport operates only through explicitly granted
    // linked metadata and preserves the original worktree index/files.
    let linked = temp.path().join("linked");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked-push",
            linked.to_str().unwrap(),
        ],
    );
    std::fs::copy(&cert, linked.join("fixture-cert.pem")).unwrap();
    let workspace = WorkspaceService::new().unwrap().register(&linked).unwrap();
    let grants = crate::folder_access::FolderAccess::open(
        workspace.clone(),
        &temp.path().join("linked-grants"),
        "linked",
    )
    .unwrap();
    let selection = grants
        .change(crate::folder_access::Change {
            revision: grants.snapshot().unwrap().selection.revision,
            action: "add".into(),
            path: root.clone(),
            mode: Some(crate::folder_access::AccessMode::ReadWrite),
        })
        .unwrap();
    let mut linked_local = LocalGit::new(
        workspace,
        &helper,
        Path::new("/usr/bin/git"),
        Path::new("/usr/bin/bwrap"),
        &state.join("git-linked"),
    )
    .unwrap()
    .with_grant(grants, selection.selection.revision, true)
    .with_github(local.github.as_ref().unwrap().clone());
    linked_local.network_fixture = local.network_fixture.clone();
    let main_index = std::fs::read(root.join(".git/index")).unwrap();
    let linked_index = linked_local.index().unwrap().unwrap().0;
    let plan = linked_local
        .prepare(
            "fetch",
            &json!({"repo":"owned/repo","branch":"main","expected_remote_sha":competing}),
            "linked",
            &control,
        )
        .unwrap();
    linked_local.execute(plan, "linked", &control).unwrap();
    assert_eq!(
        git(&linked, &["rev-parse", "refs/remotes/origin/main"]),
        competing
    );
    let plan=linked_local.prepare("push",&json!({"repo":"owned/repo","branch":"linked-push","expected_remote_sha":"0".repeat(40)}),"linked",&control).unwrap();
    linked_local.execute(plan, "linked", &control).unwrap();
    assert_eq!(git(&remote, &["rev-parse", "refs/heads/linked-push"]), pr);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), main_index);
    assert_eq!(linked_local.index().unwrap().unwrap().0, linked_index);
    assert_eq!(std::fs::read(root.join(".git/config")).unwrap(), original);
}

#[test]
fn reviewed_pull_fast_forwards_only_after_fetch_and_preserves_unrelated_work() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("workspace");
    let source = temp.path().join("source");
    let remote = temp.path().join("repo.git");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "--quiet", "-b", "main"]);
    git(
        temp.path(),
        &["init", "--bare", "--quiet", remote.to_str().unwrap()],
    );
    std::fs::write(source.join("notes"), b"original\n").unwrap();
    std::fs::write(source.join("unrelated"), b"baseline\n").unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "baseline"]);
    let baseline = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    git(
        temp.path(),
        &[
            "clone",
            "--quiet",
            "-b",
            "main",
            remote.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );
    git(
        &root,
        &[
            "remote",
            "set-url",
            "origin",
            "https://github.com/owned/repo.git",
        ],
    );
    std::fs::write(root.join("unrelated"), b"user staged bytes\r\n").unwrap();
    git(&root, &["add", "unrelated"]);
    let staged = git(&root, &["rev-parse", ":unrelated"]);
    std::fs::write(root.join("unrelated"), b"later user dirty bytes\r\n").unwrap();
    std::fs::write(root.join("untracked"), b"keep me").unwrap();
    std::fs::write(source.join("notes"), b"reviewed remote bytes\n").unwrap();
    std::fs::write(source.join("new"), b"reviewed new file\n").unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "remote descendant"]);
    let target = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    let cert = root.join("fixture-cert.pem");
    let key = temp.path().join("key.pem");
    assert!(
        Command::new("/usr/bin/openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=IP:127.0.0.1",
                "-keyout"
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .unwrap()
            .status
            .success()
    );
    let server = server(temp.path(), &cert, &key);
    let state = temp.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(state.join("github-connected"), b"owned").unwrap();
    let github = Arc::new(crate::github::service::Service::new(
        state.clone(),
        Arc::new(Auth),
    ));
    let helper = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ag1-process-host");
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let mut local = LocalGit::new(
        workspace,
        &helper,
        Path::new("/usr/bin/git"),
        Path::new("/usr/bin/bwrap"),
        &state.join("git"),
    )
    .unwrap()
    .with_github(github.clone());
    local.network_fixture=Some((server.url.clone(),b"\n[http]\nsslCAInfo = /workspace/fixture-cert.pem\nextraHeader = Authorization: Basic eC1hY2Nlc3MtdG9rZW46b3duZWQtc3ludGhldGljLXRva2Vu\n".to_vec()));
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(60),
    };
    let args = json!({"repo":"owned/repo","branch":"main","expected_remote_sha":target});
    let config = std::fs::read(root.join(".git/config")).unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    assert!(local.prepare("pull", &args, "scope", &control).is_err());
    assert!(server.requests.lock().unwrap().is_empty());
    let plan = local.prepare("fetch", &args, "scope", &control).unwrap();
    local.execute(plan, "scope", &control).unwrap();
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), baseline);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(std::fs::read(root.join("notes")).unwrap(), b"original\n");
    std::fs::write(root.join("notes"), b"affected user edit").unwrap();
    assert!(local.prepare("pull", &args, "scope", &control).is_err());
    std::fs::write(root.join("notes"), b"original\n").unwrap();
    let calls = server.requests.lock().unwrap().len();
    let plan = local.prepare("pull", &args, "scope", &control).unwrap();
    assert_eq!(plan.preview()["file_count"], 2);
    assert_eq!(plan.preview()["target_branch"], "refs/heads/main");
    assert_eq!(server.requests.lock().unwrap().len(), calls);
    // A remote move while awaiting approval cannot apply the stale plan.
    git(&remote, &["update-ref", "refs/heads/main", &baseline]);
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap_err(),
        DomainError::Conflict
    );
    git(&remote, &["update-ref", "refs/heads/main", &target]);
    for kind in [
        "tracking",
        "file",
        "scope",
        "cancel",
        "head-lock",
        "branch-lock",
        "merge",
    ] {
        let plan = local.prepare("pull", &args, "scope", &control).unwrap();
        let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
        match kind {
            "tracking" => {
                git(
                    &root,
                    &["update-ref", "refs/remotes/origin/main", &baseline],
                );
            }
            "file" => std::fs::write(root.join("notes"), b"later affected edit").unwrap(),
            "cancel" => cancel.store(true, Ordering::Release),
            "head-lock" => {
                std::fs::write(root.join(".git/HEAD.lock"), b"foreign HEAD lock").unwrap()
            }
            "branch-lock" => std::fs::write(
                root.join(".git/refs/heads/main.lock"),
                b"foreign branch lock",
            )
            .unwrap(),
            "merge" => std::fs::write(root.join(".git/MERGE_HEAD"), target.as_bytes()).unwrap(),
            _ => {}
        }
        assert!(
            local
                .execute(
                    plan,
                    if kind == "scope" { "other" } else { "scope" },
                    &control
                )
                .is_err(),
            "{kind}"
        );
        assert!(!recovery.exists(), "{kind}");
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), baseline);
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        match kind {
            "tracking" => {
                git(&root, &["update-ref", "refs/remotes/origin/main", &target]);
            }
            "file" => {
                assert_eq!(
                    std::fs::read(root.join("notes")).unwrap(),
                    b"later affected edit"
                );
                std::fs::write(root.join("notes"), b"original\n").unwrap();
            }
            "cancel" => cancel.store(false, Ordering::Release),
            "head-lock" => {
                assert_eq!(
                    std::fs::read(root.join(".git/HEAD.lock")).unwrap(),
                    b"foreign HEAD lock"
                );
                std::fs::remove_file(root.join(".git/HEAD.lock")).unwrap();
            }
            "branch-lock" => {
                assert_eq!(
                    std::fs::read(root.join(".git/refs/heads/main.lock")).unwrap(),
                    b"foreign branch lock"
                );
                std::fs::remove_file(root.join(".git/refs/heads/main.lock")).unwrap();
            }
            "merge" => {
                assert_eq!(
                    std::fs::read(root.join(".git/MERGE_HEAD")).unwrap(),
                    target.as_bytes()
                );
                assert!(local.prepare("pull", &args, "scope", &control).is_err());
                std::fs::remove_file(root.join(".git/MERGE_HEAD")).unwrap();
            }
            _ => {}
        }
    }
    // Divergence, a different local branch, and disconnected admission all fail
    // before file publication. All refs here belong to this temporary fixture.
    let divergent = git(
        &root,
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            &baseline,
            "-m",
            "local divergence",
        ],
    );
    git(&root, &["update-ref", "refs/heads/main", &divergent]);
    assert!(local.prepare("pull", &args, "scope", &control).is_err());
    git(&root, &["update-ref", "refs/heads/main", &baseline]);
    git(&root, &["update-ref", "refs/heads/other", &baseline]);
    git(&root, &["symbolic-ref", "HEAD", "refs/heads/other"]);
    assert!(local.prepare("pull", &args, "scope", &control).is_err());
    git(&root, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    let plan = local.prepare("pull", &args, "scope", &control).unwrap();
    let before_disconnect = server.requests.lock().unwrap().len();
    github.disconnect().unwrap();
    assert!(local.execute(plan, "scope", &control).is_err());
    assert_eq!(server.requests.lock().unwrap().len(), before_disconnect);
    std::fs::write(state.join("github-connected"), b"owned").unwrap();
    let checked = root.clone();
    local.transition_hook = Some(Arc::new(move |phase, _| {
        if phase == "before_files" {
            for path in [
                ".git/HEAD.lock",
                ".git/refs/heads/main.lock",
                ".git/refs/remotes/origin/main.lock",
                ".git/index.lock",
            ] {
                assert!(checked.join(path).exists(), "held {path}");
            }
        }
    }));
    let plan = local.prepare("pull", &args, "scope", &control).unwrap();
    let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    let result = local.execute(plan, "scope", &control).unwrap();
    assert_eq!(result["operation"], "pull");
    assert_eq!(result["fast_forward"], true);
    assert_eq!(result["outcome"], "applied");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), target);
    assert_eq!(git(&root, &["symbolic-ref", "HEAD"]), "refs/heads/main");
    assert_eq!(
        std::fs::read(root.join("notes")).unwrap(),
        b"reviewed remote bytes\n"
    );
    assert_eq!(
        std::fs::read(root.join("new")).unwrap(),
        b"reviewed new file\n"
    );
    assert_eq!(git(&root, &["rev-parse", ":unrelated"]), staged);
    assert_eq!(
        std::fs::read(root.join("unrelated")).unwrap(),
        b"later user dirty bytes\r\n"
    );
    assert_eq!(std::fs::read(root.join("untracked")).unwrap(), b"keep me");
    assert_eq!(std::fs::read(root.join(".git/config")).unwrap(), config);
    assert_eq!(std::fs::read(recovery.join("index-before")).unwrap(), index);
    assert!(recovery.join("completed.json").exists());
    assert!(!root.join(".git/FETCH_HEAD").exists());
    let before_index = std::fs::read(root.join(".git/index")).unwrap();
    let plan = local.prepare("pull", &args, "scope", &control).unwrap();
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap()["outcome"],
        "unchanged"
    );
    assert_eq!(
        std::fs::read(root.join(".git/index")).unwrap(),
        before_index
    );
    // A late user writer during the native batch is never overwritten. Git
    // aborts its prepared ref transaction; retained originals block blind replay.
    std::fs::write(source.join("notes"), b"next reviewed remote bytes\n").unwrap();
    git(&source, &["add", "notes"]);
    git(
        &source,
        &["commit", "--quiet", "-m", "next remote descendant"],
    );
    let next = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    let args = json!({"repo":"owned/repo","branch":"main","expected_remote_sha":next});
    let plan = local.prepare("fetch", &args, "scope", &control).unwrap();
    local.execute(plan, "scope", &control).unwrap();
    let raced = root.clone();
    local.transition_hook = Some(Arc::new(move |phase, path| {
        if phase == "after_displace" && path == "notes" {
            std::fs::write(raced.join("notes"), b"concurrent user writer").unwrap();
        }
    }));
    let plan = local.prepare("pull", &args, "scope", &control).unwrap();
    let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap_err(),
        DomainError::DurabilityUnknown
    );
    assert_eq!(git(&root, &["rev-parse", "HEAD"]), target);
    assert_eq!(
        std::fs::read(root.join(".git/index")).unwrap(),
        before_index
    );
    assert_eq!(
        std::fs::read(root.join("notes")).unwrap(),
        b"concurrent user writer"
    );
    assert!(!recovery.join("completed.json").exists());
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(recovery.join("manifest.json")).unwrap()).unwrap();
    let backup = manifest["entries"][0]["backup"].as_str().unwrap();
    assert_eq!(
        std::fs::read(root.join(backup)).unwrap(),
        b"reviewed remote bytes\n"
    );
    for path in [
        ".git/HEAD.lock",
        ".git/refs/heads/main.lock",
        ".git/refs/remotes/origin/main.lock",
        ".git/index.lock",
    ] {
        assert!(!root.join(path).exists(), "owned lock cleaned {path}");
    }
    assert!(local.prepare("pull", &args, "scope", &control).is_err());
    assert!(
        local
            .prepare("status", &json!({"repo":"owned/repo"}), "scope", &control)
            .is_ok()
    );
    // Another project host granting this same folder shares private custody.
    // Deleting its untrusted workspace marker cannot evade the pending batch.
    std::fs::remove_file(root.join(".git/ag1-transition.json")).unwrap();
    let other = LocalGit::new(
        WorkspaceService::new().unwrap().register(&root).unwrap(),
        &helper,
        Path::new("/usr/bin/git"),
        Path::new("/usr/bin/bwrap"),
        &state.join("other-host-local-git"),
    )
    .unwrap()
    .with_github(github);
    assert!(
        other
            .prepare(
                "stage",
                &json!({"repo":"owned/repo","paths":["notes"]}),
                "other-host-scope",
                &control
            )
            .is_err()
    );
    let status_plan = other
        .prepare(
            "recovery_status",
            &json!({"repo":"owned/repo"}),
            "other-host-scope",
            &control,
        )
        .unwrap();
    let status = other
        .execute(status_plan, "other-host-scope", &control)
        .unwrap();
    let repair_args =
        json!({"repo":"owned/repo","recovery_id":status["recovery_id"],"choice":"finish"});
    assert!(
        other
            .prepare("recover", &repair_args, "other-host-scope", &control)
            .is_err(),
        "a later writer cannot be discarded by recovery"
    );
    std::fs::rename(root.join("notes"), root.join("retained-user-notes")).unwrap();
    let requests = server.requests.lock().unwrap().len();
    let plan = other
        .prepare("recover", &repair_args, "other-host-scope", &control)
        .unwrap();
    let result = other.execute(plan, "other-host-scope", &control).unwrap();
    assert_eq!(result["operation"], "recover");
    assert_eq!(result["commit"], next);
    assert_eq!(
        std::fs::read(root.join("notes")).unwrap(),
        b"next reviewed remote bytes\n"
    );
    assert_eq!(
        std::fs::read(root.join("retained-user-notes")).unwrap(),
        b"concurrent user writer"
    );
    assert_eq!(git(&root, &["rev-parse", ":unrelated"]), staged);
    assert_eq!(
        server.requests.lock().unwrap().len(),
        requests,
        "recovery is local, not another remote transfer"
    );
}

#[test]
fn clone_uses_exact_owned_https_and_publishes_only_a_new_directory() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("parent");
    let source = temp.path().join("source");
    let remote = temp.path().join("repo.git");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "--quiet", "-b", "main"]);
    git(
        temp.path(),
        &["init", "--bare", "--quiet", remote.to_str().unwrap()],
    );
    std::fs::write(source.join("notes.txt"), b"exact bytes\r\n\0\xff").unwrap();
    std::fs::write(source.join("run.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(
        source.join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("notes.txt", source.join("link")).unwrap();
    std::fs::write(
        source.join(".gitattributes"),
        b"*.txt filter=unexpected text eol=lf\n",
    )
    .unwrap();
    // Retain the exact CRLF blob in the source index; ordinary git add would
    // normalize it before this test ever reaches clone.
    std::fs::write(
        source.join(".git/info/attributes"),
        b"* -filter -text -ident -working-tree-encoding\n",
    )
    .unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "owned clone fixture"]);
    let sha = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "push",
            "--quiet",
            remote.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    std::fs::write(root.join("user.txt"), b"preserve parent bytes").unwrap();
    std::fs::create_dir(root.join("nested")).unwrap();
    let cert = root.join("fixture-cert.pem");
    let key = temp.path().join("key.pem");
    assert!(
        Command::new("/usr/bin/openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "subjectAltName=IP:127.0.0.1",
                "-keyout"
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .output()
            .unwrap()
            .status
            .success()
    );
    let server = server(temp.path(), &cert, &key);
    let state = temp.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::write(state.join("github-connected"), b"owned").unwrap();
    let github = Arc::new(crate::github::service::Service::new(
        state.clone(),
        Arc::new(Auth),
    ));
    let helper = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("ag1-process-host");
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let mut local = LocalGit::new(
        workspace,
        &helper,
        Path::new("/usr/bin/git"),
        Path::new("/usr/bin/bwrap"),
        &state.join("git"),
    )
    .unwrap()
    .with_github(github);
    local.network_fixture=Some((server.url.clone(),b"\n[http]\nsslCAInfo = /workspace/fixture-cert.pem\nextraHeader = Authorization: Basic eC1hY2Nlc3MtdG9rZW46b3duZWQtc3ludGhldGljLXRva2Vu\n".to_vec()));
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(60),
    };
    let args = json!({"repo":"owned/repo","branch":"main","expected_remote_sha":sha,"destination":"nested/cloned"});
    for destination in [
        "/outside",
        "../outside",
        "nested/../outside",
        ".git/new",
        "nested//new",
        "nested/",
    ] {
        let mut invalid = args.clone();
        invalid["destination"] = json!(destination);
        assert!(local.prepare("clone", &invalid, "scope", &control).is_err());
    }
    let plan = local.prepare("clone", &args, "scope", &control).unwrap();
    let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    assert!(!recovery.exists());
    assert!(!root.join("nested/cloned").exists());
    assert!(server.requests.lock().unwrap().is_empty());
    let result = local.execute(plan, "scope", &control).unwrap();
    assert_eq!(result["commit"], sha);
    assert_eq!(result["files"], 4);
    let cloned = root.join("nested/cloned");
    assert!(!recovery.exists());
    assert_eq!(git(&cloned, &["rev-parse", "HEAD"]), sha);
    assert_eq!(
        git(&cloned, &["rev-parse", "refs/remotes/origin/main"]),
        sha
    );
    assert_eq!(
        std::fs::read(cloned.join("notes.txt")).unwrap(),
        b"exact bytes\r\n\0\xff"
    );
    assert_eq!(
        std::fs::read_link(cloned.join("link")).unwrap(),
        Path::new("notes.txt")
    );
    assert_ne!(
        std::fs::metadata(cloned.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        git(&cloned, &["remote", "get-url", "origin"]),
        "https://github.com/owned/repo.git"
    );
    let config = std::fs::read_to_string(cloned.join(".git/config")).unwrap();
    assert!(!config.contains("Authorization"));
    assert!(!config.contains("synthetic"));
    assert!(local.prepare("clone", &args, "scope", &control).is_err());
    // A destination appearing after review remains completely untouched.
    let mut race_args = args.clone();
    race_args["destination"] = json!("racing");
    let plan = local
        .prepare("clone", &race_args, "scope", &control)
        .unwrap();
    let before = server.requests.lock().unwrap().len();
    std::fs::create_dir(root.join("racing")).unwrap();
    std::fs::write(root.join("racing/user"), b"later writer").unwrap();
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap_err(),
        DomainError::Conflict
    );
    assert_eq!(server.requests.lock().unwrap().len(), before);
    assert_eq!(
        std::fs::read(root.join("racing/user")).unwrap(),
        b"later writer"
    );
    // Parent replacement invalidates the held native identity before traffic.
    race_args["destination"] = json!("nested/other");
    let plan = local
        .prepare("clone", &race_args, "scope", &control)
        .unwrap();
    std::fs::rename(root.join("nested"), root.join("retained-parent")).unwrap();
    std::fs::create_dir(root.join("nested")).unwrap();
    assert!(local.execute(plan, "scope", &control).is_err());
    assert_eq!(server.requests.lock().unwrap().len(), before);
    assert!(!root.join("nested/other").exists());
    // A stale advertised head retains a named recovery directory, never a
    // falsely completed destination, and never deletes staging automatically.
    race_args["destination"] = json!("stale");
    race_args["expected_remote_sha"] = json!("a".repeat(40));
    let plan = local
        .prepare("clone", &race_args, "scope", &control)
        .unwrap();
    let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap_err(),
        DomainError::DurabilityUnknown
    );
    assert!(recovery.join(".git/config").exists());
    assert!(!root.join("stale").exists());
    // The final native rename is NOREPLACE even if the target is created
    // during the object transfer, after every pre-transfer target check.
    race_args["destination"] = json!("late-target");
    race_args["expected_remote_sha"] = json!(sha);
    let plan = local
        .prepare("clone", &race_args, "scope", &control)
        .unwrap();
    let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    let late = root.join("late-target");
    *server.transfer_hook.lock().unwrap() = Some(Box::new(move || {
        std::fs::create_dir(&late).unwrap();
        std::fs::write(late.join("user"), b"late bytes").unwrap();
    }));
    assert_eq!(
        local.execute(plan, "scope", &control).unwrap_err(),
        DomainError::DurabilityUnknown
    );
    assert_eq!(
        std::fs::read(root.join("late-target/user")).unwrap(),
        b"late bytes"
    );
    assert!(!root.join("late-target/.git").exists());
    assert_eq!(git(&recovery, &["rev-parse", "HEAD"]), sha);
    // Cancellation during the actual owned HTTPS transfer is terminal,
    // retains recovery and reaps the native children without publication.
    let interrupted = Arc::new(AtomicBool::new(false));
    let signal = interrupted.clone();
    let interrupted_control = ScanControl {
        cancel: &interrupted,
        deadline: control.deadline,
    };
    race_args["destination"] = json!("cancelled");
    let plan = local
        .prepare("clone", &race_args, "scope", &interrupted_control)
        .unwrap();
    let recovery = PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    *server.transfer_hook.lock().unwrap() =
        Some(Box::new(move || signal.store(true, Ordering::Release)));
    assert_eq!(
        local
            .execute(plan, "scope", &interrupted_control)
            .unwrap_err(),
        DomainError::DurabilityUnknown
    );
    assert!(recovery.is_dir());
    assert!(!root.join("cancelled").exists());
    assert_eq!(
        std::fs::read(root.join("user.txt")).unwrap(),
        b"preserve parent bytes"
    );
    assert!(!root.join(".git").exists());
    assert!(
        std::fs::read_dir(state.join("git")).unwrap().all(|p| !p
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".log"))
    );
}
