
// Investigation-only diagnostic; not part of the running candidate.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overnight_diagnostic_git_without_optional_info_directories() {
    let fixture = fixture("http://127.0.0.1:1", "disabled");
    owned_repo_git(&fixture.root, &["init", "-b", "main"]);
    std::fs::remove_dir_all(fixture.root.join(".git/info")).unwrap();
    std::fs::remove_dir_all(fixture.root.join(".git/objects/info")).unwrap();
    // System Git confirms this is a valid repository, not malformed metadata.
    owned_repo_git(&fixture.root, &["status", "--porcelain"]);
    let runner = native_repo_runner(&fixture, "/usr/bin/git".as_ref()).await;
    let result = execute_repo(&fixture, &runner, "optional-git-info", "status");
    assert!(result["isError"] != true, "{result}");
    assert_eq!(result["details"]["branch"], "main");
}
