//! Shared native GitHub account lifecycle and legacy desktop operations.
//! Credentials stay in GitHub CLI's credential
//! store and never enter a model prompt, renderer reply, command argument or log.
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, String>;
pub const REST_VERSION: &str = "2026-03-10";

#[cfg(test)]
pub(crate) fn fixture_openssl() -> Command {
    // TLS fixtures generate a throwaway certificate. The installed app never
    // uses OpenSSL, but Windows development machines commonly get it from Git.
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("AG1_TEST_OPENSSL") {
        candidates.push(PathBuf::from(path));
    }
    #[cfg(windows)]
    {
        candidates.push(PathBuf::from(r"C:\Program Files\Git\usr\bin\openssl.exe"));
        candidates.push(PathBuf::from(r"C:\Program Files\Git\mingw64\bin\openssl.exe"));
    }
    #[cfg(not(windows))]
    candidates.push(PathBuf::from("/usr/bin/openssl"));

    for candidate in candidates {
        if candidate.is_file() {
            return Command::new(candidate);
        }
    }
    // Keep a normal PATH lookup as the final option for custom installations.
    Command::new("openssl")
}

#[cfg(test)]
mod acceptance_tests;
pub mod browse;
mod credential_fields;
pub mod downloads;
mod graph_mutations;
mod graph_reads;
#[cfg(target_os = "linux")]
pub mod local;
mod model_contract;
pub mod operations;
mod pull_responses;
pub mod schemas;
pub mod secrets;
pub mod service;
pub mod tool;
pub mod transport;
pub mod uploads;
mod workflow_responses;
#[derive(Default)]
pub struct GitHub {
    closing: AtomicBool,
    service: Mutex<Option<Arc<service::Service>>>,
    login: Arc<Mutex<Login>>,
    writes: Mutex<()>,
    cancel_login: Arc<AtomicBool>,
}
#[derive(Default)]
struct Login {
    running: bool,
    code: Option<String>,
    error: Option<String>,
}
fn own_group(_cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        _cmd.process_group(0);
    }
}
fn stop_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
    let _ = child.wait();
}
impl Drop for GitHub {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn gh() -> Command {
    let mut c = Command::new("gh");
    own_group(&mut c);
    c.env("GH_HOST", "github.com")
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1")
        .env("GH_BROWSER", "true");
    for key in [
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
        "GH_REPO",
        "GH_DEBUG",
        "GH_FORCE_TTY",
    ] {
        c.env_remove(key);
    }
    c
}
fn output(mut cmd: Command, input: Option<&[u8]>, timeout: Duration) -> Result<Vec<u8>> {
    own_group(&mut cmd);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "GitHub CLI is unavailable. Install gh, then reopen AG1.".to_string())?;
    if let Some(mut pipe) = child.stdin.take() {
        if let Some(bytes) = input {
            pipe.write_all(bytes)
                .map_err(|_| "Cannot send GitHub request".to_string())?;
        }
    }
    let read = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = vec![];
            let mut buf = [0; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                if bytes.len() < 4 * 1024 * 1024 {
                    bytes.extend_from_slice(&buf[..n.min(4 * 1024 * 1024 - bytes.len())]);
                }
            }
            bytes
        })
    };
    let out = read(Box::new(child.stdout.take().unwrap()));
    let err = read(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| "Cannot inspect GitHub request")?
        {
            break status;
        }
        if started.elapsed() > timeout {
            stop_child(&mut child);
            return Err("GitHub request timed out. Check GitHub before repeating a write.".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let bytes = out.join().map_err(|_| "GitHub output interrupted")?;
    let errors = err.join().map_err(|_| "GitHub output interrupted")?;
    if !status.success() {
        let text = String::from_utf8_lossy(&errors);
        // Never reflect arbitrary authentication/helper output into the renderer.
        return Err(if text.contains("401") || text.contains("auth login") { "Sign in to GitHub again." } else if text.contains("403") { "GitHub denied this request. Check repository permissions or rate limits." } else if text.contains("404") { "Repository or item not found, or your account cannot access it." } else if text.contains("422") { "GitHub could not apply this change. Check the branch, fields, and existing items." } else { "GitHub could not complete the request. Check your connection and repository permissions." }.into());
    }
    if bytes.len() >= 4 * 1024 * 1024 {
        return Err("GitHub response is too large; open the item on GitHub.".into());
    }
    Ok(bytes)
}
fn api(endpoint: &str, method: &str, body: Option<Value>) -> Result<Value> {
    let mut c = gh();
    c.args([
        "api",
        "--hostname",
        "github.com",
        "--method",
        method,
        "-H",
        "Accept: application/vnd.github+json",
        "-H",
        "X-GitHub-Api-Version: 2026-03-10",
        endpoint,
    ]);
    let body = body.map(|v| serde_json::to_vec(&v).unwrap());
    if body.is_some() {
        c.args(["--input", "-"]);
    }
    let bytes = output(c, body.as_deref(), Duration::from_secs(45))?;
    if bytes.is_empty() {
        Ok(json!({"ok":true}))
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "Invalid GitHub response".into())
    }
}
fn repo(args: &Value) -> Result<String> {
    let v = args["repo"].as_str().unwrap_or("");
    let parts: Vec<_> = v.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.starts_with('-')
                || p.len() > 100
                || !p
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return Err("Choose an owner/repository.".into());
    }
    Ok(v.into())
}
fn text(args: &Value, key: &str, max: usize) -> Result<String> {
    args[key]
        .as_str()
        .filter(|v| !v.trim().is_empty() && v.len() <= max && !v.contains('\0'))
        .map(str::to_owned)
        .ok_or_else(|| format!("Enter a valid {key}."))
}
fn number(args: &Value) -> Result<u64> {
    args["number"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(|| "Select an issue or pull request.".into())
}
fn page(args: &Value) -> u64 {
    args["page"].as_u64().unwrap_or(1).clamp(1, 1000)
}
fn connected(state: &Path) -> bool {
    state.join("github-connected").is_file()
}
fn mark(state: &Path) -> Result<()> {
    let generation =
        crate::random_nonce().map_err(|_| "Cannot create GitHub connection identity")?;
    let path = state.join("github-connected");

    #[cfg(target_os = "linux")]
    use std::os::unix::fs::OpenOptionsExt;

    #[cfg(target_os = "linux")]
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|_| "Cannot save GitHub connection")?;

    #[cfg(windows)]
    let mut file = {
        // The state folder is private to AG1W. Reject a pre-existing reparse
        // point before opening it so the durable connection marker cannot be
        // redirected somewhere outside that folder.
        if fs::symlink_metadata(&path)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err("Cannot save GitHub connection".into());
        }
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|_| "Cannot save GitHub connection")?
    };

    file.write_all(format!("github.com\n{generation}\n").as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|_| "Cannot save GitHub connection".into())
}
impl GitHub {
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        self.cancel_login.store(true, Ordering::Release);
        if let Ok(slot) = self.service.lock()
            && let Some(service) = slot.as_ref()
        {
            service.shutdown();
        }
    }
    pub fn wait_stopped(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(3);
        if let Some(service) = self
            .service
            .lock()
            .map_err(|_| "GitHub service busy")?
            .clone()
        {
            service
                .wait_stopped(deadline)
                .map_err(|e| e.message.to_string())?;
        }
        while self
            .login
            .lock()
            .map_err(|_| "GitHub login control busy")?
            .running
        {
            if Instant::now() >= deadline {
                return Err(
                    "GitHub sign-in cleanup is still in progress. Wait before quitting again."
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
    pub fn with_service(service: Arc<service::Service>) -> Self {
        let mut result = Self::default();
        result.service = Mutex::new(Some(service));
        result
    }
    pub fn shared(&self, state: &Path) -> Result<Arc<service::Service>> {
        let mut slot = self.service.lock().map_err(|_| "GitHub service busy")?;
        if slot.is_none() {
            *slot = Some(service::Service::shared(state).map_err(|e| e.message.to_string())?);
        }
        slot.clone()
            .ok_or_else(|| "GitHub service unavailable".into())
    }
    pub fn request(
        &self,
        state: &Path,
        action: &str,
        args: Value,
        _workspace: Option<PathBuf>,
    ) -> Result<Value> {
        if self.closing.load(Ordering::Acquire) {
            return Err("GitHub is shutting down.".into());
        }
        if action == "status" {
            let l = self.login.lock().map_err(|_| "GitHub connection busy")?;
            if l.running {
                return Ok(
                    json!({"state":"connecting","code":l.code,"url":"https://github.com/login/device"}),
                );
            }
            if !connected(state) {
                return Ok(
                    json!({"state":"disconnected","available":gh().arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s|s.success()),"error":l.error}),
                );
            }
            drop(l);
            let cancel = AtomicBool::new(false);
            let user = self
                .shared(state)?
                .account_view(&crate::resource_fs::ScanControl {
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(45),
                })
                .map_err(|e| e.message.to_string())?;
            return Ok(
                json!({"state":"connected","login":user["login"],"name":user["name"],"url":user["html_url"]}),
            );
        }
        if action == "cancel_login" {
            self.cancel_login.store(true, Ordering::Release);
            return Ok(json!({"state":"connecting"}));
        }
        if action == "connect" {
            let mut l = self.login.lock().map_err(|_| "GitHub connection busy")?;
            if l.running {
                return Ok(json!({"state":"connecting"}));
            }
            self.cancel_login.store(false, Ordering::Release);
            let cancel = self.cancel_login.clone();
            l.running = true;
            l.code = None;
            l.error = None;
            let login = self.login.clone();
            let state = state.to_owned();
            std::thread::spawn(move || {
                let result = (|| -> Result<()> {
                    if api("user", "GET", None).is_ok() {
                        if cancel.load(Ordering::Acquire) {
                            return Err("Sign-in cancelled.".into());
                        }
                        return mark(&state);
                    }
                    if cancel.load(Ordering::Acquire) {
                        return Err("Sign-in cancelled.".into());
                    }
                    let mut c = gh();
                    c.args([
                        "auth",
                        "login",
                        "--hostname",
                        "github.com",
                        "--git-protocol",
                        "https",
                        "--web",
                        "--skip-ssh-key",
                    ]);
                    let mut child = c
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::piped())
                        .spawn()
                        .map_err(|_| "Install GitHub CLI to connect your account".to_string())?;
                    let mut pipe = child.stderr.take().unwrap();
                    let shared = login.clone();
                    let reader = std::thread::spawn(move || {
                        let mut all = String::new();
                        let mut bytes = [0; 1024];
                        while let Ok(n) = pipe.read(&mut bytes) {
                            if n == 0 {
                                break;
                            }
                            if all.len() < 16384 {
                                all.push_str(&String::from_utf8_lossy(&bytes[..n]));
                            }
                            for word in all.split_whitespace() {
                                let word = word
                                    .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                                if word.len() == 9
                                    && word.as_bytes()[4] == b'-'
                                    && word.bytes().enumerate().all(|(i, b)| {
                                        i == 4 || b.is_ascii_uppercase() || b.is_ascii_digit()
                                    })
                                {
                                    if let Ok(mut s) = shared.lock() {
                                        s.code = Some(word.into());
                                    }
                                }
                            }
                        }
                    });
                    let started = Instant::now();
                    loop {
                        if let Some(status) = child
                            .try_wait()
                            .map_err(|_| "Cannot inspect GitHub sign-in")?
                        {
                            let _ = reader.join();
                            if !status.success() {
                                return Err("Sign-in did not complete. Try again, or check GitHub CLI authentication.".into());
                            }
                            break;
                        }
                        if cancel.load(Ordering::Acquire) {
                            stop_child(&mut child);
                            return Err("Sign-in cancelled.".into());
                        }
                        if started.elapsed() > Duration::from_secs(900) {
                            stop_child(&mut child);
                            return Err("Sign-in expired. Connect again for a new code.".into());
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    api("user", "GET", None)?;
                    if cancel.load(Ordering::Acquire) {
                        return Err("Sign-in cancelled.".into());
                    }
                    mark(&state)
                })();
                if let Ok(mut l) = login.lock() {
                    l.running = false;
                    l.error = result.err();
                }
            });
            return Ok(json!({"state":"connecting"}));
        }
        if action == "disconnect" {
            let _guard = self.writes.lock().map_err(|_| "GitHub busy")?;
            if self.login.lock().map_err(|_| "GitHub busy")?.running {
                return Err("Finish the current sign-in first.".into());
            }
            self.shared(state)?
                .disconnect()
                .map_err(|e| e.message.to_string())?;
            return Ok(json!({"state":"disconnected"}));
        }
        if action == "open_url" {
            let url = text(&args, "url", 2048)?;
            if !(url == "https://github.com/login/device"
                || url.starts_with("https://github.com/") && !url.contains(['\n', '\r', '\\']))
            {
                return Err("Only GitHub links can open from this panel".into());
            }
            #[cfg(target_os = "linux")]
            let mut c = Command::new("xdg-open");
            #[cfg(target_os = "macos")]
            let mut c = Command::new("open");
            #[cfg(target_os = "windows")]
            let mut c = Command::new("explorer.exe");
            c.arg(url)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| "Cannot open your browser")?;
            return Ok(json!({"opened":true}));
        }
        if !connected(state) {
            return Err("Connect your GitHub account first.".into());
        }
        if ["local", "push", "clone", "checkout"].contains(&action) {
            return Err(
                "Use the shared Local Git panel and its native exact-target confirmation.".into(),
            );
        }
        if ["create_issue", "create_pr", "comment", "review", "merge"].contains(&action) {
            return Err(
                "Review this change through the native GitHub confirmation service.".into(),
            );
        }
        // Serialize deliberate writes, including disconnect, to prevent double-click publication.
        let writing = matches!(
            action,
            "create_issue"
                | "create_pr"
                | "comment"
                | "review"
                | "merge"
                | "clone"
                | "checkout"
                | "push"
        );
        let _guard = if writing {
            Some(
                self.writes
                    .try_lock()
                    .map_err(|_| "A GitHub change is already in progress")?,
            )
        } else {
            None
        };
        if !connected(state) {
            return Err("GitHub was disconnected.".into());
        }
        match action {
            "repos" => {
                let v = api(
                    &format!("user/repos?sort=pushed&per_page=30&page={}", page(&args)),
                    "GET",
                    None,
                )?;
                Ok(
                    json!({"items":v.as_array().unwrap_or(&vec![]).iter().map(|r|json!({"name":r["full_name"],"description":r["description"],"private":r["private"],"branch":r["default_branch"],"url":r["html_url"]})).collect::<Vec<_>>()}),
                )
            }
            "issues" => {
                let r = repo(&args)?;
                let rows = api(
                    &format!(
                        "repos/{r}/issues?state=open&per_page=30&page={}",
                        page(&args)
                    ),
                    "GET",
                    None,
                )?;
                let rows = rows.as_array().ok_or("Invalid issue list")?;
                Ok(
                    json!({"items":rows.iter().filter(|i|i.get("pull_request").is_none()).collect::<Vec<_>>(),"has_more":rows.len()==30}),
                )
            }
            "pulls" | "runs" => {
                let r = repo(&args)?;
                let path = if action == "runs" {
                    "actions/runs"
                } else {
                    action
                };
                api(
                    &format!(
                        "repos/{r}/{path}?state=open&per_page=30&page={}",
                        page(&args)
                    ),
                    "GET",
                    None,
                )
            }
            "item" => {
                let r = repo(&args)?;
                let n = number(&args)?;
                let kind = if args["pull"] == true {
                    "pulls"
                } else {
                    "issues"
                };
                let item = api(&format!("repos/{r}/{kind}/{n}"), "GET", None)?;
                let comments = api(
                    &format!("repos/{r}/issues/{n}/comments?per_page=30"),
                    "GET",
                    None,
                )?;
                Ok(json!({"item":item,"comments":comments}))
            }
            "files" => {
                let r = repo(&args)?;
                let n = number(&args)?;
                api(
                    &format!("repos/{r}/pulls/{n}/files?per_page=30&page={}", page(&args)),
                    "GET",
                    None,
                )
            }
            "create_issue" => {
                let r = repo(&args)?;
                api(
                    &format!("repos/{r}/issues"),
                    "POST",
                    Some(
                        json!({"title":text(&args,"title",256)?,"body":text(&args,"body",60000)?}),
                    ),
                )
            }
            "create_pr" => {
                let r = repo(&args)?;
                api(
                    &format!("repos/{r}/pulls"),
                    "POST",
                    Some(
                        json!({"title":text(&args,"title",256)?,"body":text(&args,"body",60000)?,"head":text(&args,"head",256)?,"base":text(&args,"base",256)?,"draft":args["draft"].as_bool().unwrap_or(true)}),
                    ),
                )
            }
            "comment" => {
                let r = repo(&args)?;
                let n = number(&args)?;
                api(
                    &format!("repos/{r}/issues/{n}/comments"),
                    "POST",
                    Some(json!({"body":text(&args,"body",60000)?})),
                )
            }
            "review" => {
                let r = repo(&args)?;
                let n = number(&args)?;
                let event = text(&args, "event", 32)?;
                if !["APPROVE", "REQUEST_CHANGES", "COMMENT"].contains(&event.as_str()) {
                    return Err("Choose a review type".into());
                }
                api(
                    &format!("repos/{r}/pulls/{n}/reviews"),
                    "POST",
                    Some(json!({"event":event,"body":text(&args,"body",60000)?})),
                )
            }
            "merge" => {
                let r = repo(&args)?;
                let n = number(&args)?;
                let sha = text(&args, "sha", 64)?;
                if !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("Refresh the pull request before merging".into());
                }
                {
                    let result = api(
                        &format!("repos/{r}/pulls/{n}/merge"),
                        "PUT",
                        Some(json!({"sha":sha,"merge_method":"squash"})),
                    )?;
                    if result["merged"] != true {
                        return Err("GitHub did not merge this pull request. Review its checks and merge requirements.".into());
                    }
                    Ok(result)
                }
            }
            _ => Err("Unknown GitHub action".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_local_git_actions_cannot_bypass_shared_native_custody() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("github-connected"), b"owned marker").unwrap();
        let client = GitHub::default();
        for action in ["local", "push", "clone", "checkout"] {
            assert!(
                client
                    .request(dir.path(), action, json!({"repo":"owned/repo"}), None)
                    .unwrap_err()
                    .contains("shared Local Git")
            );
        }
    }
    #[test]
    fn quit_blocks_new_sign_in_without_removing_the_connection_marker() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("github-connected"), "saved-generation").unwrap();
        let client = GitHub::default();
        client.shutdown();
        client.wait_stopped().unwrap();
        assert!(
            client
                .request(dir.path(), "connect", json!({}), None)
                .unwrap_err()
                .contains("shutting down")
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("github-connected")).unwrap(),
            "saved-generation"
        );
    }
    #[test]
    fn repository_targets_are_not_commands_or_urls() {
        for r in [
            "a/b;rm",
            "a/../b",
            "https://github.com/a/b",
            "-r/evil",
            "a/b?x",
            "a/b\n",
        ] {
            assert!(repo(&json!({"repo":r})).is_err());
        }
        assert_eq!(
            repo(&json!({"repo":"owner/my.repo"})).unwrap(),
            "owner/my.repo"
        );
    }
    #[test]
    fn disconnected_cannot_read_or_write_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let client = GitHub::default();
        assert!(
            client
                .request(
                    dir.path(),
                    "create_issue",
                    json!({"repo":"a/b","title":"x","body":"x"}),
                    None
                )
                .is_err()
        );
    }
}
