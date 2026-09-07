#[path = "../../../../../fixtures/runtime/custody.rs"]
mod custody_fixture;
use ag1_platform::{
    client::{Config, Host},
    cutover::{FrozenBundle, Owner, Prepare, Registry, Stage},
};
use ag1_storage::{FileKind, FrozenFile, FrozenManifest, Store, digest};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    _dir: tempfile::TempDir,
    registry: Registry,
    workspace: PathBuf,
    source: PathBuf,
    archive: PathBuf,
    destination: PathBuf,
    bundle: FrozenBundle,
    config: Vec<u8>,
}
fn config() -> Config {
    thread_local! { static UNUSED: std::net::TcpListener = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); }
    let origin = UNUSED.with(|socket| format!("http://{}", socket.local_addr().unwrap()));
    serde_json::from_value(json!({"models":[{"provider":"fixture","id":"initial","api":"openai-completions","baseUrl":origin,"contextWindow":32768,"maxTokens":4096,"input":["text"],"reasoning":false}],"selected":"initial","credential_env":null,"system_prompt":"Fixture transfer","fixture":true})).unwrap()
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::at(&dir.path().join("authority")).unwrap();
        let workspace = dir.path().join("workspace");
        let source = dir.path().join("source");
        let archive = dir.path().join("archive");
        let destination = dir.path().join("destination");
        fs::create_dir(&workspace).unwrap();
        fs::create_dir(&source).unwrap();
        fs::write(workspace.join("retained.txt"), b"original worktree bytes\n").unwrap();
        let journal = [json!({"type":"session","version":3,"id":"migrated","cwd":workspace,"timestamp":"2026-09-07T00:00:00Z"}),
            json!({"type":"message","id":"user-one","parentId":null,"timestamp":"2026-09-07T00:00:01Z","message":{"role":"user","content":"Retain this exact imported message","timestamp":1}})]
            .iter().map(|v| v.to_string()+"\n").collect::<String>();
        let mut files = Vec::new();
        for (path, bytes, kind, origin) in [
            (
                "session.jsonl",
                journal.into_bytes(),
                FileKind::Journal,
                None,
            ),
            (
                "cursor-key",
                vec![7; 32],
                FileKind::CursorKey,
                Some("migrated".into()),
            ),
            (
                "opaque.json",
                br#"{"unknown":false,"zero":0,"ordered":[3,1,2]}"#.to_vec(),
                FileKind::Extra,
                None,
            ),
        ] {
            fs::write(source.join(path), &bytes).unwrap();
            files.push(FrozenFile {
                path: path.into(),
                sha256: digest(&bytes),
                bytes: bytes.len() as u64,
                kind,
                session: Some("migrated".into()),
                origin,
            });
        }
        let config = serde_json::to_vec(&config()).unwrap();
        let bundle = FrozenBundle {
            version: 1,
            files: FrozenManifest {
                version: 1,
                frozen: true,
                files,
            },
            selected_session: "migrated".into(),
            config_sha256: digest(&config),
            runtime_files: Vec::new(),
            runtime_configuration: None,
            source_snapshot: None,
        };
        Self {
            _dir: dir,
            registry,
            workspace,
            source,
            archive,
            destination,
            bundle,
            config,
        }
    }
    fn prepare(&self) -> Prepare<'_> {
        Prepare {
            id: "cutover-1",
            source: &self.source,
            archive: &self.archive,
            destination: &self.destination,
            bundle: &self.bundle,
            config: &self.config,
        }
    }
    fn drain(&self) {
        let mut owner = self
            .registry
            .open_typescript(&self.workspace, &self.source)
            .unwrap();
        owner.admit().unwrap();
        owner.begin_drain().unwrap();
        assert!(owner.admit().is_err());
        assert!(self.registry.resume_transfer(&self.workspace).is_err());
        owner.quiesced().unwrap();
        assert!(self.registry.resume_transfer(&self.workspace).is_err());
    }
    fn migrate(&self) {
        self.drain();
        let mut lease = self.registry.resume_transfer(&self.workspace).unwrap();
        lease.freeze(self.prepare()).unwrap();
        lease.import().unwrap();
        lease.validate().unwrap();
        lease.revoke().unwrap();
        lease.commit_fixture().unwrap();
    }
}

#[test]
fn workspace_memory_import_is_cas_bound_to_the_new_store_and_retains_original_bytes() {
    let f = Fixture::new();
    fs::create_dir(f.workspace.join(".ag1")).unwrap();
    let raw = br#"{"version":1,"entries":[{"id":"retained","text":"Original memory","kind":"fact","createdAt":1}]}"#;
    fs::write(f.workspace.join(".ag1/memory.json"), raw).unwrap();
    f.drain();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    assert!(lease.import_workspace_memory().is_err());
    lease.freeze(f.prepare()).unwrap();
    lease.import().unwrap();
    let report = lease.import_workspace_memory().unwrap();
    assert_eq!(lease.import_workspace_memory().unwrap(), report);
    let store = Store::open(&f.destination.join("store")).unwrap();
    let saved = store
        .memory(report["memory_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(saved.digest, digest(raw));
    assert_eq!(
        saved.normalized.unwrap()["entries"][0]["text"],
        "Original memory"
    );
    assert_eq!(fs::read(f.workspace.join(".ag1/memory.json")).unwrap(), raw);
    fs::write(
        f.workspace.join(".ag1/memory.json"),
        br#"{"version":1,"entries":[]}"#,
    )
    .unwrap();
    assert!(lease.import_workspace_memory().is_err());
    assert_eq!(
        store
            .memory(report["memory_id"].as_str().unwrap())
            .unwrap()
            .unwrap()
            .digest,
        digest(raw)
    );
}

#[test]
fn inactive_transfer_requires_verified_shutdown_and_preserves_the_old_owner_on_failure() {
    let f = Fixture::new();
    let mut owner = f.registry.open_typescript(&f.workspace, &f.source).unwrap();
    assert!(f.registry.begin_transfer(&f.workspace).is_err());
    owner.quiesced().unwrap();
    assert!(f.registry.begin_transfer(&f.workspace).is_err());
    drop(owner);
    let mut transfer = f.registry.begin_transfer(&f.workspace).unwrap();
    assert_eq!(transfer.record().unwrap().stage, Stage::Draining);
    transfer.cancel_transfer().unwrap();
    drop(transfer);
    let owner = f.registry.open_typescript(&f.workspace, &f.source).unwrap();
    drop(owner); // Simulated unclean exit; acquiring the lock does not certify it.
    assert!(f.registry.begin_transfer(&f.workspace).is_err());
}
#[test]
fn unaccepted_cutover_can_return_to_unchanged_ts_at_every_boundary_without_losing_archives() {
    for boundary in 0..4 {
        let f = Fixture::new();
        f.drain();
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        lease.freeze(f.prepare()).unwrap();
        if boundary >= 1 {
            lease.import().unwrap();
        }
        if boundary >= 2 {
            lease.validate().unwrap();
        }
        if boundary >= 3 {
            lease.revoke().unwrap();
        }
        let before = fs::read(f.archive.join("bundle.json")).unwrap();
        drop(lease);
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        lease.cancel_unaccepted().unwrap();
        assert_eq!(lease.record().unwrap().owner, Owner::TypeScript);
        assert_eq!(lease.record().unwrap().state.as_ref(), Some(&f.source));
        assert!(lease.cancel_unaccepted().is_err());
        drop(lease);
        assert_eq!(fs::read(f.archive.join("bundle.json")).unwrap(), before);
        f.bundle.files.preflight(&f.source).unwrap();
        assert!(
            f.registry
                .open_native(&f.workspace, &f.destination)
                .is_err()
        );
        assert!(f.registry.open_typescript(&f.workspace, &f.source).is_ok());
    }
    let f = Fixture::new();
    f.drain();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    lease.freeze(f.prepare()).unwrap();
    fs::write(f.source.join("opaque.json"), b"changed original").unwrap();
    assert!(lease.cancel_unaccepted().is_err());
    assert_eq!(lease.record().unwrap().stage, Stage::Frozen);
}
#[test]
fn drain_requires_both_quiescence_and_released_execution_lease() {
    let f = Fixture::new();
    let mut owner = f.registry.open_typescript(&f.workspace, &f.source).unwrap();
    owner.begin_drain().unwrap();
    drop(owner);
    assert!(f.registry.resume_transfer(&f.workspace).is_err());
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
    assert!(f.registry.open_native(&f.workspace, &f.source).is_err());
}

#[test]
fn every_persisted_boundary_reopens_unowned_or_with_exactly_one_owner() {
    let f = Fixture::new();
    f.drain();
    for step in 0..5 {
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        match step {
            0 => lease.freeze(f.prepare()).unwrap(),
            1 => lease.import().unwrap(),
            2 => lease.validate().unwrap(),
            3 => lease.revoke().unwrap(),
            4 => lease.commit_fixture().unwrap(),
            _ => unreachable!(),
        }
        assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
        if f.destination.exists() {
            assert!(
                f.registry
                    .open_native(&f.workspace, &f.destination)
                    .is_err()
            );
        }
        drop(lease);
        let registry = Registry::at(&f._dir.path().join("authority")).unwrap();
        assert_eq!(
            registry.inspect(&f.workspace).unwrap(),
            f.registry.inspect(&f.workspace).unwrap()
        );
    }
    let native = f
        .registry
        .open_native(&f.workspace, &f.destination)
        .unwrap();
    native.admit().unwrap();
    assert_eq!(native.record().unwrap().owner, Owner::Native);
    assert_eq!(native.record().unwrap().epoch, 2);
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
}

#[test]
fn duplicate_import_after_restart_is_idempotent_and_commit_cannot_repeat() {
    let f = Fixture::new();
    f.drain();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    lease.freeze(f.prepare()).unwrap();
    lease.import().unwrap();
    drop(lease);
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    lease.import().unwrap();
    lease.validate().unwrap();
    lease.validate().unwrap();
    lease.revoke().unwrap();
    lease.commit_fixture().unwrap();
    assert!(lease.commit_fixture().is_err());
    assert!(lease.freeze(f.prepare()).is_err());
    assert_eq!(
        Store::open(&f.destination.join("store"))
            .unwrap()
            .snapshot("migrated")
            .unwrap()
            .entries
            .len(),
        1
    );
}

#[test]
fn altered_manifest_configuration_and_source_never_publish_native_authority() {
    for file in ["bundle.json", "host.json", "source/opaque.json"] {
        let f = Fixture::new();
        f.drain();
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        lease.freeze(f.prepare()).unwrap();
        fs::write(f.archive.join(file), b"tampered").unwrap();
        assert!(lease.import().is_err());
        assert_eq!(lease.record().unwrap().stage, Stage::Frozen);
        assert!(f.registry.open_native(&f.workspace, &f.source).is_err());
    }
}

#[test]
fn authority_tamper_fences_admission_and_cannot_be_silently_overwritten() {
    let f = Fixture::new();
    let mut owner = f.registry.open_typescript(&f.workspace, &f.source).unwrap();
    let registry = f._dir.path().join("authority");
    let marker = fs::read_dir(registry)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|s| s == "json"))
        .unwrap();
    fs::write(&marker, b"{}").unwrap();
    assert!(owner.admit().is_err());
    assert!(owner.begin_drain().is_err());
    assert_eq!(fs::read(marker).unwrap(), b"{}");
}

