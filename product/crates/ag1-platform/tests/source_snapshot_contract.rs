use ag1_platform::cutover::{Registry, SourceSnapshot};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn git(workspace: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=P13 Fixture",
            "-c",
            "user.email=p13@example.invalid",
        ])
        .args(args)
        .current_dir(workspace)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

fn capture(workspace: &Path, state: &Path, registry: &Registry, archive: &Path) -> SourceSnapshot {
    let mut owner = registry.open_typescript(workspace, state).unwrap();
    assert!(owner.snapshot_source(archive).is_err());
    owner.begin_drain().unwrap();
    assert!(owner.snapshot_source(archive).is_err());
    owner.quiesced().unwrap();
    drop(owner);
    let lease = registry.resume_transfer(workspace).unwrap();
    lease.snapshot_source(archive).unwrap()
}

#[test]
fn source_snapshot_keeps_real_git_dirty_files_modes_and_empty_directories_separate_from_state() {
    for mode in ["unborn", "loose", "packed", "detached"] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let state = temp.path().join("state");
        let archive = temp.path().join("archive");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&state).unwrap();
        fs::write(state.join("session-private"), "agent state stays separate").unwrap();
        git(&workspace, &["init", "--quiet"]);
        fs::write(workspace.join("run.sh"), "original\n").unwrap();
        fs::set_permissions(workspace.join("run.sh"), fs::Permissions::from_mode(0o751)).unwrap();
        let oid = if mode == "unborn" {
            None
        } else {
            git(&workspace, &["add", "run.sh"]);
            git(&workspace, &["commit", "--quiet", "-m", "fixture"]);
            let oid = git(&workspace, &["rev-parse", "HEAD"]).trim().to_owned();
            if mode == "packed" {
                git(&workspace, &["pack-refs", "--all"]);
            }
            if mode == "detached" {
                git(&workspace, &["checkout", "--quiet", "--detach"]);
            }
            Some(oid)
        };
        fs::write(workspace.join("run.sh"), "dirty source\n").unwrap();
        fs::write(workspace.join("untracked.txt"), "retained too").unwrap();
        fs::create_dir(workspace.join("empty")).unwrap();
        let status = git(&workspace, &["status", "--porcelain=v1", "-z"]);
        let registry = Registry::at(&temp.path().join("registry")).unwrap();
        let receipt = capture(&workspace, &state, &registry, &archive);
        receipt.verify_archive(&workspace).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(archive.join("source-snapshot.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["git"]["object"],
            serde_json::to_value(oid).unwrap()
        );
        assert_eq!(manifest["modes"]["run.sh"], 0o751);
        assert_eq!(
            fs::read(archive.join("files/run.sh")).unwrap(),
            b"dirty source\n"
        );
        assert!(archive.join("files/empty").is_dir());
        assert!(!archive.join("files/session-private").exists());
        assert_eq!(
            fs::metadata(archive.join("files/run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(git(&workspace, &["status", "--porcelain=v1", "-z"]), status);
        fs::write(workspace.join("run.sh"), "post-cutover work\n").unwrap();
        receipt.verify_archive(&workspace).unwrap(); // No source rollback or stale-source requirement.
        assert_eq!(
            fs::read(workspace.join("run.sh")).unwrap(),
            b"post-cutover work\n"
        );
        assert!(
            registry
                .resume_transfer(&workspace)
                .unwrap()
                .snapshot_source(&archive)
                .is_err()
        );
        fs::write(archive.join("files/run.sh"), "tampered").unwrap();
        assert!(receipt.verify_archive(&workspace).is_err());
    }
}

#[test]
fn source_snapshot_refuses_external_links_alternates_and_nested_destinations_without_modification()
{
    for mode in [
        "symlink",
        "linked-git",
        "alternates",
        "nested",
        "parent-git",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let state = temp.path().join("state");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&state).unwrap();
        if !["linked-git", "parent-git"].contains(&mode) {
            git(&workspace, &["init", "--quiet"]);
        }
        let secret = temp.path().join("external");
        fs::write(&secret, "never follow").unwrap();
        match mode {
            "symlink" => std::os::unix::fs::symlink(&secret, workspace.join("link")).unwrap(),
            "linked-git" => fs::write(
                workspace.join(".git"),
                format!("gitdir: {}\n", secret.display()),
            )
            .unwrap(),
            "alternates" => {
                fs::write(
                    workspace.join(".git/objects/info/alternates"),
                    secret.to_str().unwrap(),
                )
                .unwrap();
            }
            "parent-git" => {
                git(temp.path(), &["init", "--quiet"]);
            }
            _ => (),
        }
        let registry = Registry::at(&temp.path().join("registry")).unwrap();
        let mut owner = registry.open_typescript(&workspace, &state).unwrap();
        owner.begin_drain().unwrap();
        owner.quiesced().unwrap();
        drop(owner);
        let destination = if mode == "nested" {
            workspace.join("archive")
        } else {
            temp.path().join("archive")
        };
        assert!(
            registry
                .resume_transfer(&workspace)
                .unwrap()
                .snapshot_source(&destination)
                .is_err()
        );
        assert!(!destination.exists());
        assert_eq!(fs::read(&secret).unwrap(), b"never follow");
    }
}

#[test]
fn source_snapshot_binds_workspace_identity_and_detects_missing_or_injected_archive_files() {
    for tamper in ["missing", "injected", "replaced-workspace", "manifest"] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let state = temp.path().join("state");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&state).unwrap();
        git(&workspace, &["init", "--quiet"]);
        fs::write(workspace.join("source"), "original").unwrap();
        let registry = Registry::at(&temp.path().join("registry")).unwrap();
        let archive = temp.path().join("archive");
        let receipt = capture(&workspace, &state, &registry, &archive);
        match tamper {
            "missing" => fs::remove_file(archive.join("files/source")).unwrap(),
            "injected" => fs::write(archive.join("files/injected"), "extra").unwrap(),
            "replaced-workspace" => {
                fs::rename(&workspace, temp.path().join("preserved-original")).unwrap();
                fs::create_dir(&workspace).unwrap();
            }
            _ => fs::write(archive.join("source-snapshot.json"), "{}").unwrap(),
        }
        assert!(receipt.verify_archive(&workspace).is_err());
    }
}

