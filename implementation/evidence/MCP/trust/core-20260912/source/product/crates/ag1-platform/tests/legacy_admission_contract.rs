use ag1_platform::cutover::{Owner, Registry, Stage};
use ag1_storage::{FileKind, FrozenFile, FrozenManifest, digest};
use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[test]
fn inactive_legacy_admission_requires_complete_unchanged_state_no_open_owner_and_no_existing_authority()
 {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let source = temp.path().join("legacy");
    fs::create_dir(&workspace).unwrap();
    fs::create_dir(&source).unwrap();
    let path = source.join("journal.jsonl");
    let bytes = b"retained legacy bytes";
    fs::write(&path, bytes).unwrap();
    let manifest = FrozenManifest {
        version: 1,
        frozen: true,
        files: vec![FrozenFile {
            path: "journal.jsonl".into(),
            sha256: digest(bytes),
            bytes: bytes.len() as u64,
            kind: FileKind::Extra,
            session: None,
            origin: None,
        }],
    };
    let registry = Registry::at(&temp.path().join("registry")).unwrap();
    // An owned fixture shell holds the selected input open until stdin closes.
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exec 3< \"$1\"; echo ready; read line", "fixture"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    assert!(
        registry
            .register_stopped_legacy(&workspace, &source, &manifest)
            .is_err()
    );
    assert!(registry.inspect(&workspace).unwrap().is_none());
    drop(child.stdin.take());
    child.wait().unwrap();
    fs::write(source.join("unlisted"), "cannot omit").unwrap();
    assert!(
        registry
            .register_stopped_legacy(&workspace, &source, &manifest)
            .is_err()
    );
    fs::remove_file(source.join("unlisted")).unwrap();
    fs::write(&path, "changed").unwrap();
    assert!(
        registry
            .register_stopped_legacy(&workspace, &source, &manifest)
            .is_err()
    );
    fs::write(&path, bytes).unwrap();
    let record = registry
        .register_stopped_legacy(&workspace, &source, &manifest)
        .unwrap();
    assert_eq!(record.owner, Owner::TypeScript);
    assert_eq!(record.stage, Stage::Active);
    assert!(record.quiescent);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(
        registry
            .register_stopped_legacy(&workspace, &source, &manifest)
            .is_err()
    );
    let lease = registry.begin_transfer(&workspace).unwrap();
    assert_eq!(lease.record().unwrap().stage, Stage::Draining);
}