#[test]
fn existing_destination_and_partial_archive_are_never_overwritten() {
    let f = Fixture::new();
    f.drain();
    fs::create_dir(&f.destination).unwrap();
    fs::write(f.destination.join("recovery"), b"retain").unwrap();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    assert!(lease.freeze(f.prepare()).is_err());
    assert_eq!(fs::read(f.destination.join("recovery")).unwrap(), b"retain");
    assert!(!f.archive.exists());
}

#[test]
fn destination_symlink_cannot_redirect_import_into_user_data() {
    let f = Fixture::new();
    f.drain();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    lease.freeze(f.prepare()).unwrap();
    std::os::unix::fs::symlink(&f.source, &f.destination).unwrap();
    assert!(lease.import().is_err());
    f.bundle.files.preflight(&f.source).unwrap();
}

#[tokio::test]
async fn copied_isolation_marker_cannot_bypass_transfer_or_open_another_state_writer() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("isolated");
    Host::init(&root).unwrap();
    let copied = d.path().join("copied");
    fs::create_dir(&copied).unwrap();
    fs::copy(
        root.join(".ag1-native-isolated"),
        copied.join(".ag1-native-isolated"),
    )
    .unwrap();
    assert!(
        Registry::for_user()
            .unwrap()
            .initialize_isolated(&copied)
            .is_err()
    );
    assert!(
        Host::open(&copied, &d.path().join("copied-state"), config())
            .await
            .is_err()
    );
    let first = Host::open(&root, &d.path().join("first"), config())
        .await
        .unwrap();
    assert!(
        Host::open(&root, &d.path().join("second"), config())
            .await
            .is_err()
    );
    first.shutdown().await.unwrap();
    drop(first);
    assert!(
        Host::open(&root, &d.path().join("second"), config())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn migrated_host_resnapshots_full_journal_and_denies_stale_client_and_ts_startup() {
    let f = Fixture::new();
    f.migrate();
    let host = Host::open_with_registry(&f.workspace, &f.destination, config(), &f.registry)
        .await
        .unwrap();
    let view = host.settled_view().await.unwrap();
    assert!(
        view.snapshot
            .messages
            .iter()
            .any(|m| m.text.contains("Retain this exact imported message"))
    );
    assert_eq!(view.snapshot.scope.session.as_str(), "migrated");
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
    let envelope = ag1_protocol::CommandEnvelope {
        version: Default::default(),
        id: "stale-command".to_string().try_into().unwrap(),
        scope: view.snapshot.scope.clone(),
        expected_revision: view.snapshot.revision,
        extension: None,
        command: ag1_protocol::Command::Prompt {
            input: ag1_protocol::PromptInput::Inline {
                text: "must not run".into(),
            },
        },
    };
    host.drain_for_cutover().await.unwrap();
    assert!(host.dispatch(envelope).await.is_err());
    assert!(host.check_authority().is_err());
    assert!(f.registry.resume_transfer(&f.workspace).is_err());
    drop(host);
    let lease = f.registry.resume_transfer(&f.workspace).unwrap();
    let rollback = f._dir.path().join("rollback-archive");
    lease.rollback_archive(&rollback).unwrap();
    let restored = f._dir.path().join("rollback-restored");
    Store::restore_export(&rollback, &restored).unwrap();
    assert_eq!(
        Store::open(&restored)
            .unwrap()
            .snapshot("migrated")
            .unwrap()
            .entries[0]
            .id,
        "user-one"
    );
    f.bundle.files.preflight(&f.source).unwrap();
    assert_eq!(
        fs::read(f.workspace.join("retained.txt")).unwrap(),
        b"original worktree bytes\n"
    );
}

#[test]
fn uncertain_client_effects_survive_rollback_as_receipts_never_session_commands() {
    let f = Fixture::new();
    f.migrate();
    let state = f.destination.join("store");
    {
        let store = Store::open(&state).unwrap();
        store.record_intent("uncertain-client", &serde_json::to_vec(&json!({"kind":"native-client-effect","session":"migrated","payload":{"scope":{"workspace":"workspace"}},"effect":"export"})).unwrap()).unwrap();
    }
    let mut owner = f
        .registry
        .open_native(&f.workspace, &f.destination)
        .unwrap();
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let lease = f.registry.resume_transfer(&f.workspace).unwrap();
    let archive = f._dir.path().join("rollback");
    assert_eq!(
        lease.rollback_archive(&archive).unwrap()["compatible_resume"],
        false
    );
    let restored = f._dir.path().join("restored");
    Store::restore_export(&archive, &restored).unwrap();
    let store = Store::open(&restored).unwrap();
    assert!(
        store
            .interrupted_commands("workspace", "migrated", None, 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .export(&f._dir.path().join("unsafe-ts"), true)
            .is_err()
    );
    assert!(archive.join("storage.sqlite").is_file());
}

#[test]
fn full_native_recovery_retains_private_auxiliary_state_and_never_grants_authority() {
    let f = Fixture::new();
    f.migrate();
    let files = [
        (
            "home/agent/settings.json",
            br#"{"unknown":false,"zero":0}"#.as_slice(),
        ),
        (
            "projects/owned-process.json",
            br#"{"pid":1311504,"historical":true}"#.as_slice(),
        ),
        (
            "terminal/output.bin",
            b"untruncated\x00output\xff".as_slice(),
        ),
        ("future/unknown-version", b"preserve exactly".as_slice()),
    ];
    for (name, bytes) in files {
        let path = f.destination.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    fs::create_dir_all(f.destination.join("empty/retained")).unwrap();
    let mut owner = f
        .registry
        .open_native(&f.workspace, &f.destination)
        .unwrap();
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let lease = f.registry.resume_transfer(&f.workspace).unwrap();
    let archive = f._dir.path().join("full-recovery");
    let receipt = lease.rollback_archive(&archive).unwrap();
    let hash = receipt["cutover_recovery_sha256"].as_str().unwrap();
    let restored = f._dir.path().join("full-restored");
    let before = f.registry.inspect(&f.workspace).unwrap();
    let manifest =
        ag1_platform::cutover::restore_native_archive(&archive, &restored, hash).unwrap();
    assert_eq!(manifest.authority, before.clone().unwrap());
    assert_eq!(f.registry.inspect(&f.workspace).unwrap(), before);
    for (name, bytes) in files {
        assert_eq!(fs::read(restored.join(name)).unwrap(), bytes);
        assert_eq!(fs::read(f.destination.join(name)).unwrap(), bytes);
    }
    assert!(restored.join("empty/retained").is_dir());
    assert_eq!(
        Store::open(&restored.join("store"))
            .unwrap()
            .snapshot("migrated")
            .unwrap()
            .entries
            .len(),
        1
    );
    assert!(f.registry.open_native(&f.workspace, &restored).is_err());
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
    assert!(ag1_platform::cutover::restore_native_archive(&archive, &restored, hash).is_err());
    f.bundle.files.preflight(&f.source).unwrap();
}

#[test]
fn recovery_rejects_tampered_missing_injected_and_symlink_auxiliary_files() {
    for mutation in ["tamper", "missing", "injected", "symlink", "manifest"] {
        let f = Fixture::new();
        f.migrate();
        let mut owner = f
            .registry
            .open_native(&f.workspace, &f.destination)
            .unwrap();
        owner.begin_drain().unwrap();
        owner.quiesced().unwrap();
        drop(owner);
        let lease = f.registry.resume_transfer(&f.workspace).unwrap();
        let archive = f._dir.path().join("recovery");
        let receipt = lease.rollback_archive(&archive).unwrap();
        let path = archive.join("native-state/host.json");
        match mutation {
            "tamper" => fs::write(path, b"changed").unwrap(),
            "missing" => fs::remove_file(path).unwrap(),
            "injected" => fs::write(archive.join("native-state/extra"), b"injected").unwrap(),
            "symlink" => {
                fs::remove_file(&path).unwrap();
                std::os::unix::fs::symlink(f.destination.join("host.json"), path).unwrap();
            }
            "manifest" => fs::write(archive.join("recovery.json"), b"{}").unwrap(),
            _ => unreachable!(),
        }
        let restored = f._dir.path().join("must-not-publish");
        assert!(
            ag1_platform::cutover::restore_native_archive(
                &archive,
                &restored,
                receipt["cutover_recovery_sha256"].as_str().unwrap()
            )
            .is_err(),
            "{mutation}"
        );
        assert!(!restored.exists());
        assert!(archive.is_dir());
        assert_eq!(fs::read(f.destination.join("host.json")).unwrap(), f.config);
    }
}

#[test]
fn auxiliary_symlinks_and_nested_archive_destinations_block_recovery_export() {
    let f = Fixture::new();
    f.migrate();
    let mut owner = f
        .registry
        .open_native(&f.workspace, &f.destination)
        .unwrap();
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let lease = f.registry.resume_transfer(&f.workspace).unwrap();
    assert!(
        lease
            .rollback_archive(&f.destination.join("nested"))
            .is_err()
    );
    assert!(
        lease
            .rollback_archive(&f.workspace.join("worktree-archive"))
            .is_err()
    );
    assert!(!f.destination.join("nested").exists());
    assert!(!f.workspace.join("worktree-archive").exists());
    std::os::unix::fs::symlink(&f.source, f.destination.join("foreign-state")).unwrap();
    let archive = f._dir.path().join("unsafe-recovery");
    assert!(lease.rollback_archive(&archive).is_err());
    assert!(!archive.exists());
    f.bundle.files.preflight(&f.source).unwrap();
}

fn runtime_import_request(
    f: &Fixture,
    capture: &serde_json::Value,
) -> ag1_platform::cutover::configuration::ImportRequest {
    use ag1_platform::cutover::configuration::*;
    let server = f._dir.path().join("pinned-server");
    let weight = f._dir.path().join("pinned-weight.gguf");
    let library = f._dir.path().join("lib-pinned.so");
    let mut materials = Vec::new();
    for (path, bytes, kind) in [
        (&server, b"fixture executable".as_slice(), "executable"),
        (&weight, b"fixture weight".as_slice(), "weight"),
        (&library, b"fixture library".as_slice(), "library"),
    ] {
        fs::write(path, bytes).unwrap();
        materials.push(ag1_platform::llama::Material {
            path: path.display().to_string(),
            sha256: digest(bytes),
            kind: kind.into(),
        });
    }
    let raw = serde_json::to_string(capture)
        .unwrap()
        .replace(
            capture["observed"]["command"].as_str().unwrap(),
            server.to_str().unwrap(),
        )
        .replace(
            &format!("<fixture>/{}.gguf", capture["model"].as_str().unwrap()),
            weight.to_str().unwrap(),
        );
    let c: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let profile = serde_json::to_vec_pretty(
        &json!({"version":1,"models":{c["model"].as_str().unwrap():c["inputs"]["profile"]}}),
    )
    .unwrap();
    let environment = serde_json::to_vec(&c["inputs"]["environment"]).unwrap();
    let pin = |name: &str, bytes: &[u8]| {
        let path = f._dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        PinnedInput {
            path,
            sha256: digest(bytes),
        }
    };
    ImportRequest {
        profile: pin("raw-profiles.json", &profile),
        inherited_environment: pin("private-env.json", &environment),
        additional_inputs: [
            (
                "settings".into(),
                pin(
                    "raw-settings.json",
                    br#"{ "future": false, "zero": 0, "ordered": [3, 1, 2] }"#,
                ),
            ),
            (
                "credentials".into(),
                pin(
                    "private-vault.json",
                    br#"{"fixture":{"type":"api_key","key":"fixture-secret-do-not-report"}}"#,
                ),
            ),
        ]
        .into(),
        physical_cores: 12,
        logical_cores: 24,
        vram_mib: Some(24576),
        models: vec![ModelImport {
            model_id: c["model"].as_str().unwrap().into(),
            model_path: weight.display().to_string(),
            context_override: c["inputs"]["contextOverride"].as_u64(),
            forced_moe: None,
            cached_moe: None,
            discovered_projector: None,
            materials,
            expected: ExpectedLaunch {
                command: c["observed"]["command"].as_str().unwrap().into(),
                args: serde_json::from_value(c["observed"]["args"].clone()).unwrap(),
                environment: serde_json::from_value(c["observed"]["environment"].clone()).unwrap(),
                context_window: c["observed"]["providerConfig"]["models"][0]["contextWindow"]
                    .as_u64()
                    .unwrap(),
                max_tokens: c["observed"]["providerConfig"]["models"][0]["maxTokens"]
                    .as_u64()
                    .unwrap(),
            },
        }],
    }
}

#[test]
fn configuration_import_matches_all_nine_ts_captures_and_retains_private_raw_inputs() {
    use std::os::unix::fs::PermissionsExt;
    let captures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/llama/launch-plans.json"
    ))
    .unwrap();
    assert_eq!(captures["captures"].as_array().unwrap().len(), 9);
    for capture in captures["captures"].as_array().unwrap() {
        let f = Fixture::new();
        let request = runtime_import_request(&f, capture);
        let path = f._dir.path().join("private-capture.json");
        let imported =
            ag1_platform::cutover::configuration::import_configuration(&path, &request).unwrap();
        assert_eq!(imported.verify().unwrap().len(), 1);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let stored: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (name, input) in [
            ("profile", &request.profile),
            ("environment", &request.inherited_environment),
        ]
        .into_iter()
        .chain(
            request
                .additional_inputs
                .iter()
                .map(|(name, input)| (name.as_str(), input)),
        ) {
            let bytes: Vec<u8> = serde_json::from_value(stored["inputs"][name].clone()).unwrap();
            assert_eq!(bytes, fs::read(&input.path).unwrap());
            assert_eq!(digest(&bytes), input.sha256);
        }
        assert!(
            !serde_json::to_string(&imported)
                .unwrap()
                .contains("fixture-secret")
        );
        assert!(
            ag1_platform::cutover::configuration::import_configuration(&path, &request).is_err()
        );
    }
}

#[test]
fn configuration_import_rejects_launch_drift_missing_models_and_changed_materials() {
    let captures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/llama/launch-plans.json"
    ))
    .unwrap();
    for drift in [
        "args",
        "environment",
        "limits",
        "duplicate",
        "missing",
        "library",
        "profile",
        "vault",
    ] {
        let f = Fixture::new();
        let mut request = runtime_import_request(&f, &captures["captures"][0]);
        match drift {
            "args" => request.models[0].expected.args.push("--retune".into()),
            "environment" => {
                request.models[0]
                    .expected
                    .environment
                    .insert("IMPLICIT_CHANGE".into(), "1".into());
            }
            "limits" => request.models[0].expected.context_window += 1,
            "duplicate" => request.models.push(request.models[0].clone()),
            "missing" => request.models.clear(),
            "library" => {
                fs::write(&request.models[0].materials[2].path, b"changed library").unwrap()
            }
            "profile" => fs::write(&request.profile.path, b"{}").unwrap(),
            "vault" => fs::write(&request.additional_inputs["credentials"].path, b"{}").unwrap(),
            _ => unreachable!(),
        }
        let path = f._dir.path().join("must-not-import.json");
        assert!(
            ag1_platform::cutover::configuration::import_configuration(&path, &request).is_err(),
            "{drift}"
        );
        assert!(!path.exists());
    }
}

