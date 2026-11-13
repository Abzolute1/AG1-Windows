#![cfg(target_os = "linux")]
use ag1_core::DomainError;
use ag1_platform::{
    github::local::LocalGit, resource_fs::ScanControl, workspace::WorkspaceService,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("/usr/bin/git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(root)
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
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
struct Fixture {
    temp: tempfile::TempDir,
    local: LocalGit,
    cancel: AtomicBool,
}

#[test]
fn reviewed_checkout_preserves_unrelated_dirty_and_staged_bytes_and_retains_recovery() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let root = f.root();
    let original_branch = git(&root, &["branch", "--show-current"]).trim().to_string();
    std::fs::write(root.join("keep.txt"), b"baseline keep\n").unwrap();
    git(&root, &["add", "keep.txt"]);
    git(&root, &["commit", "--quiet", "-m", "unrelated baseline"]);
    let original = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    git(&root, &["checkout", "--quiet", "-b", "reviewed"]);
    std::fs::write(root.join("notes.txt"), b"target\r\n\0\xff").unwrap();
    std::fs::write(root.join("run.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(root.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    symlink("notes.txt", root.join("link")).unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "target"]);
    let target = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    git(&root, &["checkout", "--quiet", &original_branch]);
    std::fs::write(root.join("keep.txt"), b"unrelated staged\n").unwrap();
    git(&root, &["add", "keep.txt"]);
    std::fs::write(root.join("keep.txt"), b"later unrelated work\r\n").unwrap();
    std::fs::write(root.join("untracked.txt"), b"untracked preserved").unwrap();
    let config = std::fs::read(root.join(".git/config")).unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let plan = f
        .local
        .prepare(
            "checkout",
            &json!({"repo":"owned/repo","branch":"reviewed","target_sha":target}),
            "scope",
            &f.control(),
        )
        .unwrap();
    let recovery = std::path::PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
    assert!(!recovery.exists());
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), original);
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    let result = f.local.execute(plan, "scope", &f.control()).unwrap();
    assert_eq!(result["outcome"], "applied");
    assert_eq!(git(&root, &["branch", "--show-current"]).trim(), "reviewed");
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), target);
    assert_eq!(
        std::fs::read(root.join("notes.txt")).unwrap(),
        b"target\r\n\0\xff"
    );
    assert_eq!(
        std::fs::read_link(root.join("link")).unwrap(),
        Path::new("notes.txt")
    );
    assert_ne!(
        std::fs::metadata(root.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        std::fs::read(root.join("keep.txt")).unwrap(),
        b"later unrelated work\r\n"
    );
    assert_eq!(git(&root, &["show", ":keep.txt"]), "unrelated staged\n");
    assert_eq!(
        std::fs::read(root.join("untracked.txt")).unwrap(),
        b"untracked preserved"
    );
    assert_eq!(std::fs::read(root.join(".git/config")).unwrap(), config);
    assert_eq!(std::fs::read(recovery.join("index-before")).unwrap(), index);
    assert!(recovery.join("completed.json").exists());
    assert!(!root.join(".git/index.lock").exists());
    assert!(!root.join(".git/HEAD.lock").exists());
    let back = f
        .local
        .prepare(
            "checkout",
            &json!({"repo":"owned/repo","branch":original_branch,"target_sha":original}),
            "scope",
            &f.control(),
        )
        .unwrap();
    f.local.execute(back, "scope", &f.control()).unwrap();
    assert_eq!(
        std::fs::read(root.join("notes.txt")).unwrap(),
        b"original\n"
    );
    assert!(!root.join("link").exists());
    assert_eq!(
        std::fs::read(root.join("keep.txt")).unwrap(),
        b"later unrelated work\r\n"
    );
    assert_eq!(git(&root, &["show", ":keep.txt"]), "unrelated staged\n");
}