#[test]
fn streaming_source_preserves_large_files_and_literal_links_without_following_targets() {
    use base64::{Engine, engine::general_purpose::STANDARD as B64};
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let state = temp.path().join("state");
    let archive = temp.path().join("archive");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&state).unwrap();
    git(&workspace, &["init", "--quiet"]);
    fs::create_dir(workspace.join("empty")).unwrap();
    let bytes = vec![73; 2 * 1024 * 1024 + 3];
    fs::write(workspace.join("large"), &bytes).unwrap();
    fs::set_permissions(workspace.join("large"), fs::Permissions::from_mode(0o751)).unwrap();
    let external = temp.path().join("external");
    fs::write(&external, "not archived").unwrap();
    std::os::unix::fs::symlink(&external, workspace.join("link")).unwrap();
    std::os::unix::fs::symlink("absent", workspace.join("dangling")).unwrap();
    let registry = Registry::at(&temp.path().join("registry")).unwrap();
    let mut owner = registry.open_typescript(&workspace, &state).unwrap();
    assert!(owner.snapshot_source_streaming(&archive).is_err());
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let lease = registry.resume_transfer(&workspace).unwrap();
    let receipt = lease.snapshot_source_streaming(&archive).unwrap();
    receipt.verify_unchanged(&workspace).unwrap();
    assert!(
        fs::read(archive.join("tree.data"))
            .unwrap()
            .ends_with(&bytes)
    );
    let index: Vec<serde_json::Value> = fs::read_to_string(archive.join("tree.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert!(index.len() > 4);
    assert_eq!(
        index
            .iter()
            .filter(|row| row["kind"] == "literal-symlink")
            .count(),
        2
    );
    let large = index
        .iter()
        .find(|row| row["path_base64"] == B64.encode("large"))
        .unwrap();
    assert_eq!(large["mode"], 0o751);
    fs::write(&external, "external changes are not source entries").unwrap();
    receipt.verify_unchanged(&workspace).unwrap();
    fs::write(workspace.join("large"), "new work").unwrap();
    assert!(receipt.verify_unchanged(&workspace).is_err());
    receipt.verify_archive(&workspace).unwrap();
    assert!(lease.snapshot_source_streaming(&archive).is_err());
    assert_eq!(
        fs::read_to_string(workspace.join("large")).unwrap(),
        "new work"
    );
}

#[test]
fn streaming_source_rejects_archive_tampering_and_external_git_closure() {
    for tamper in [
        "data",
        "index",
        "injected",
        "symlink",
        "link-drift",
        "git-alternates",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let state = temp.path().join("state");
        let archive = temp.path().join("archive");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&state).unwrap();
        git(&workspace, &["init", "--quiet"]);
        fs::write(workspace.join("file"), "original").unwrap();
        std::os::unix::fs::symlink("file", workspace.join("link")).unwrap();
        let registry = Registry::at(&temp.path().join("registry")).unwrap();
        let mut owner = registry.open_typescript(&workspace, &state).unwrap();
        owner.begin_drain().unwrap();
        owner.quiesced().unwrap();
        drop(owner);
        let lease = registry.resume_transfer(&workspace).unwrap();
        if tamper == "git-alternates" {
            fs::write(workspace.join(".git/objects/info/alternates"), "/external").unwrap();
            assert!(lease.snapshot_source_streaming(&archive).is_err());
            assert!(!archive.exists());
            continue;
        }
        let receipt = lease.snapshot_source_streaming(&archive).unwrap();
        match tamper {
            "data" => fs::write(archive.join("tree.data"), "bad").unwrap(),
            "index" => fs::write(archive.join("tree.jsonl"), "bad").unwrap(),
            "injected" => fs::write(archive.join("extra"), "extra").unwrap(),
            "link-drift" => {
                fs::remove_file(workspace.join("link")).unwrap();
                std::os::unix::fs::symlink("different", workspace.join("link")).unwrap();
            }
            _ => {
                fs::remove_file(archive.join("tree.data")).unwrap();
                std::os::unix::fs::symlink(workspace.join("file"), archive.join("tree.data"))
                    .unwrap();
            }
        }
        if tamper == "link-drift" {
            assert!(receipt.verify_unchanged(&workspace).is_err());
            receipt.verify_archive(&workspace).unwrap();
        } else {
            assert!(receipt.verify_archive(&workspace).is_err());
        }
        assert_eq!(
            fs::read_to_string(workspace.join("file")).unwrap(),
            "original"
        );
    }
}