#[test]
fn frozen_configuration_is_self_contained_and_rechecks_original_runtime_before_import() {
    let captures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/llama/launch-plans.json"
    ))
    .unwrap();
    for drift in [false, true] {
        let mut f = Fixture::new();
        let request = runtime_import_request(&f, &captures["captures"][0]);
        let path = f._dir.path().join("capture.json");
        f.bundle.runtime_configuration = Some(
            ag1_platform::cutover::configuration::import_configuration(&path, &request).unwrap(),
        );
        f.drain();
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        lease.freeze(f.prepare()).unwrap();
        fs::remove_file(path).unwrap();
        if drift {
            fs::write(&request.models[0].materials[2].path, b"drift").unwrap();
        }
        assert_eq!(lease.import().is_ok(), !drift);
        assert_eq!(
            f.registry.inspect(&f.workspace).unwrap().unwrap().stage,
            if drift {
                Stage::Frozen
            } else {
                Stage::Imported
            }
        );
        assert!(f.archive.join("runtime-configuration.json").is_file());
        f.bundle.files.preflight(&f.source).unwrap();
    }
}

#[test]
fn administrative_cli_restores_archive_and_captures_configuration_without_runtime_admission() {
    let f = Fixture::new();
    f.migrate();
    let mut owner = f
        .registry
        .open_native(&f.workspace, &f.destination)
        .unwrap();
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let lease = f.registry.resume_transfer(&f.workspace).unwrap();
    let archive = f._dir.path().join("cli-archive");
    let receipt = lease.rollback_archive(&archive).unwrap();
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/ag1-cutover");
    let restored = f._dir.path().join("cli-restored");
    let invoke = |hash: &str| {
        std::process::Command::new(&cli)
            .arg("restore-native")
            .arg(&archive)
            .arg(hash)
            .arg(&restored)
            .output()
            .unwrap()
    };
    assert!(!invoke(&"0".repeat(64)).status.success());
    assert!(!restored.exists());
    let result = invoke(receipt["cutover_recovery_sha256"].as_str().unwrap());
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["authority_transferred"], false);
    assert_eq!(report["runtime_control_granted"], false);
    assert!(
        !invoke(receipt["cutover_recovery_sha256"].as_str().unwrap())
            .status
            .success()
    );
    assert!(f.registry.open_native(&f.workspace, &restored).is_err());
    let captures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/llama/launch-plans.json"
    ))
    .unwrap();
    let request = runtime_import_request(&f, &captures["captures"][0]);
    let request_path = f._dir.path().join("cli-request.json");
    fs::write(&request_path, serde_json::to_vec(&request).unwrap()).unwrap();
    let capture = f._dir.path().join("cli-capture.json");
    let result = std::process::Command::new(cli)
        .arg("capture-runtime")
        .arg(request_path)
        .arg(&capture)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("fixture-secret"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("fixture-secret"));
    let report: ag1_platform::cutover::configuration::ConfigurationCapture =
        serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report.path, capture);
    assert_eq!(report.verify().unwrap().len(), 1);
    assert_eq!(
        f.registry.inspect(&f.workspace).unwrap().unwrap().stage,
        Stage::Draining
    );
}

fn begin_native_rollback(f: &Fixture) -> ag1_platform::cutover::Lease {
    let mut owner = f
        .registry
        .open_native(&f.workspace, &f.destination)
        .unwrap();
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    f.registry.resume_transfer(&f.workspace).unwrap()
}
const TS_MODEL: &str = "Qwen3.6-35B-A3B-UD-Q4_K_XL";