#[test]
fn checkout_handles_directory_transitions_without_discarding_untracked_collisions() {
    let f = Fixture::new();
    let root = f.root();
    let branch = git(&root, &["branch", "--show-current"]).trim().to_string();
    std::fs::create_dir_all(root.join("folder/nested")).unwrap();
    std::fs::write(root.join("folder/nested/old"), b"old nested").unwrap();
    std::fs::write(root.join("plain"), b"old plain").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "directory baseline"]);
    let from = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    git(&root, &["checkout", "--quiet", "-b", "shape"]);
    git(&root, &["rm", "-r", "--quiet", "folder", "plain"]);
    std::fs::write(root.join("folder"), b"new file").unwrap();
    std::fs::create_dir(root.join("plain")).unwrap();
    std::fs::write(root.join("plain/new"), b"new nested").unwrap();
    git(&root, &["add", "--all"]);
    git(&root, &["commit", "--quiet", "-m", "shape target"]);
    let target = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
    git(&root, &["checkout", "--quiet", &branch]);
    let args = json!({"repo":"owned/repo","branch":"shape","target_sha":target});
    std::fs::write(root.join("folder/ignored-user"), b"keep me").unwrap();
    assert!(
        f.local
            .prepare("checkout", &args, "scope", &f.control())
            .is_err()
    );
    assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), from);
    assert_eq!(
        std::fs::read(root.join("folder/nested/old")).unwrap(),
        b"old nested"
    );
    std::fs::remove_file(root.join("folder/ignored-user")).unwrap();
    let plan = f
        .local
        .prepare("checkout", &args, "scope", &f.control())
        .unwrap();
    f.local.execute(plan, "scope", &f.control()).unwrap();
    assert_eq!(std::fs::read(root.join("folder")).unwrap(), b"new file");
    assert_eq!(
        std::fs::read(root.join("plain/new")).unwrap(),
        b"new nested"
    );
}

