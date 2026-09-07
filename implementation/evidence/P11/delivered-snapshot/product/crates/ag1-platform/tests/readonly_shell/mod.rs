use super::*;
use ag1_core::{SessionId, context::features::Features, tools::shell::quote};
use ag1_platform::{
    process::ProcessSupervisor,
    shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
};
use ag1_storage::{Store, context::ContextStore};

#[test]
fn readonly_shell_reads_and_blocks_real_file_metadata_fifo_socket_and_git_writes() {
    let (dir, ws, _, mut registry, scope) = setup();
    registry
        .activate(&["bash".into()], &["bash".into()].into())
        .unwrap();
    fs::write(dir.path().join("original"), "retained\n").unwrap();
    fs::set_permissions(
        dir.path().join("original"),
        fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    assert!(
        std::process::Command::new("/usr/bin/git")
            .args(["init", "-q"])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success()
    );
    let listener = std::os::unix::net::UnixListener::bind(dir.path().join("socket")).unwrap();
    listener.set_nonblocking(true).unwrap();
    let fifo = std::ffi::CString::new(dir.path().join("fifo").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    // Holding both ends avoids a blocked writer obscuring an authorization bug.
    use std::os::unix::fs::OpenOptionsExt;
    let mut fifo_reader = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(dir.path().join("fifo"))
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("original");
    fs::write(&outside_file, "outside").unwrap();
    symlink(&outside_file, dir.path().join("link")).unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-06T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[8; 32])
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let config = ShellConfig {
        executable: "/bin/bash".into(),
        env: [("PATH".into(), "/usr/bin:/bin".into())].into(),
        command_prefix: None,
        command_template: None,
        sandbox: Sandbox::Off,
        bubblewrap: Some("/usr/bin/bwrap".into()),
        output_bytes: 64 * 1024,
        address_space: Some(1024 * 1024 * 1024),
    };
    let original = ShellService::new(
        ws.clone(),
        supervisor.clone(),
        spools.path(),
        config.clone(),
    )
    .unwrap();
    let shell = original.readonly().unwrap();
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "readonly",
        timestamp,
        Features::default(),
    )
    .unwrap();
    let run =
        |shell: &ShellService, id: &str, command: String, artifacts: &mut ContextStore<'_>| {
            let call = call(&registry, id, "bash", json!({"command":command}));
            let plan = shell.prepare(&call, &registry, &scope).unwrap();
            let ticket = search_ticket(
                plan.call().clone(),
                plan.target_revision(),
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            shell
                .execute(
                    plan,
                    ticket,
                    ShellExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &AtomicBool::new(false),
                        deadline: Instant::now() + Duration::from_secs(5),
                    },
                    artifacts,
                    |_| {},
                )
                .unwrap()
                .result
        };
    let reads = run(&shell,"reads","cat original; readlink /proc/self/ns/net; readlink /proc/self/ns/pid; git status --porcelain; printf done".into(),&mut artifacts);
    assert_ne!(reads["isError"], true, "{reads}");
    let text = reads["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("retained") && text.contains("done"),
        "{reads}"
    );
    assert!(
        !text.contains(
            fs::read_link("/proc/self/ns/net")
                .unwrap()
                .to_str()
                .unwrap()
        )
    );
    assert!(
        !text.contains(
            fs::read_link("/proc/self/ns/pid")
                .unwrap()
                .to_str()
                .unwrap()
        )
    );
    let commands = [
        "env touch marker".to_owned(),
        "printf changed > original".into(),
        "find . -name original -delete".into(),
        "chmod 777 original".into(),
        "sort original -o original".into(),
        "git config --local ag1.changed yes".into(),
        "printf changed > link".into(),
        "printf changed > fifo".into(),
        "python3 -c 'import socket; socket.socket(socket.AF_UNIX).connect(\"socket\")'".into(),
        "python3 -c 'import socket; socket.socket(socket.AF_INET)'".into(),
        "python3 -c 'import os; os.open(\"original\", os.O_RDONLY|os.O_TRUNC)'".into(),
        format!("printf changed > {}", quote(outside_file.to_str().unwrap())),
    ];
    for (i, command) in commands.into_iter().enumerate() {
        let result = run(&shell, &format!("deny-{i}"), command, &mut artifacts);
        assert_eq!(result["isError"], true, "case {i}: {result}");
        assert_eq!(
            fs::read_to_string(dir.path().join("original")).unwrap(),
            "retained\n"
        );
        assert_eq!(
            fs::metadata(dir.path().join("original"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }
    assert!(!dir.path().join("marker").exists());
    assert_eq!(fs::read_to_string(&outside_file).unwrap(), "outside");
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let mut byte = [0];
    assert_eq!(
        std::io::Read::read(&mut fifo_reader, &mut byte)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::WouldBlock
    );
    // Derived capability leaves ordinary host configuration unchanged.
    let result = run(
        &original,
        "normal",
        "printf allowed > normal".into(),
        &mut artifacts,
    );
    assert_ne!(result["isError"], true);
    assert_eq!(
        fs::read_to_string(dir.path().join("normal")).unwrap(),
        "allowed"
    );
    let mut missing = config;
    missing.bubblewrap = None;
    assert!(matches!(
        ShellService::new(ws, supervisor, spools.path(), missing)
            .unwrap()
            .readonly(),
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}