#[test]
fn rollback_restarts_at_each_boundary_and_publishes_exactly_one_new_ts_owner() {
    let f = Fixture::new();
    f.migrate();
    let destination = f._dir.path().join("new-ts-state");
    let archive = f._dir.path().join("native-rollback");
    let mut lease = begin_native_rollback(&f);
    lease
        .freeze_rollback(&archive, &destination, "migrated", TS_MODEL)
        .unwrap();
    for step in 0..3 {
        assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
        assert!(
            f.registry
                .open_native(&f.workspace, &f.destination)
                .is_err()
        );
        drop(lease);
        lease = f.registry.resume_transfer(&f.workspace).unwrap();
        if step == 0 {
            lease.prepare_rollback().unwrap();
        }
        if step == 1 {
            lease.prepare_rollback().unwrap();
        }
        if step == 2 {
            lease.commit_rollback().unwrap();
        }
    }
    assert!(lease.commit_rollback().is_err());
    drop(lease);
    let owner = f
        .registry
        .open_typescript(&f.workspace, &destination)
        .unwrap();
    owner.admit().unwrap();
    assert_eq!(owner.record().unwrap().epoch, 3);
    assert!(
        f.registry
            .open_native(&f.workspace, &f.destination)
            .is_err()
    );
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
    assert!(f.archive.join("source/session.jsonl").is_file());
    assert!(archive.join("recovery.json").is_file());
    assert_eq!(
        fs::read(destination.join("opaque.json")).unwrap(),
        fs::read(f.source.join("opaque.json")).unwrap()
    );
    f.bundle.files.preflight(&f.source).unwrap();
}