#[test]
fn checkout_rechecks_files_index_target_scope_cancellation_worktrees_and_foreign_locks() {
    use std::sync::atomic::Ordering;
    for changed in [
        "file",
        "index",
        "target",
        "scope",
        "cancel",
        "worktree",
        "head_lock",
    ] {
        let f = Fixture::new();
        let root = f.root();
        let branch = git(&root, &["branch", "--show-current"]).trim().to_string();
        let from = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
        git(&root, &["checkout", "--quiet", "-b", "target"]);
        std::fs::write(root.join("notes.txt"), b"target\n").unwrap();
        git(&root, &["add", "notes.txt"]);
        git(&root, &["commit", "--quiet", "-m", "target"]);
        let target = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
        git(&root, &["checkout", "--quiet", &branch]);
        let plan = f
            .local
            .prepare(
                "checkout",
                &json!({"repo":"owned/repo","branch":"target","target_sha":target}),
                "scope",
                &f.control(),
            )
            .unwrap();
        let recovery = std::path::PathBuf::from(plan.preview()["recovery_path"].as_str().unwrap());
        match changed {
            "file" => std::fs::write(root.join("notes.txt"), b"later writer").unwrap(),
            "index" => {
                std::fs::write(root.join("other"), b"later staged").unwrap();
                git(&root, &["add", "other"]);
            }
            "target" => {
                git(&root, &["update-ref", "refs/heads/target", &from]);
            }
            "cancel" => f.cancel.store(true, Ordering::Release),
            "worktree" => {
                git(
                    &root,
                    &[
                        "worktree",
                        "add",
                        "--quiet",
                        f.temp.path().join("other-worktree").to_str().unwrap(),
                        "target",
                    ],
                );
            }
            "head_lock" => {
                std::fs::write(root.join(".git/HEAD.lock"), b"foreign writer lock").unwrap()
            }
            _ => {}
        }
        let file = std::fs::read(root.join("notes.txt")).unwrap();
        let index = std::fs::read(root.join(".git/index")).unwrap();
        assert!(
            f.local
                .execute(
                    plan,
                    if changed == "scope" { "other" } else { "scope" },
                    &f.control()
                )
                .is_err(),
            "{changed}"
        );
        assert!(!recovery.exists(), "{changed}");
        assert_eq!(git(&root, &["rev-parse", "HEAD"]).trim(), from);
        assert_eq!(std::fs::read(root.join("notes.txt")).unwrap(), file);
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert!(!root.join(".git/index.lock").exists());
        if changed == "head_lock" {
            assert_eq!(
                std::fs::read(root.join(".git/HEAD.lock")).unwrap(),
                b"foreign writer lock"
            );
        }
    }
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "--quiet"]);
        git(
            &root,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/owned/repo.git",
            ],
        );
        std::fs::write(root.join("notes.txt"), "original\n").unwrap();
        git(&root, &["add", "notes.txt"]);
        git(&root, &["commit", "--quiet", "-m", "baseline"]);
        let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
        let local = LocalGit::new(
            workspace,
            Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
            Path::new("/usr/bin/git"),
            Path::new("/usr/bin/bwrap"),
            &temp.path().join("state"),
        )
        .unwrap();
        Self {
            temp,
            local,
            cancel: AtomicBool::new(false),
        }
    }
    fn root(&self) -> std::path::PathBuf {
        self.temp.path().join("workspace")
    }
    fn control(&self) -> ScanControl<'_> {
        ScanControl {
            cancel: &self.cancel,
            deadline: Instant::now() + Duration::from_secs(30),
        }
    }
    fn apply(&self, action: &str, args: Value) -> Value {
        let plan = self
            .local
            .prepare(action, &args, "scope-a", &self.control())
            .unwrap();
        self.local
            .execute(plan, "scope-a", &self.control())
            .unwrap()
    }
}
#[test]
fn exact_staging_unstaging_and_commit_preserve_dirty_working_bytes() {
    let f = Fixture::new();
    let root = f.root();
    let bytes = "dirty user bytes\n".repeat(24000);
    std::fs::write(root.join("notes.txt"), &bytes).unwrap();
    std::fs::write(root.join("unrelated.txt"), "untracked stays").unwrap();
    f.apply("stage", json!({"repo":"owned/repo","paths":["notes.txt"]}));
    assert_eq!(git(&root, &["show", ":notes.txt"]), bytes);
    assert!(!root.join(".git/index.lock").exists());
    f.apply(
        "unstage",
        json!({"repo":"owned/repo","paths":["notes.txt"]}),
    );
    assert_eq!(git(&root, &["show", ":notes.txt"]), "original\n");
    f.apply("stage", json!({"repo":"owned/repo","paths":["notes.txt"]}));
    let result=f.apply("commit",json!({"repo":"owned/repo","message":"Reviewed exact bytes","author_name":"Owned fixture","author_email":"fixture@example.invalid"}));
    assert_eq!(
        git(&root, &["rev-parse", "HEAD"]).trim(),
        result["commit"].as_str().unwrap()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.txt")).unwrap(),
        bytes
    );
    assert_eq!(
        std::fs::read_to_string(root.join("unrelated.txt")).unwrap(),
        "untracked stays"
    );
    assert_eq!(git(&root, &["show", "HEAD:notes.txt"]), bytes);
    assert!(
        std::fs::read_dir(f.temp.path().join("state"))
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("git-index-recovery-"))
    );
}
#[test]
fn stale_files_indexes_scopes_and_existing_git_locks_fail_closed() {
    let f = Fixture::new();
    let root = f.root();
    let args = json!({"repo":"owned/repo","paths":["notes.txt"]});
    let plan = f
        .local
        .prepare("stage", &args, "scope-a", &f.control())
        .unwrap();
    std::fs::write(root.join("notes.txt"), "later user edit").unwrap();
    assert!(matches!(
        f.local.execute(plan, "scope-a", &f.control()),
        Err(DomainError::Conflict)
    ));
    let plan = f
        .local
        .prepare("stage", &args, "scope-a", &f.control())
        .unwrap();
    assert!(matches!(
        f.local.execute(plan, "scope-b", &f.control()),
        Err(DomainError::PermissionDenied)
    ));
    let plan = f
        .local
        .prepare("stage", &args, "scope-a", &f.control())
        .unwrap();
    std::fs::write(root.join(".git/index.lock"), "external owner").unwrap();
    assert!(matches!(
        f.local.execute(plan, "scope-a", &f.control()),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        std::fs::read_to_string(root.join(".git/index.lock")).unwrap(),
        "external owner"
    );
    assert_eq!(git(&root, &["show", ":notes.txt"]), "original\n");
    assert_eq!(
        std::fs::read_to_string(root.join("notes.txt")).unwrap(),
        "later user edit"
    );
}
#[test]
fn local_scope_rejects_remote_mismatch_symlink_ancestors_and_unbound_linked_metadata() {
    let f = Fixture::new();
    let root = f.root();
    assert!(
        f.local
            .prepare(
                "status",
                &json!({"repo":"other/repo"}),
                "scope-a",
                &f.control()
            )
            .is_err()
    );
    let outside = f.temp.path().join("outside-grant");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(
        outside.join("secret"),
        b"owned outside bytes must not be read",
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, root.join("outside")).unwrap();
    f.apply("stage", json!({"repo":"owned/repo","paths":["outside"]}));
    assert_eq!(git(&root, &["show", ":outside"]), outside.to_str().unwrap());
    assert!(git(&root, &["ls-files", "--stage", "outside"]).starts_with("120000 "));
    assert!(
        f.local
            .prepare(
                "stage",
                &json!({"repo":"owned/repo","paths":["outside/secret"]}),
                "scope-a",
                &f.control()
            )
            .is_err()
    );
    std::fs::rename(root.join(".git"), root.join(".saved-git")).unwrap();
    std::fs::write(root.join(".git"), "gitdir: .saved-git\n").unwrap();
    assert!(matches!(
        f.local.prepare(
            "status",
            &json!({"repo":"owned/repo"}),
            "scope-a",
            &f.control()
        ),
        Err(DomainError::Conflict)
    ));
}

#[test]
fn changed_index_branch_and_cancellation_cannot_consume_old_plans() {
    let f = Fixture::new();
    let root = f.root();
    let args = json!({"repo":"owned/repo","paths":["notes.txt"]});
    let plan = f
        .local
        .prepare("stage", &args, "scope-a", &f.control())
        .unwrap();
    std::fs::write(root.join("another.txt"), "independent staged work").unwrap();
    git(&root, &["add", "another.txt"]);
    let index = std::fs::read(root.join(".git/index")).unwrap();
    assert!(matches!(
        f.local.execute(plan, "scope-a", &f.control()),
        Err(DomainError::Conflict)
    ));
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    let plan = f
        .local
        .prepare(
            "branch_create",
            &json!({"repo":"owned/repo","branch":"reviewed"}),
            "scope-a",
            &f.control(),
        )
        .unwrap();
    git(&root, &["switch", "--quiet", "-c", "later"]);
    assert!(matches!(
        f.local.execute(plan, "scope-a", &f.control()),
        Err(DomainError::Conflict)
    ));
    assert!(!git(&root, &["branch", "--list"]).contains("reviewed"));
    f.apply(
        "branch_create",
        json!({"repo":"owned/repo","branch":"reviewed"}),
    );
    assert!(git(&root, &["branch", "--list"]).contains("reviewed"));
    let plan = f
        .local
        .prepare("stage", &args, "scope-a", &f.control())
        .unwrap();
    f.cancel.store(true, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        f.local.execute(plan, "scope-a", &f.control()),
        Err(DomainError::Cancelled)
    ));
    assert!(!root.join(".git/index.lock").exists());
}

