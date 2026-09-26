use ag1_core::{DomainError, evidence::Freshness};
use ag1_platform::{source::SourceService, workspace::WorkspaceService};
use sha2::{Digest, Sha256};
use std::{fs, os::unix::fs::symlink};
fn fixture() -> (tempfile::TempDir, SourceService) {
    let dir = tempfile::tempdir().unwrap();
    let registry = WorkspaceService::new().unwrap();
    let service = SourceService::new(registry.register(dir.path()).unwrap());
    (dir, service)
}
#[test]
fn raw_bom_crlf_unicode_hash_precedes_normalized_clipped_display() {
    let (dir, mut service) = fixture();
    let bytes = "\u{feff}hello\r\n🌙 β\r\n".as_bytes();
    fs::write(dir.path().join("a.rs"), bytes).unwrap();
    let snapshot = service.capture("a.rs").unwrap();
    assert_eq!(snapshot.snapshot().bytes, bytes);
    assert_eq!(
        snapshot.reference().digest(),
        format!("{:x}", Sha256::digest(bytes))
    );
    let display = snapshot.display(8).unwrap();
    assert_eq!(display.text, "hello\n");
    assert!(display.normalized && display.truncated);
    assert_eq!(display.full_bytes, bytes.len());
    assert_eq!(
        service.validate(&snapshot.reference().reference()).status,
        Freshness::Current
    );
    fs::write(dir.path().join("a.rs"), "changed").unwrap();
    assert_eq!(
        service.validate(&snapshot.reference().reference()).status,
        Freshness::Stale
    );
    assert_eq!(snapshot.snapshot().bytes, bytes);
}
#[test]
fn forced_same_size_edit_during_read_cannot_pair_old_text_with_new_hash() {
    let (dir, service) = fixture();
    let path = dir.path().join("a");
    fs::write(&path, b"before").unwrap();
    let result = service.capture_with_barrier("a", || fs::write(&path, b"after!").unwrap());
    assert!(matches!(result, Err(DomainError::Conflict)));
    let next = service.capture("a").unwrap();
    assert_eq!(next.snapshot().bytes, b"after!");
    assert_eq!(
        next.reference().digest(),
        format!("{:x}", Sha256::digest(b"after!"))
    );
}
#[test]
fn forced_rename_replacement_and_symlink_swaps_fail_snapshot_validation() {
    for mode in 0..3 {
        let (dir, service) = fixture();
        let original = dir.path().join("a");
        let other = dir.path().join("b");
        fs::write(&original, b"same").unwrap();
        fs::write(&other, b"same").unwrap();
        let input = if mode == 2 {
            symlink("a", dir.path().join("link")).unwrap();
            "link"
        } else {
            "a"
        };
        let result = service.capture_with_barrier(input, || match mode {
            0 => fs::rename(&other, &original).unwrap(),
            1 => {
                fs::remove_file(&original).unwrap();
                symlink("b", &original).unwrap();
            }
            _ => {
                fs::remove_file(dir.path().join("link")).unwrap();
                symlink("b", dir.path().join("link")).unwrap();
            }
        });
        assert!(result.is_err(), "mode {mode}");
    }
}
#[test]
fn contained_symlinks_are_canonical_external_paths_and_special_files_rejected() {
    let (dir, mut service) = fixture();
    fs::write(dir.path().join("a"), b"data").unwrap();
    symlink("a", dir.path().join("link")).unwrap();
    let snapshot = service.capture("link").unwrap();
    assert_eq!(snapshot.reference().path(), "a");
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), b"outside").unwrap();
    symlink(outside.path().join("secret"), dir.path().join("escape")).unwrap();
    assert!(matches!(
        service.capture("escape"),
        Err(DomainError::PermissionDenied)
    ));
    assert!(service.capture(".").is_err());
    assert_eq!(
        service.validate("artifact:abc").status,
        Freshness::Unverified
    );
    assert_eq!(
        service.validate("file:../escape@sha256:bad").status,
        Freshness::Invalid
    );
    fs::remove_file(dir.path().join("a")).unwrap();
    assert_eq!(
        service.validate(&snapshot.reference().reference()).status,
        Freshness::Missing
    );
}
#[test]
fn replacement_root_and_oversized_or_binary_display_are_explicit() {
    let (dir, service) = fixture();
    fs::write(dir.path().join("binary"), [0xff, 0xfe]).unwrap();
    assert!(service.capture("binary").unwrap().display(10).is_err());
    let file = fs::File::create(dir.path().join("large")).unwrap();
    file.set_len(8 * 1024 * 1024 + 1).unwrap();
    assert!(service.capture("large").is_err());
    let old = dir.path().with_extension("moved-fixture");
    fs::rename(dir.path(), &old).unwrap();
    fs::create_dir(dir.path()).unwrap();
    fs::write(dir.path().join("a"), b"replacement").unwrap();
    assert!(matches!(service.capture("a"), Err(DomainError::Conflict)));
    fs::remove_dir_all(old).unwrap();
}
#[test]
fn repeated_validation_detects_same_length_changes_without_watcher_hints() {
    let (dir, mut service) = fixture();
    let path = dir.path().join("a");
    fs::write(&path, b"first").unwrap();
    let snapshot = service.capture("a").unwrap();
    for _ in 0..5 {
        assert_eq!(
            service.validate(&snapshot.reference().reference()).status,
            Freshness::Current
        );
    }
    fs::write(&path, b"other").unwrap();
    assert_eq!(
        service.validate(&snapshot.reference().reference()).status,
        Freshness::Stale
    );
}

#[test]
fn immutable_snapshot_rejects_forged_digest_and_preserves_full_byte_count() {
    let (dir, service) = fixture();
    fs::write(dir.path().join("a"), b"honest").unwrap();
    let snapshot = service.capture("a").unwrap();
    let mut bytes = snapshot.snapshot().clone();
    bytes.bytes = b"forged".to_vec();
    assert!(ag1_core::evidence::SourceSnapshot::new("a", bytes).is_err());
    let mut bytes = snapshot.snapshot().clone();
    bytes.revision.size = 0;
    assert!(ag1_core::evidence::SourceSnapshot::new("a", bytes).is_err());
}