fn spawn_rollback_worker(f: &Fixture, destination: &Path) -> ag1_desktop::Worker {
    use ag1_platform::process::ProcessSpec;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap();
    let authority = f
        .registry
        .open_typescript(&f.workspace, destination)
        .unwrap();
    let mut worker = ag1_desktop::Worker::spawn_authorized(
        ProcessSpec {
            resident: false,
            executable: root.join(".local/toolchains/node-v24.14.0-linux-x64/bin/node"),
            argv: vec![
                root.join("product/compat/agent-worker/worker.ts")
                    .display()
                    .to_string(),
                root.join("reference/ag1-bridge").display().to_string(),
                destination.display().to_string(),
                f.workspace.display().to_string(),
                ag1_protocol::schema_hash().to_owned(),
                root.join("product/target/debug/ag1-desktop")
                    .display()
                    .to_string(),
                root.join(".local/bridge-runtime.json")
                    .display()
                    .to_string(),
                "true".into(),
            ],
            cwd: f.workspace.clone(),
            env: [
                ("PATH".into(), "/usr/bin:/bin".into()),
                (
                    "HOME".into(),
                    f._dir.path().join("home").display().to_string(),
                ),
                ("AG1_LLAMA_CPP".into(), "0".into()),
            ]
            .into(),
            timeout_ms: 60000,
            grace_ms: 1000,
        },
        &root.join("product/target/debug/ag1-process-host"),
        &f._dir.path().join(format!(
            "worker-{}.log",
            ag1_platform::client::nonce().unwrap()
        )),
        authority,
    )
    .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while worker.view()["snapshot"].is_null() {
        if std::time::Instant::now() >= until || !worker.view()["error"].is_null() {
            let error = worker.view();
            worker.stop();
            panic!("Rollback worker did not start: {error}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    worker
}
fn ts_command(
    worker: &ag1_desktop::Worker,
    id: &str,
    command: ag1_protocol::Command,
) -> ag1_protocol::CommandEnvelope {
    let snapshot = &worker.view()["snapshot"];
    ag1_protocol::CommandEnvelope {
        version: Default::default(),
        id: id.to_owned().try_into().unwrap(),
        scope: serde_json::from_value(snapshot["scope"].clone()).unwrap(),
        expected_revision: serde_json::from_value(snapshot["revision"].clone()).unwrap(),
        command,
        extension: None,
    }
}

#[test]
fn real_ts_rollback_resnapshots_native_progress_and_inspects_uncertain_effects_without_replay() {
    use ag1_protocol::{BridgeFrame, Command, PromptInput};
    let f = Fixture::new();
    fs::write(f.workspace.join("README.md"), b"Rollback tool evidence\n").unwrap();
    f.migrate();
    {
        let store = Store::open(&f.destination.join("store")).unwrap();
        let revision = store.snapshot("migrated").unwrap().revision;
        store.append_batch("migrated", "native-progress", revision, &[serde_json::to_vec(&json!({"type":"message","id":"native-note","parentId":"user-one","timestamp":"2026-09-07T01:00:00Z","message":{"role":"user","content":"Progress created by native execution","timestamp":2}})).unwrap()]).unwrap();
        let revision = store.snapshot("migrated").unwrap().revision;
        store.admit_client_command("migrated", "native-client-reserved", revision, &json!({"kind":"client-effect","envelope":{"id":"uncertain-write","command":{"kind":"files","action":"save","args":{"path":"retained.txt","text":"must not replay"}}}})).unwrap();
    }
    let destination = f._dir.path().join("ts-resume");
    let mut lease = begin_native_rollback(&f);
    lease
        .freeze_rollback(
            &f._dir.path().join("rollback"),
            &destination,
            "migrated",
            TS_MODEL,
        )
        .unwrap();
    lease.prepare_rollback().unwrap();
    lease.commit_rollback().unwrap();
    drop(lease);
    let mut worker = spawn_rollback_worker(&f, &destination);
    assert!(
        worker
            .view()
            .to_string()
            .contains("Progress created by native execution")
    );
    let replay = ts_command(
        &worker,
        "uncertain-write",
        Command::Files {
            action: "save".into(),
            args: json!({"path":"retained.txt","text":"must not replay"}),
        },
    );
    assert!(
        matches!(worker.command(replay).unwrap(), BridgeFrame::Reply { status, data, .. }
        if status == "rejected" && data.to_string().contains("No replay is allowed"))
    );
    assert_eq!(
        fs::read(f.workspace.join("retained.txt")).unwrap(),
        b"original worktree bytes\n"
    );
    let receipt = worker
        .command(ts_command(
            &worker,
            "inspect",
            Command::CommandResult {
                command: "uncertain-write".to_owned().try_into().unwrap(),
            },
        ))
        .unwrap();
    assert!(
        serde_json::to_string(&receipt)
            .unwrap()
            .contains("native-client-reserved")
    );
    let detail = worker
        .command(ts_command(
            &worker,
            "detail",
            Command::Runtime {
                action: "migration_receipt".into(),
                args: json!({"key":"native-client-reserved"}),
            },
        ))
        .unwrap();
    let BridgeFrame::Reply { data, .. } = detail else {
        panic!("Expected receipt detail");
    };
    assert_eq!(data["replay_allowed"], false);
    assert_eq!(data["state"], "uncertain");
    let artifact = data["evidence"].as_str().unwrap();
    let detail = fs::read_to_string(
        destination
            .join("evidence")
            .join(format!("{artifact}.json")),
    )
    .unwrap();
    assert!(detail.contains("native-client-effect") && detail.contains("must not replay"));
    let prompt = ts_command(
        &worker,
        "after-rollback",
        Command::Prompt {
            input: PromptInput::Inline {
                text: "[tool] Read README.md".into(),
            },
        },
    );
    assert!(
        matches!(worker.command(prompt).unwrap(), BridgeFrame::Reply { status, .. } if status == "accepted")
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let view = worker.view();
        if view["snapshot"]["status"] == "idle"
            && view["snapshot"]["message_count"].as_u64().unwrap_or(0) >= 6
        {
            break;
        }
        assert!(std::time::Instant::now() < until, "{view}");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    worker.stop();
    drop(worker);
    let mut reopened = spawn_rollback_worker(&f, &destination);
    assert!(
        reopened
            .view()
            .to_string()
            .contains("Progress created by native execution")
    );
    reopened.stop();
    f.bundle.files.preflight(&f.source).unwrap();
}

#[test]
fn unsupported_ts_downgrade_and_candidate_tamper_keep_archives_and_native_recovery() {
    for unsupported in [true, false] {
        let f = Fixture::new();
        f.migrate();
        if unsupported {
            let store = Store::open(&f.destination.join("store")).unwrap();
            let revision = store.snapshot("migrated").unwrap().revision;
            store
                .append_batch(
                    "migrated",
                    "future-entry",
                    revision,
                    &[serde_json::to_vec(
                        &json!({"type":"future-v99","id":"future","parentId":"user-one"}),
                    )
                    .unwrap()],
                )
                .unwrap();
        }
        let destination = f._dir.path().join("ts-candidate");
        let archive = f._dir.path().join("rollback");
        let mut lease = begin_native_rollback(&f);
        lease
            .freeze_rollback(&archive, &destination, "migrated", TS_MODEL)
            .unwrap();
        if unsupported {
            assert!(lease.prepare_rollback().is_err());
            assert!(!destination.exists());
        } else {
            lease.prepare_rollback().unwrap();
            fs::write(destination.join("sessions/migrated.jsonl"), b"tampered").unwrap();
            assert!(lease.commit_rollback().is_err());
        }
        assert!(archive.join("recovery.json").is_file());
        assert!(f.archive.join("bundle.json").is_file());
        lease.cancel_rollback().unwrap();
        drop(lease);
        let owner = f
            .registry
            .open_native(&f.workspace, &f.destination)
            .unwrap();
        owner.admit().unwrap();
        assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
        f.bundle.files.preflight(&f.source).unwrap();
    }
}

#[test]
fn rollback_worker_restores_the_selected_branch_without_discarding_other_entries() {
    let f = Fixture::new();
    f.migrate();
    {
        let store = Store::open(&f.destination.join("store")).unwrap();
        let revision = store.snapshot("migrated").unwrap().revision;
        store.append_batch("migrated", "other-branch", revision, &[serde_json::to_vec(&json!({"type":"message","id":"other-note","parentId":"user-one","timestamp":"2026-09-07T01:00:00Z","message":{"role":"user","content":"Unselected branch remains archived","timestamp":2}})).unwrap()]).unwrap();
        let revision = store.snapshot("migrated").unwrap().revision;
        store
            .select_leaf("migrated", "select-old-leaf", revision, Some("user-one"))
            .unwrap();
    }
    let destination = f._dir.path().join("ts-branch");
    let mut lease = begin_native_rollback(&f);
    lease
        .freeze_rollback(
            &f._dir.path().join("rollback"),
            &destination,
            "migrated",
            TS_MODEL,
        )
        .unwrap();
    lease.prepare_rollback().unwrap();
    lease.commit_rollback().unwrap();
    drop(lease);
    for _ in 0..2 {
        let mut worker = spawn_rollback_worker(&f, &destination);
        assert!(
            worker
                .view()
                .to_string()
                .contains("Retain this exact imported message")
        );
        assert!(
            !worker
                .view()
                .to_string()
                .contains("Unselected branch remains archived")
        );
        worker.stop();
    }
    assert!(
        fs::read_to_string(destination.join("sessions/migrated.jsonl"))
            .unwrap()
            .contains("Unselected branch remains archived")
    );
    f.bundle.files.preflight(&f.source).unwrap();
}

#[test]
fn corrupt_memory_blocks_ts_resume_and_forked_old_cursors_survive_lossless_recovery() {
    let mut f = Fixture::new();
    let raw = b"not a valid memory document";
    fs::write(f.source.join("memory.json"), raw).unwrap();
    f.bundle.files.files.push(FrozenFile {
        path: "memory.json".into(),
        sha256: digest(raw),
        bytes: raw.len() as u64,
        kind: FileKind::Memory,
        session: Some("migrated".into()),
        origin: None,
    });
    f.migrate();
    let mut lease = begin_native_rollback(&f);
    let archive = f._dir.path().join("corrupt-rollback");
    lease
        .freeze_rollback(
            &archive,
            &f._dir.path().join("unsafe-ts"),
            "migrated",
            TS_MODEL,
        )
        .unwrap();
    assert!(lease.prepare_rollback().is_err());
    assert!(archive.join("recovery.json").is_file());
    lease.cancel_rollback().unwrap();
    drop(lease);

    let f = Fixture::new();
    f.migrate();
    let cursor;
    {
        let store = Store::open(&f.destination.join("store")).unwrap();
        let bytes = "Old cursor: retained β evidence".as_bytes().to_vec();
        let record = json!({"version":1,"id":"art_00000000000000000000000000000001","sessionId":"migrated","kind":"text","mimeType":"text/plain","encoding":"utf8","createdAt":"2026-09-07T00:00:00Z","sha256":digest(&bytes),"bytes":bytes.len(),"producer":{"type":"system","name":"cutover"}});
        let id = store
            .put_artifact(&ag1_storage::ArtifactInput {
                origin: "migrated".into(),
                session: "migrated".into(),
                record: serde_json::to_vec(&record).unwrap(),
                bytes,
            })
            .unwrap();
        cursor = store.cursor("migrated", "migrated", &id, 0, 128).unwrap();
        store.fork("migrated", "child", None).unwrap();
        store.archive_session("migrated").unwrap();
        store.collect_orphans().unwrap();
        assert!(store.read_cursor("child", &cursor).is_ok());
    }
    let mut lease = begin_native_rollback(&f);
    let archive = f._dir.path().join("fork-rollback");
    lease
        .freeze_rollback(
            &archive,
            &f._dir.path().join("unsafe-fork-ts"),
            "child",
            TS_MODEL,
        )
        .unwrap();
    assert!(lease.prepare_rollback().is_err());
    let restored = f._dir.path().join("native-restored");
    Store::restore_export(&archive, &restored).unwrap();
    let store = Store::open(&restored).unwrap();
    assert!(store.read_cursor("child", &cursor).is_ok());
    store.create_session("unrelated", &serde_json::to_vec(&json!({"type":"session","version":3,"id":"unrelated","cwd":f.workspace,"timestamp":"2026-09-07T00:00:00Z"})).unwrap()).unwrap();
    assert!(store.read_cursor("unrelated", &cursor).is_err());
    f.bundle.files.preflight(&f.source).unwrap();
}

#[test]
fn abrupt_exit_at_each_rollback_boundary_recovers_without_reopening_stale_ts_state() {
    if let Some(root) = std::env::var_os("AG1_ROLLBACK_CRASH_ROOT") {
        let root = PathBuf::from(root);
        let step: usize = std::env::var("AG1_ROLLBACK_CRASH_STEP")
            .unwrap()
            .parse()
            .unwrap();
        let registry = Registry::at(&root.join("authority")).unwrap();
        let mut owner = registry
            .open_native(&root.join("workspace"), &root.join("destination"))
            .unwrap();
        owner.begin_drain().unwrap();
        owner.quiesced().unwrap();
        if step == 0 {
            std::process::exit(92);
        }
        drop(owner);
        let mut lease = registry.resume_transfer(&root.join("workspace")).unwrap();
        lease
            .freeze_rollback(
                &root.join("rollback"),
                &root.join("new-ts"),
                "migrated",
                TS_MODEL,
            )
            .unwrap();
        if step == 1 {
            std::process::exit(92);
        }
        lease.prepare_rollback().unwrap();
        if step == 2 {
            std::process::exit(92);
        }
        lease.commit_rollback().unwrap();
        std::process::exit(92);
    }
    for step in 0..4 {
        let f = Fixture::new();
        f.migrate();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "abrupt_exit_at_each_rollback_boundary_recovers_without_reopening_stale_ts_state",
            ])
            .env("AG1_ROLLBACK_CRASH_ROOT", f._dir.path())
            .env("AG1_ROLLBACK_CRASH_STEP", step.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(92),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if step < 3 {
            let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
            if step == 0 {
                lease
                    .freeze_rollback(
                        &f._dir.path().join("rollback"),
                        &f._dir.path().join("new-ts"),
                        "migrated",
                        TS_MODEL,
                    )
                    .unwrap();
            }
            lease.prepare_rollback().unwrap();
            lease.commit_rollback().unwrap();
        }
        let owner = f
            .registry
            .open_typescript(&f.workspace, &f._dir.path().join("new-ts"))
            .unwrap();
        owner.admit().unwrap();
        assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
        assert!(
            f.registry
                .open_native(&f.workspace, &f.destination)
                .is_err()
        );
        f.bundle.files.preflight(&f.source).unwrap();
    }
}

#[test]
fn production_configuration_cannot_use_fixture_acceptance() {
    let mut f = Fixture::new();
    init_source_git(&f.workspace);
    let mut cfg = config();
    cfg.fixture = false;
    f.config = serde_json::to_vec(&cfg).unwrap();
    f.bundle.config_sha256 = digest(&f.config);
    f.drain();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    assert!(lease.freeze(f.prepare()).is_err());
    assert!(!f.archive.exists());
    f.bundle.source_snapshot = Some(
        lease
            .snapshot_source(&f._dir.path().join("source-archive"))
            .unwrap(),
    );
    lease.freeze(f.prepare()).unwrap();
    lease.import().unwrap();
    lease.validate().unwrap();
    lease.revoke().unwrap();
    assert!(lease.commit_fixture().is_err());
    assert_eq!(lease.record().unwrap().stage, Stage::Revoked);
}

#[test]
fn final_authority_audit_requires_quiescence_and_retained_untampered_state() {
    for tamper in [
        "none",
        "original",
        "archive",
        "host",
        "staged",
        "source-snapshot",
    ] {
        let mut f = Fixture::new();
        init_source_git(&f.workspace);
        assert!(f.registry.audit_cutover(&f.workspace).is_err());
        f.drain();
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        f.bundle.source_snapshot = Some(
            lease
                .snapshot_source(&f._dir.path().join("source-archive"))
                .unwrap(),
        );
        lease.freeze(f.prepare()).unwrap();
        lease.import().unwrap();
        lease.validate().unwrap();
        lease.revoke().unwrap();
        assert!(f.registry.audit_cutover(&f.workspace).is_err());
        lease.commit_fixture().unwrap();
        drop(lease);
        let before = f.registry.inspect(&f.workspace).unwrap();
        let report = f.registry.audit_cutover(&f.workspace).unwrap();
        assert_eq!(report["status"], "fixture_verified");
        assert_eq!(report["fixture"], true);
        assert_eq!(report["runtime_control_granted"], false);
        assert_eq!(report["phase_acceptance"], false);
        assert_eq!(f.registry.inspect(&f.workspace).unwrap(), before);
        let mut owner = f
            .registry
            .open_native(&f.workspace, &f.destination)
            .unwrap();
        assert!(f.registry.audit_cutover(&f.workspace).is_err());
        owner.quiesced().unwrap();
        drop(owner);
        match tamper {
            "original" => fs::write(f.source.join("opaque.json"), "changed").unwrap(),
            "archive" => fs::write(f.archive.join("source/opaque.json"), "changed").unwrap(),
            "host" => fs::write(f.destination.join("host.json"), "{}").unwrap(),
            "staged" => fs::write(f.destination.join("store/staging/pending"), "retain").unwrap(),
            "source-snapshot" => fs::write(
                f.bundle
                    .source_snapshot
                    .as_ref()
                    .unwrap()
                    .path
                    .join("files/retained.txt"),
                "changed",
            )
            .unwrap(),
            _ => fs::write(f.workspace.join("retained.txt"), "valid later source work").unwrap(),
        }
        let result = f.registry.audit_cutover(&f.workspace);
        assert_eq!(result.is_ok(), tamper == "none", "{tamper}");
        if tamper == "staged" {
            assert_eq!(
                fs::read(f.destination.join("store/staging/pending")).unwrap(),
                b"retain"
            );
            assert!(!f.destination.join("store/quarantine/pending").exists());
        }
    }
}

fn init_source_git(workspace: &Path) {
    let result = std::process::Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null", "init", "--quiet"])
        .current_dir(workspace)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(result.status.success());
}

#[test]
fn source_snapshot_drift_fences_each_transfer_boundary_without_rolling_back_source() {
    for boundary in 0..4 {
        let mut f = Fixture::new();
        init_source_git(&f.workspace);
        f.drain();
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        let snapshot = lease
            .snapshot_source(&f._dir.path().join("source-archive"))
            .unwrap();
        f.bundle.source_snapshot = Some(snapshot.clone());
        if boundary > 0 {
            lease.freeze(f.prepare()).unwrap();
        }
        if boundary > 1 {
            lease.import().unwrap();
        }
        if boundary > 2 {
            lease.validate().unwrap();
        }
        fs::write(
            f.workspace.join("retained.txt"),
            "changed by an external editor",
        )
        .unwrap();
        let result = match boundary {
            0 => lease.freeze(f.prepare()),
            1 => lease.import(),
            2 => lease.validate(),
            _ => lease.revoke(),
        };
        assert!(result.is_err());
        assert_eq!(lease.record().unwrap().owner, Owner::TypeScript);
        snapshot.verify_archive(&f.workspace).unwrap();
        assert_eq!(
            fs::read(f.workspace.join("retained.txt")).unwrap(),
            b"changed by an external editor"
        );
        if boundary > 0 {
            lease.cancel_unaccepted().unwrap();
            assert_eq!(lease.record().unwrap().stage, Stage::Active);
            assert!(snapshot.path.exists());
            assert_eq!(
                fs::read(f.workspace.join("retained.txt")).unwrap(),
                b"changed by an external editor"
            );
        }
    }
}

#[test]
fn quiescent_drain_can_be_cancelled_before_any_archive_or_import() {
    let f = Fixture::new();
    f.drain();
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    lease.cancel_transfer().unwrap();
    drop(lease);
    f.registry
        .open_typescript(&f.workspace, &f.source)
        .unwrap()
        .admit()
        .unwrap();
    assert!(!f.archive.exists());
}

#[test]
fn registry_rejects_symlinks_and_replaced_workspace_identity() {
    let f = Fixture::new();
    let alias = f._dir.path().join("alias");
    std::os::unix::fs::symlink(f._dir.path().join("authority"), &alias).unwrap();
    assert!(Registry::at(&alias).is_err());
    drop(f.registry.open_typescript(&f.workspace, &f.source).unwrap());
    fs::rename(&f.workspace, f._dir.path().join("old-workspace")).unwrap();
    fs::create_dir(&f.workspace).unwrap();
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
}

#[test]
fn fixture_import_retains_raw_false_zero_order_and_full_source_archive() {
    let f = Fixture::new();
    f.migrate();
    for row in &f.bundle.files.files {
        assert_eq!(
            fs::read(f.source.join(&row.path)).unwrap(),
            fs::read(f.archive.join("source").join(&row.path)).unwrap()
        );
    }
    assert_eq!(fs::read(f.destination.join("host.json")).unwrap(), f.config);
    assert!(Path::new(&f.archive).join("bundle.json").is_file());
}

#[test]
fn runtime_binary_library_weight_projector_and_profile_drift_block_import() {
    for kind in ["executable", "library", "weight", "projector", "profile"] {
        let mut f = Fixture::new();
        let material = f._dir.path().join(kind);
        fs::write(&material, b"pinned bytes").unwrap();
        f.bundle.runtime_files.push(ag1_platform::llama::Material {
            path: material.to_str().unwrap().into(),
            sha256: digest(b"pinned bytes"),
            kind: kind.into(),
        });
        f.drain();
        let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
        lease.freeze(f.prepare()).unwrap();
        fs::write(&material, b"changed bytes").unwrap();
        assert!(lease.import().is_err());
        assert_eq!(lease.record().unwrap().stage, Stage::Frozen);
    }
}

#[test]
fn freeze_must_cover_the_drained_state_not_an_unrelated_valid_bundle() {
    let f = Fixture::new();
    let another = f._dir.path().join("another-state");
    fs::create_dir(&another).unwrap();
    let mut owner = f.registry.open_typescript(&f.workspace, &another).unwrap();
    owner.begin_drain().unwrap();
    owner.quiesced().unwrap();
    drop(owner);
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    assert!(lease.freeze(f.prepare()).is_err());
    assert!(!f.archive.exists());
}

#[test]
fn endpoint_aliases_and_competing_hosts_share_one_permit() {
    let f = Fixture::new();
    let first = f
        .registry
        .endpoint_leases(&[
            "http://127.0.0.1:34567".into(),
            "http://localhost:34567".into(),
        ])
        .unwrap();
    assert_eq!(first.len(), 1);
    assert!(
        f.registry
            .endpoint_leases(&["http://[::1]:34567".into()])
            .is_err()
    );
    assert!(
        f.registry
            .endpoint_leases(&["https://localhost:34567".into()])
            .is_err()
    );
    drop(first);
    assert_eq!(
        f.registry
            .endpoint_leases(&["http://localhost:34567".into()])
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unclean_typescript_exit_is_not_implicit_permission_to_restart() {
    let f = Fixture::new();
    drop(f.registry.open_typescript(&f.workspace, &f.source).unwrap());
    assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
    assert!(f.registry.resume_transfer(&f.workspace).is_err());
}

#[tokio::test]
async fn native_live_endpoint_requires_control_not_just_an_isolated_workspace() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("workspace");
    Host::init(&root).unwrap();
    let mut cfg = config();
    cfg.fixture = false;
    let error = Host::open(&root, &d.path().join("state"), cfg)
        .await
        .err()
        .unwrap();
    assert!(format!("{error:?}").contains("ownership has not been transferred"));
}

#[test]
fn abrupt_child_exit_at_each_transfer_stage_is_recoverable() {
    if let Some(root) = std::env::var_os("AG1_CUTOVER_CRASH_ROOT") {
        let root = PathBuf::from(root);
        let step: usize = std::env::var("AG1_CUTOVER_CRASH_STEP")
            .unwrap()
            .parse()
            .unwrap();
        let registry = Registry::at(&root.join("authority")).unwrap();
        let workspace = root.join("workspace");
        let source = root.join("source");
        let bundle: FrozenBundle =
            serde_json::from_slice(&fs::read(root.join("input.json")).unwrap()).unwrap();
        let config = fs::read(root.join("config.json")).unwrap();
        let mut owner = registry.open_typescript(&workspace, &source).unwrap();
        owner.begin_drain().unwrap();
        owner.quiesced().unwrap();
        if step == 0 {
            std::process::exit(91);
        }
        drop(owner);
        let mut lease = registry.resume_transfer(&workspace).unwrap();
        lease
            .freeze(Prepare {
                id: "cutover-1",
                source: &source,
                archive: &root.join("archive"),
                destination: &root.join("destination"),
                bundle: &bundle,
                config: &config,
            })
            .unwrap();
        if step == 1 {
            std::process::exit(91);
        }
        lease.import().unwrap();
        if step == 2 {
            std::process::exit(91);
        }
        lease.validate().unwrap();
        if step == 3 {
            std::process::exit(91);
        }
        lease.revoke().unwrap();
        if step == 4 {
            std::process::exit(91);
        }
        lease.commit_fixture().unwrap();
        std::process::exit(91);
    }
    for step in 0..6 {
        let f = Fixture::new();
        fs::write(
            f._dir.path().join("input.json"),
            serde_json::to_vec(&f.bundle).unwrap(),
        )
        .unwrap();
        fs::write(f._dir.path().join("config.json"), &f.config).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "abrupt_child_exit_at_each_transfer_stage_is_recoverable",
            ])
            .env("AG1_CUTOVER_CRASH_ROOT", f._dir.path())
            .env("AG1_CUTOVER_CRASH_STEP", step.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(91),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if step < 5 {
            let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
            if step == 0 {
                lease.freeze(f.prepare()).unwrap();
            }
            if step <= 1 {
                lease.import().unwrap();
            }
            if step <= 2 {
                lease.validate().unwrap();
            }
            if step <= 3 {
                lease.revoke().unwrap();
            }
            lease.commit_fixture().unwrap();
        }
        let native = f
            .registry
            .open_native(&f.workspace, &f.destination)
            .unwrap();
        native.admit().unwrap();
        assert!(f.registry.open_typescript(&f.workspace, &f.source).is_err());
        f.bundle.files.preflight(&f.source).unwrap();
    }
}

#[tokio::test]
async fn durable_desktop_startup_and_actual_cli_compete_for_the_same_workspace() {
    let mut f = Fixture::new();
    f.registry = Registry::for_user().unwrap();
    f.migrate();
    let desktop = ag1_desktop::native_client::NativeClient::from_environment().unwrap();
    assert!(desktop.selects(&f.workspace).unwrap());
    desktop
        .open(&f.workspace, &f._dir.path().join("desktop-private"))
        .await
        .unwrap();
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/ag1-cli");
    let run = |state: &Path| {
        std::process::Command::new(&cli)
            .arg("--workspace")
            .arg(&f.workspace)
            .arg("--state")
            .arg(state)
            .arg("--config")
            .arg(f.destination.join("host.json"))
            .arg("--export")
            .arg(f._dir.path().join("export.html"))
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    };
    let competing = run(&f._dir.path().join("different-cli-state"));
    assert!(!competing.status.success());
    assert!(String::from_utf8_lossy(&competing.stderr).contains("migrating"));
    desktop.shutdown().await.unwrap();
    let reopened = run(&f.destination);
    assert!(
        reopened.status.success(),
        "{}",
        String::from_utf8_lossy(&reopened.stderr)
    );
    assert!(
        fs::read_to_string(f._dir.path().join("export.html"))
            .unwrap()
            .contains("Retain this exact imported message")
    );
    assert!(!f.registry.inspect(&f.workspace).unwrap().unwrap().isolated);
}

#[tokio::test]
async fn measured_workflow_rejects_wrong_scope_changed_inputs_and_reused_evidence_without_replay() {
    use ag1_platform::cutover::workflow::{Request, rehearse, run};
    let mut f = Fixture::new();
    f.registry = Registry::for_user().unwrap();
    f.migrate();
    let evidence = f._dir.path().join("measured-workflow");
    let request = || Request {
        workspace: f.workspace.clone(),
        session: "migrated".into(),
        evidence: evidence.clone(),
        input_sha256: "0".repeat(64),
        read_path: "retained.txt".into(),
        read_sha256: digest(b"original worktree bytes\n"),
        steering_marker: "WORKFLOW_MARKER".into(),
        timeout_seconds: 2,
    };
    let before = f.registry.inspect(&f.workspace).unwrap();
    assert!(
        run(request()).await.is_err(),
        "production runner must reject fixture authority"
    );
    let mut wrong = request();
    wrong.session = "unselected".into();
    assert!(rehearse(wrong, &f.registry, None).await.is_err());
    let mut nested = request();
    nested.evidence = f.source.join("evidence");
    assert!(rehearse(nested, &f.registry, None).await.is_err());
    assert_eq!(f.registry.inspect(&f.workspace).unwrap(), before);
    assert!(!evidence.exists());
    fs::write(f.source.join("opaque.json"), b"tampered fixture source").unwrap();
    assert!(rehearse(request(), &f.registry, None).await.is_err());
    assert!(!evidence.exists());
    fs::write(
        f.source.join("opaque.json"),
        br#"{"unknown":false,"zero":0,"ordered":[3,1,2]}"#,
    )
    .unwrap();
    let mut changed = request();
    changed.read_sha256 = "f".repeat(64);
    assert!(rehearse(changed, &f.registry, None).await.is_err());
    let result = fs::read(evidence.join("result.json")).unwrap();
    let result_value: serde_json::Value = serde_json::from_slice(&result).unwrap();
    assert_eq!(result_value["status"], "command_failed");
    assert!(!evidence.join("prompt-admission").exists());
    let record = f.registry.inspect(&f.workspace).unwrap().unwrap();
    assert_eq!(record.stage, Stage::Active);
    assert!(record.quiescent);
    assert!(rehearse(request(), &f.registry, None).await.is_err());
    assert_eq!(f.registry.inspect(&f.workspace).unwrap(), Some(record));
    assert_eq!(fs::read(evidence.join("result.json")).unwrap(), result);
    assert!(!evidence.join("prompt-admission").exists());
    f.bundle.files.preflight(&f.source).unwrap();
}

#[tokio::test]
async fn imported_workspace_runs_prompt_tool_steering_compaction_and_resume() {
    use std::{
        io::{Read, Write},
        time::{Duration, Instant},
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server_stop = stop.clone();
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        let mut read_sent = false;
        for _ in 0..12 {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if server_stop.load(std::sync::atomic::Ordering::Acquire) {
                            return requests;
                        }
                        assert!(Instant::now() < deadline, "fake provider accept deadline");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 8192];
            let body = loop {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() < 4 * 1024 * 1024);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let length: usize = String::from_utf8_lossy(&bytes[..end])
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<serde_json::Value>(
                            &bytes[end + 4..end + 4 + length],
                        )
                        .unwrap();
                    }
                }
            };
            let summary = body.get("tools").is_none();
            let call_read = !summary && !read_sent;
            requests.push(body);
            if call_read {
                ready_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
                read_sent = true;
            }
            let delta = if call_read {
                json!({"tool_calls":[{"index":0,"id":"read-cutover","type":"function","function":{"name":"read","arguments":"{\"path\":\"retained.txt\"}"}}]})
            } else if !summary {
                json!({"content":"Fixture finished reading the retained file. STEERING_RETAINED"})
            } else {
                json!({"content":"## Goal\nPreserve the imported workspace.\n## Progress\nRead retained.txt successfully. STEERING_RETAINED\n## Next Steps\nResume without repeating the tool."})
            };
            let data = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"delta":delta,"finish_reason":if call_read {"tool_calls"} else {"stop"}}],"usage":{"prompt_tokens":3000,"completion_tokens":30}})
            );
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",data.len(),data).unwrap();
        }
        panic!("Fixture request limit exceeded")
    });
    let mut f = Fixture::new();
    let mut cfg = config();
    cfg.models[0]["baseUrl"] = origin.into();
    cfg.models[0]["contextWindow"] = 131072.into();
    f.config = serde_json::to_vec(&cfg).unwrap();
    f.bundle.config_sha256 = digest(&f.config);
    let mut journal=json!({"type":"session","version":3,"id":"migrated","cwd":f.workspace,"timestamp":"2026-09-07T00:00:00Z"}).to_string()+"\n";
    let mut parent = serde_json::Value::Null;
    for i in 0..35 {
        let id = format!("history-{i}");
        journal.push_str(&(json!({"type":"message","id":id,"parentId":parent,"timestamp":"2026-09-07T00:00:01Z","message":{"role":"user","content":format!("Historical note {i}: {}","retained history ".repeat(300)),"timestamp":i}}).to_string()+"\n"));
        parent = id.into();
    }
    journal.push_str(&(json!({"type":"message","id":"user-one","parentId":parent,"timestamp":"2026-09-07T00:00:02Z","message":{"role":"user","content":"Retain this exact imported message","timestamp":99}}).to_string()+"\n"));
    fs::write(f.source.join("session.jsonl"), journal.as_bytes()).unwrap();
    f.bundle.files.files[0].sha256 = digest(journal.as_bytes());
    f.bundle.files.files[0].bytes = journal.len() as u64;
    f.migrate();
    let audit = std::sync::Arc::new(std::sync::Mutex::new(custody_fixture::Audit::default()));
    let port = cfg.models[0]["baseUrl"]
        .as_str()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let bundle = std::sync::Arc::new(
        ag1_platform::llama::custody::RuntimeBundle::new(
            custody_fixture::FakeHost::new(audit.clone()),
            custody_fixture::plans(f._dir.path(), port, &["initial"], 131072),
            f.registry.runtime_lease().unwrap(),
            Duration::from_secs(2),
        )
        .unwrap(),
    );
    let evidence = f._dir.path().join("workflow-evidence");
    let release_evidence = evidence.clone();
    let release = std::thread::spawn(move || {
        ready_rx.recv_timeout(Duration::from_secs(60)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !release_evidence
            .join("steering-admission/result.json")
            .exists()
        {
            assert!(
                Instant::now() < deadline,
                "workflow steering receipt deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        release_tx.send(()).unwrap();
    });
    let report = ag1_platform::cutover::workflow::rehearse(
        ag1_platform::cutover::workflow::Request {
            workspace: f.workspace.clone(),
            session: "migrated".into(),
            evidence: evidence.clone(),
            input_sha256: "0".repeat(64),
            read_path: "retained.txt".into(),
            read_sha256: digest(b"original worktree bytes\n"),
            steering_marker: "STEERING_RETAINED".into(),
            timeout_seconds: 30,
        },
        &f.registry,
        Some(bundle.clone()),
    )
    .await
    .unwrap();
    release.join().unwrap();
    assert_eq!(report["fixture"], true);
    assert_eq!(report["status"], "passed");
    assert_eq!(report["authority"]["stage"], "draining");
    assert_eq!(report["renderer_acceptance"], "pending_actual_renderer");
    for name in [
        "prompt-admission",
        "steering-admission",
        "prompt-settlement",
        "compaction",
        "resume",
        "stale-client-rejection",
        "client-resnapshot",
        "drain",
    ] {
        let path = evidence.join(name);
        let start = fs::read(path.join("started.json")).unwrap();
        let finish: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("result.json")).unwrap()).unwrap();
        let observation: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("observation.json")).unwrap()).unwrap();
        assert_eq!(finish["started_sha256"], digest(&start));
        assert_eq!(finish["status"], "command_succeeded");
        assert_eq!(observation["fixture"], true);
        assert_eq!(observation["elapsed_ms"], finish["elapsed_ms"]);
        assert_eq!(observation["selection"]["session"], "migrated");
    }
    stop.store(true, std::sync::atomic::Ordering::Release);
    let requests = server.join().unwrap();
    assert_eq!(
        bundle
            .events()
            .unwrap()
            .iter()
            .filter(|e| e.action == "owned_ready")
            .count(),
        requests.len()
    );
    assert!(audit.lock().unwrap().active.is_none());
    assert_eq!(audit.lock().unwrap().maximum_active, 1);
    drop(bundle);
    assert!(
        requests
            .iter()
            .filter(|r| r.get("tools").is_some())
            .any(|r| r["messages"]
                .to_string()
                .contains("Retain this exact imported message"))
    );
    assert!(requests.iter().any(|r| {
        r["messages"]
            .to_string()
            .contains("original worktree bytes")
    }));
    assert!(
        requests
            .iter()
            .filter(|r| r.get("tools").is_some())
            .any(|r| r["messages"].to_string().contains("STEERING_RETAINED"))
    );
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    let ts = f._dir.path().join("workflow-ts-rollback");
    lease
        .freeze_rollback(
            &f._dir.path().join("workflow-native-archive"),
            &ts,
            "migrated",
            TS_MODEL,
        )
        .unwrap();
    lease.prepare_rollback().unwrap();
    lease.commit_rollback().unwrap();
    drop(lease);
    let mut worker = spawn_rollback_worker(&f, &ts);
    assert!(worker.view()["error"].is_null(), "{}", worker.view());
    let journal = fs::read_to_string(ts.join("sessions/migrated.jsonl")).unwrap();
    assert!(journal.contains("compaction"));
    assert!(journal.contains("STEERING_RETAINED"));
    assert!(worker.view().to_string().contains("STEERING_RETAINED"));
    worker.stop();
    f.bundle.files.preflight(&f.source).unwrap();
}