#[test]
fn git_ref_transaction_locks_head_until_commit() {
    use std::io::{BufRead, BufReader, Write};
    let f = Fixture::new();
    let root = f.root();
    let head = git(&root, &["rev-parse", "HEAD"]);
    let branch = git(&root, &["symbolic-ref", "HEAD"]);
    let mut child = Command::new("/usr/bin/git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "update-ref",
            "--no-deref",
            "--stdin",
        ])
        .current_dir(&root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(
            format!(
                "start\nupdate {} {} {}\ncreate refs/heads/transaction {}\nprepare\n",
                branch.trim(),
                head.trim(),
                head.trim(),
                head.trim()
            )
            .as_bytes(),
        )
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    assert_eq!(line, "start: ok\n");
    line.clear();
    output.read_line(&mut line).unwrap();
    assert_eq!(line, "prepare: ok\n");
    assert!(root.join(".git/HEAD.lock").exists());
    assert_eq!(git(&root, &["symbolic-ref", "HEAD"]), branch);
    input.write_all(b"commit\n").unwrap();
    drop(input);
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn staging_bypasses_required_filters_and_never_runs_commit_hooks() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let root = f.root();
    git(&root, &["config", "filter.owned.clean", "false"]);
    git(&root, &["config", "filter.owned.required", "true"]);
    std::fs::write(root.join(".gitattributes"), "*.txt filter=owned\n").unwrap();
    std::fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\nexit 73\n").unwrap();
    std::fs::set_permissions(
        root.join(".git/hooks/pre-commit"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(root.join("notes.txt"), "exact unfiltered bytes\n").unwrap();
    std::fs::set_permissions(
        root.join("notes.txt"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    f.apply("stage", json!({"repo":"owned/repo","paths":["notes.txt"]}));
    assert_eq!(
        git(&root, &["show", ":notes.txt"]),
        "exact unfiltered bytes\n"
    );
    assert!(git(&root, &["ls-files", "--stage", "notes.txt"]).starts_with("100755 "));
    f.apply("commit",json!({"repo":"owned/repo","message":"No filter or hook","author_name":"Owned fixture","author_email":"fixture@example.invalid"}));
    assert_eq!(
        git(&root, &["show", "HEAD:notes.txt"]),
        "exact unfiltered bytes\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".gitattributes")).unwrap(),
        "*.txt filter=owned\n"
    );
}

#[test]
fn unstage_new_file_preserves_untracked_source_and_recovery_index_bytes() {
    let f = Fixture::new();
    let root = f.root();
    let original = std::fs::read(root.join(".git/index")).unwrap();
    std::fs::write(root.join(" new,file.txt "), "literal filename and bytes").unwrap();
    f.apply(
        "stage",
        json!({"repo":"owned/repo","paths":[" new,file.txt "]}),
    );
    assert_eq!(
        git(&root, &["show", ": new,file.txt "]),
        "literal filename and bytes"
    );
    f.apply(
        "unstage",
        json!({"repo":"owned/repo","paths":[" new,file.txt "]}),
    );
    assert!(!git(&root, &["ls-files"]).contains("new,file"));
    assert_eq!(
        std::fs::read_to_string(root.join(" new,file.txt ")).unwrap(),
        "literal filename and bytes"
    );
    assert!(
        std::fs::read_dir(f.temp.path().join("state"))
            .unwrap()
            .any(|e| {
                let p = e.unwrap().path();
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("git-index-recovery-")
                    && std::fs::read(p).unwrap() == original
            })
    );
}

#[test]
fn staging_exact_deletions_links_and_modes_never_changes_working_entries() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    };
    let f = Fixture::new();
    let root = f.root();
    std::fs::rename(
        root.join("notes.txt"),
        f.temp.path().join("retained-deleted-notes"),
    )
    .unwrap();
    let link_bytes = b"../owned-missing-target-\xff";
    symlink(
        std::ffi::OsString::from_vec(link_bytes.to_vec()),
        root.join("link,with\nnewline"),
    )
    .unwrap();
    std::fs::write(root.join("run"), b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(root.join("run"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let args = json!({"repo":"owned/repo","paths":["notes.txt","link,with\nnewline","run"]});
    let plan = f
        .local
        .prepare("stage", &args, "scope-a", &f.control())
        .unwrap();
    let files = plan.preview()["files"].as_array().unwrap().clone();
    assert_eq!(files[0]["operation"], "stage deletion (index only)");
    assert_eq!(files[1]["mode"], "120000");
    assert_eq!(files[2]["mode"], "100755");
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    f.local.execute(plan, "scope-a", &f.control()).unwrap();
    assert!(git(&root, &["ls-files", "--", "notes.txt"]).is_empty());
    let blob = Command::new("/usr/bin/git")
        .args(["show", ":link,with\nnewline"])
        .current_dir(&root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(blob.status.success());
    assert_eq!(blob.stdout, link_bytes);
    assert!(git(&root, &["ls-files", "--stage", "--", "run"]).starts_with("100755 "));
    assert_eq!(
        std::fs::read_link(root.join("link,with\nnewline"))
            .unwrap()
            .into_os_string()
            .into_vec(),
        link_bytes
    );
    assert!(!root.join("notes.txt").exists());
    assert_eq!(
        std::fs::read(f.temp.path().join("retained-deleted-notes")).unwrap(),
        b"original\n"
    );
    f.apply("unstage", args);
    assert_eq!(git(&root, &["show", ":notes.txt"]), "original\n");
    assert!(git(&root, &["ls-files", "--", "run"]).is_empty());
    assert!(
        std::fs::symlink_metadata(root.join("link,with\nnewline"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!root.join("notes.txt").exists());
    assert!(
        f.local
            .prepare(
                "stage",
                &json!({"repo":"owned/repo","paths":["never-tracked"]}),
                "scope-a",
                &f.control()
            )
            .is_err()
    );
}

#[test]
fn staging_refuses_reappearing_deletions_retargeted_links_modes_and_directory_expansion() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for kind in ["deletion", "link", "mode", "parent"] {
        let f = Fixture::new();
        let root = f.root();
        let path = match kind {
            "deletion" => {
                std::fs::rename(
                    root.join("notes.txt"),
                    f.temp.path().join("retained-deletion"),
                )
                .unwrap();
                "notes.txt"
            }
            "link" => {
                symlink("notes.txt", root.join("link")).unwrap();
                "link"
            }
            "parent" => {
                std::fs::create_dir(root.join("parent")).unwrap();
                std::fs::write(root.join("parent/file"), b"initial").unwrap();
                "parent/file"
            }
            _ => "notes.txt",
        };
        let plan = f
            .local
            .prepare(
                "stage",
                &json!({"repo":"owned/repo","paths":[path]}),
                "scope-a",
                &f.control(),
            )
            .unwrap();
        let index = std::fs::read(root.join(".git/index")).unwrap();
        match kind {
            "deletion" => std::fs::write(root.join(path), b"later user recreation").unwrap(),
            "link" => {
                std::fs::rename(root.join(path), root.join("retained-link")).unwrap();
                symlink("later-target", root.join(path)).unwrap();
            }
            "parent" => {
                std::fs::rename(root.join("parent"), f.temp.path().join("retained-parent"))
                    .unwrap();
                symlink(f.temp.path().join("retained-parent"), root.join("parent")).unwrap();
            }
            "mode" => {
                std::fs::set_permissions(root.join(path), std::fs::Permissions::from_mode(0o755))
                    .unwrap()
            }
            _ => unreachable!(),
        }
        assert!(
            f.local.execute(plan, "scope-a", &f.control()).is_err(),
            "{kind}"
        );
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        if kind == "deletion" {
            assert_eq!(
                std::fs::read(root.join(path)).unwrap(),
                b"later user recreation"
            );
        }
        if kind == "link" {
            assert_eq!(
                std::fs::read_link(root.join(path)).unwrap(),
                Path::new("later-target")
            );
        }
    }
    let f = Fixture::new();
    std::fs::create_dir(f.root().join("folder")).unwrap();
    std::fs::write(f.root().join("folder/child"), b"not recursively selected").unwrap();
    assert!(
        f.local
            .prepare(
                "stage",
                &json!({"repo":"owned/repo","paths":["folder"]}),
                "scope-a",
                &f.control()
            )
            .is_err()
    );
}

#[test]
fn workspace_review_is_read_only_and_ignores_repository_filter_programs() {
    let f = Fixture::new();
    let root = f.root();
    git(&root, &["config", "filter.owned.clean", "false"]);
    git(&root, &["config", "filter.owned.required", "true"]);
    std::fs::write(root.join(".gitattributes"), "*.txt filter=owned\n").unwrap();
    std::fs::write(root.join(".git/info/attributes"), "*.txt filter=owned\n").unwrap();
    std::fs::write(root.join("notes.txt"), "raw reviewed bytes\n").unwrap();
    let before = std::fs::read(root.join(".git/index")).unwrap();
    let workspace = WorkspaceService::new().unwrap().register(&root).unwrap();
    let spools = tempfile::tempdir().unwrap();
    let read = ag1_platform::git::GitService::new(
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        Path::new("/usr/bin/git"),
        spools.path(),
    )
    .unwrap();
    let view = read.status_diff(&workspace, None).unwrap();
    assert!(String::from_utf8_lossy(&view.diff).contains("raw reviewed bytes"));
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), before);
    assert_eq!(
        std::fs::read_to_string(root.join(".git/info/attributes")).unwrap(),
        "*.txt filter=owned\n"
    );
    assert!(!root.join(".git/index.lock").exists());
}

#[test]
fn forged_worktree_pointer_cannot_borrow_another_worktrees_metadata() {
    let f = Fixture::new();
    let root = f.root();
    let linked = f.temp.path().join("legitimate-worktree");
    git(
        &root,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let fake = f.temp.path().join("forged");
    std::fs::create_dir(&fake).unwrap();
    std::fs::copy(linked.join(".git"), fake.join(".git")).unwrap();
    let workspace = WorkspaceService::new().unwrap().register(&fake).unwrap();
    let spools = tempfile::tempdir().unwrap();
    let read = ag1_platform::git::GitService::new(
        Path::new(env!("CARGO_BIN_EXE_ag1-process-host")),
        Path::new("/usr/bin/git"),
        spools.path(),
    )
    .unwrap();
    assert!(matches!(
        read.status_diff(&workspace, None),
        Err(DomainError::PermissionDenied)
    ));
}
