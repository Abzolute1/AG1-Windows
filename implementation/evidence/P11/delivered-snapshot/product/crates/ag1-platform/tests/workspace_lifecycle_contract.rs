#![cfg(target_os = "linux")]
use ag1_core::DomainError;
use ag1_platform::{
    resource_fs::ScanControl,
    workspace::WorkspaceService,
    workspace_lifecycle::{Lifecycle, State},
};
use std::{
    fs,
    os::unix::fs::symlink,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
fn selected_workspace_summary_active_guard_and_preserved_discard_use_owned_directory_identity() {
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("owned");
    fs::create_dir(&storage).unwrap();
    let outside = dir.path().join("source");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "source unchanged").unwrap();
    let service = WorkspaceService::new().unwrap();
    let parent = service.register(&storage).unwrap();
    let cancel = AtomicBool::new(false);
    let control = ScanControl {
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let mut lifecycle = Lifecycle::default();
    let view = lifecycle.view();
    assert!(
        view.output
            .contains("No autonomous workspace is active or preserved")
    );
    assert!(view.dim);
    assert_eq!(view.state, State::Empty);
    assert!(view.root.is_none());
    for name in ["", ".", "..", "../source", "/", "nested/child"] {
        assert!(lifecycle.admit(parent.clone(), name, false).is_err());
    }
    fs::create_dir(storage.join("sandbox")).unwrap();
    fs::write(storage.join("sandbox/evidence"), "preserved evidence").unwrap();
    symlink(&outside, storage.join("sandbox/source-link")).unwrap();
    lifecycle.admit(parent.clone(), "sandbox", true).unwrap();
    assert!(
        lifecycle
            .discard(&control)
            .unwrap()
            .warning
            .unwrap()
            .contains("Cannot discard the autonomous workspace while it is active")
    );
    assert!(storage.join("sandbox/evidence").exists());
    lifecycle.preserve_quiescent().unwrap();
    let stopped = AtomicBool::new(true);
    let cancelled = ScanControl {
        cancel: &stopped,
        deadline: control.deadline,
    };
    assert!(matches!(
        lifecycle.discard(&cancelled),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(lifecycle.view().state, State::Preserved);
    let result = lifecycle.discard(&control).unwrap();
    assert!(
        result
            .output
            .contains("Discarded preserved autonomous workspace")
    );
    assert!(result.dim);
    assert!(result.root.is_none());
    assert_eq!(result.state, State::Empty);
    assert!(!storage.join("sandbox").exists());
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "source unchanged"
    );
    assert!(lifecycle.discard(&control).unwrap().warning.is_some());

    // The same display path does not authorize deleting a replacement directory.
    fs::create_dir(storage.join("sandbox")).unwrap();
    lifecycle.admit(parent.clone(), "sandbox", false).unwrap();
    fs::rename(storage.join("sandbox"), storage.join("old")).unwrap();
    fs::create_dir(storage.join("sandbox")).unwrap();
    fs::write(storage.join("sandbox/new"), "unadmitted").unwrap();
    assert!(matches!(
        lifecycle.discard(&control),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        fs::read_to_string(storage.join("sandbox/new")).unwrap(),
        "unadmitted"
    );
    assert!(storage.join("old").exists());
    assert_eq!(lifecycle.view().state, State::Preserved);
}