#[tokio::test]
async fn bound_configuration_applies_native_settings_and_preserves_updated_vault_on_ts_rollback() {
    let captures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/llama/launch-plans.json"
    ))
    .unwrap();
    let mut f = Fixture::new();
    let mut request = runtime_import_request(&f, &captures["captures"][0]);
    let raw = br#"{ "defaultModel":"initial", "defaultProvider":"fixture", "defaultThinkingLevel":"medium", "compaction":{"enabled":false,"reserveTokens":0} }"#;
    let input = request.additional_inputs.get_mut("settings").unwrap();
    fs::write(&input.path, raw).unwrap();
    input.sha256 = digest(raw);
    let capture = ag1_platform::cutover::configuration::import_configuration(
        &f._dir.path().join("capture.json"),
        &request,
    )
    .unwrap();
    let mut cfg = config();
    capture.bind_host(&f.destination, &mut cfg).unwrap();
    assert!(!f.destination.exists());
    f.config = serde_json::to_vec_pretty(&cfg).unwrap();
    f.bundle.config_sha256 = digest(&f.config);
    f.bundle.runtime_configuration = Some(capture);
    f.migrate();
    let host = Host::open_with_registry(&f.workspace, &f.destination, cfg.clone(), &f.registry)
        .await
        .unwrap();
    assert!(
        host.registry
            .selected()
            .unwrap()
            .services
            .runtime()
            .configuration()
            .unwrap()
            .thinking
            == "medium"
    );
    assert_eq!(
        fs::read(f.destination.join("home/.ag1/agent/settings.json")).unwrap(),
        raw
    );
    host.shutdown().await.unwrap();
    drop(host);
    let store = Store::open(&f.destination.join("credential-store")).unwrap();
    let old = store
        .metadata("provider-credential", "fixture")
        .unwrap()
        .unwrap();
    store.save_metadata("provider-credential", "fixture", Some(&digest(&old)), br#"{"revision":1,"credential":{"type":"api_key","key":"updated-native-only-fixture"}}"#).unwrap();
    drop(store);
    let changed = br#"{"defaultThinkingLevel":"high","compaction":{"enabled":true}}"#;
    fs::write(f.destination.join("home/.ag1/agent/settings.json"), changed).unwrap();
    let host = Host::open_with_registry(&f.workspace, &f.destination, cfg, &f.registry)
        .await
        .unwrap();
    assert!(
        host.registry
            .selected()
            .unwrap()
            .services
            .runtime()
            .configuration()
            .unwrap()
            .thinking
            == "high"
    );
    host.drain_for_cutover().await.unwrap();
    drop(host);
    let mut lease = f.registry.resume_transfer(&f.workspace).unwrap();
    let archive = f._dir.path().join("native-recovery");
    let destination = f._dir.path().join("new-ts");
    lease
        .freeze_rollback(&archive, &destination, "migrated", TS_MODEL)
        .unwrap();
    lease.prepare_rollback().unwrap();
    let auth: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("agent/auth.json")).unwrap()).unwrap();
    assert_eq!(auth["fixture"]["key"], "updated-native-only-fixture");
    assert_eq!(
        fs::read(destination.join("agent/settings.json")).unwrap(),
        changed
    );
    for input in request.additional_inputs.values() {
        assert_eq!(digest(&fs::read(&input.path).unwrap()), input.sha256);
    }
    assert_eq!(fs::read(f.destination.join("host.json")).unwrap(), f.config);
    lease.commit_rollback().unwrap();
}

#[test]
fn ts_rollback_hides_archived_sessions_and_removes_stale_original_journal_duplicates() {
    let mut f = Fixture::new();
    fs::create_dir(f.source.join("sessions")).unwrap();
    fs::rename(
        f.source.join("session.jsonl"),
        f.source.join("sessions/old-timestamp-name.jsonl"),
    )
    .unwrap();
    f.bundle.files.files[0].path = "sessions/old-timestamp-name.jsonl".into();
    f.migrate();
    let store = Store::open(&f.destination.join("store")).unwrap();
    let header = serde_json::to_vec(&json!({"type":"session","version":3,"id":"archived","cwd":f.workspace,"timestamp":"2026-09-07T00:00:00Z"})).unwrap();
    store
        .bootstrap_session(ag1_storage::SessionBootstrap {
            command: "archived-bootstrap",
            session: "archived",
            origin: "archived",
            cursor_key: &[11; 32],
            seed: ag1_storage::SessionSeed::Fresh {
                header: &header,
                records: &[],
            },
        })
        .unwrap();
    store.archive_session("archived").unwrap();
    drop(store);
    let mut lease = begin_native_rollback(&f);
    let archive = f._dir.path().join("current-native");
    let ts = f._dir.path().join("current-ts");
    lease
        .freeze_rollback(&archive, &ts, "migrated", TS_MODEL)
        .unwrap();
    lease.prepare_rollback().unwrap();
    assert!(!ts.join("sessions/archived.jsonl").exists());
    assert!(!ts.join("sessions/old-timestamp-name.jsonl").exists());
    assert!(
        ts.join("native-recovery/archived-sessions/archived.jsonl")
            .is_file()
    );
    assert!(
        archive
            .join("original/sessions/old-timestamp-name.jsonl")
            .is_file()
    );
    lease.commit_rollback().unwrap();
    drop(lease);
    let mut worker = spawn_rollback_worker(&f, &ts);
    let reply = ts_command(
        &worker,
        "list-after-rollback",
        ag1_protocol::Command::Session {
            action: "list".into(),
            args: json!({}),
        },
    );
    let value = serde_json::to_value(worker.command(reply).unwrap()).unwrap();
    assert_eq!(value["data"].as_array().unwrap().len(), 1);
    assert!(
        value["data"][0]
            .as_str()
            .unwrap()
            .ends_with("/migrated.jsonl")
    );
    worker.stop();
    f.bundle.files.preflight(&f.source).unwrap();
}

#[test]
fn administrative_cli_publishes_rollback_and_explicit_native_startup_obeys_ts_authority() {
    if let Some(workspace) = std::env::var_os("P13_EXPLICIT_ROLLBACK_WORKSPACE") {
        let desktop = ag1_desktop::native_client::NativeClient::from_environment().unwrap();
        assert!(desktop.enabled());
        assert!(!desktop.selects(Path::new(&workspace)).unwrap());
        return;
    }
    let mut f = Fixture::new();
    f.registry = Registry::for_user().unwrap();
    f.migrate();
    drop(begin_native_rollback(&f));
    let cli = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/ag1-cutover");
    let archive = f._dir.path().join("cli-rollback-archive");
    let ts = f._dir.path().join("cli-ts");
    let run = |action: &str, extra: &[&std::ffi::OsStr]| {
        let output = std::process::Command::new(&cli)
            .arg(action)
            .arg(&f.workspace)
            .args(extra)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    run(
        "freeze-ts-rollback",
        &[
            archive.as_os_str(),
            ts.as_os_str(),
            "migrated".as_ref(),
            TS_MODEL.as_ref(),
        ],
    );
    assert_eq!(
        run("prepare-ts-rollback", &[])["stage"],
        "rollback_prepared"
    );
    assert_eq!(
        run("prepare-ts-rollback", &[])["stage"],
        "rollback_prepared"
    );
    assert_eq!(run("commit-ts-rollback", &[])["owner"], "type_script");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "administrative_cli_publishes_rollback_and_explicit_native_startup_obeys_ts_authority",
            "--nocapture",
        ])
        .env("P13_EXPLICIT_ROLLBACK_WORKSPACE", &f.workspace)
        .env("AG1_NATIVE_CONFIG", f.destination.join("host.json"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        f.registry
            .open_native(&f.workspace, &f.destination)
            .is_err()
    );
    let mut worker = spawn_rollback_worker(&f, &ts);
    assert!(worker.view()["error"].is_null());
    worker.stop();
    f.bundle.files.preflight(&f.source).unwrap();
}

#[test]
fn unknown_native_execution_metadata_blocks_downgrade_with_lossless_archive_retained() {
    for kind in [
        "future-runtime-feature",
        "subagent_checkpoint",
        "subagent_run",
        "background-jobs",
    ] {
        let f = Fixture::new();
        f.migrate();
        let store = Store::open(&f.destination.join("store")).unwrap();
        let raw = br#"{"state":"intent","unknown":false,"pending":[3,1,2]}"#;
        store
            .save_metadata(kind, "pending-native-state", None, raw)
            .unwrap();
        drop(store);
        let mut lease = begin_native_rollback(&f);
        let archive = f._dir.path().join("unsupported-native-archive");
        let ts = f._dir.path().join("unsupported-ts");
        lease
            .freeze_rollback(&archive, &ts, "migrated", TS_MODEL)
            .unwrap();
        assert!(lease.prepare_rollback().is_err());
        assert!(!ts.exists());
        assert!(archive.join("recovery.json").is_file());
        let restored = f._dir.path().join("lossless-native");
        Store::restore_export(&archive, &restored).unwrap();
        let store = Store::open(&restored).unwrap();
        assert_eq!(
            store
                .metadata(kind, "pending-native-state")
                .unwrap()
                .unwrap(),
            raw
        );
        lease.cancel_rollback().unwrap();
        assert_eq!(lease.record().unwrap().owner, Owner::Native);
    }
}

#[tokio::test]
async fn runtime_binding_is_archived_and_fixture_config_cannot_launch_the_pinned_backend() {
    let captures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/llama/launch-plans.json"
    ))
    .unwrap();
    let mut f = Fixture::new();
    let request = runtime_import_request(&f, &captures["captures"][0]);
    let capture = ag1_platform::cutover::configuration::import_configuration(
        &f._dir.path().join("capture.json"),
        &request,
    )
    .unwrap();
    let helper = request.models[0].materials[0].clone();
    let mut cfg = config();
    capture
        .bind_runtime(&f.destination, helper, &mut cfg)
        .unwrap();
    f.config = serde_json::to_vec(&cfg).unwrap();
    f.bundle.config_sha256 = digest(&f.config);
    f.bundle.runtime_configuration = Some(capture.clone());
    f.migrate();
    let runtime = cfg.runtime.as_ref().unwrap();
    assert_eq!(runtime.capture.verify().unwrap().len(), 1);
    assert_eq!(
        fs::read(&runtime.capture.path).unwrap(),
        fs::read(&capture.path).unwrap()
    );
    assert!(
        Host::open_with_registry(&f.workspace, &f.destination, cfg, &f.registry)
            .await
            .is_err()
    );
    let report = f.registry.inspect(&f.workspace).unwrap().unwrap();
    assert_eq!(report.owner, Owner::Native);
    f.bundle.files.preflight(&f.source).unwrap();
}
