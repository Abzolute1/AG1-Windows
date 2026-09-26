#![cfg(target_os = "linux")]
mod readonly_shell;
use ag1_core::{
    DomainError,
    tools::{
        approval::{AuthorizedCall, Authorizer, Preflight},
        policy::Policy,
        registry::{Registry, Scope, ValidatedCall},
    },
};
use ag1_platform::{
    file_tools::{FileService, PreparedFile},
    workspace::{Workspace, WorkspaceService},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn setup() -> (
    tempfile::TempDir,
    Arc<Workspace>,
    FileService,
    Registry,
    Scope,
) {
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let files = FileService::new(ws.clone());
    let mut registry = Registry::builtins().unwrap();
    let names = ["write", "edit", "append", "editPreview", "editApply"].map(str::to_string);
    registry
        .activate(&names, &names.iter().cloned().collect())
        .unwrap();
    let scope = Scope {
        workspace: ws.id().as_str().into(),
        session: "fixture".into(),
        epoch: 1,
        revision: 1,
    };
    (dir, ws, files, registry, scope)
}
fn call(registry: &Registry, id: &str, name: &str, args: Value) -> ValidatedCall {
    registry.prepare(id, id, name, &args, true).unwrap()
}
// Keep captured SDK payloads intact while asserting the native revision fields
// from the fixture's original source bytes, never from the observed result.
fn native_file_provenance(expected: &mut Value, tool: &str, path: &str, original: &[u8]) {
    if !expected.is_object() { return; }
    let hash = ag1_storage::digest(original);
    if tool == "read" {
        expected["details"]["sourceReference"] = json!(format!("file:{path}@sha256:{hash}"));
        expected["details"]["sourceSha256"] = json!(hash);
        expected["details"]["sourceKind"] = json!("file_revision");
    } else if tool == "edit" && expected["details"]["diff"].is_string() {
        expected["details"]["sourceSha256"] = json!(hash);
        expected["details"]["mode"] = json!("apply");
    }
}
fn authorize(
    call: ValidatedCall,
    p: &PreparedFile,
    registry: &Registry,
    scope: &Scope,
    cwd: &str,
) -> AuthorizedCall {
    let policy: Policy = serde_json::from_value(json!({"mode":"interactive","cwd":cwd})).unwrap();
    let mut auth = Authorizer::new("fixture-incarnation").unwrap();
    match auth
        .preflight(
            call,
            ag1_core::tools::approval::PreflightContext {
                scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: p.target_revision(),
                now: 10,
                ttl: 10000,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    {
        Preflight::Ready(ticket) => ticket,
        Preflight::Approval(request) => auth
            .answer(
                &request.binding,
                true,
                registry,
                ag1_core::tools::approval::ExecutionFence {
                    scope,
                    policy_revision: 1,
                    target_revision: p.target_revision(),
                    now: 11,
                },
            )
            .unwrap()
            .unwrap(),
        Preflight::Rejected(reason) => panic!("{reason}"),
    }
}
#[test]
fn native_files_match_39_reference_cases_with_explicit_native_safety_corrections() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/file-oracle.json")).unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 39);
    let mut corrected = 0;
    let mut failure_corrected = 0;
    for (index, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let (dir, _ws, files, registry, scope) = setup();
        let input = &row["input"];
        let path = dir.path().join(input["args"]["path"].as_str().unwrap());
        if let Some(old) = input["original"].as_str() {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, old).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
        }
        let call = call(
            &registry,
            "case",
            input["tool"].as_str().unwrap(),
            input["args"].clone(),
        );
        let prepared = files.prepare(&call, &scope);
        if let Some(error) = row["output"]["error"].as_str() {
            assert!(
                matches!(prepared,Err(DomainError::InvalidInput(ref s)) if s==error),
                "{index}"
            );
        } else {
            let prepared = prepared.unwrap_or_else(|e| panic!("{index}: {e:?}"));
            // Preparing a write creates no directories and changes no existing bytes.
            assert_eq!(
                fs::read(&path).ok(),
                input["original"].as_str().map(|s| s.as_bytes().to_vec()),
                "prepare {index}"
            );
            let ticket = authorize(
                call,
                &prepared,
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            let output = files
                .execute(
                    prepared,
                    ticket,
                    ag1_platform::file_tools::FileExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &AtomicBool::new(false),
                        deadline: Instant::now() + Duration::from_secs(5),
                    },
                )
                .unwrap_or_else(|e| panic!("{index}: {e:?}"));
            let normalized: Value = serde_json::from_str(
                &output
                    .result
                    .to_string()
                    .replace(dir.path().to_str().unwrap(), "/fixture"),
            )
            .unwrap();
            if index == 32 || index == 33 {
                assert_ne!(normalized, row["output"]["result"]);
                corrected += 1;
                let next = input["original"].as_str().unwrap().replacen(
                    input["args"]["oldText"].as_str().unwrap(),
                    input["args"]["newText"].as_str().unwrap(),
                    1,
                );
                assert_preview_applies(
                    input["original"].as_str().unwrap(),
                    &next,
                    &normalized["details"]["hunks"],
                );
            } else if matches!(index, 25..=27) {
                // Legacy first-match apply could silently edit the wrong site,
                // or report partial applications although it wrote no bytes.
                // Keep the oracle intact and assert the native correction.
                assert_eq!(normalized["isError"], true, "case {index}");
                assert_eq!(normalized["details"]["applied"], 0);
                assert_eq!(normalized["details"]["verified"], false);
                assert!(
                    normalized["details"]["error"]
                        .as_str()
                        .unwrap()
                        .contains("No changes from this call were written")
                );
                assert!(output.published.is_none());
                failure_corrected += 1;
            } else if matches!(index, 15 | 31 | 34..=36) {
                // Recorded failure payloads lacked the error discriminator.
                let mut expected = row["output"]["result"].clone();
                expected["isError"] = json!(true);
                assert_eq!(normalized, expected, "case {index}");
                assert!(output.published.is_none());
                failure_corrected += 1;
            } else {
                let mut expected = row["output"]["result"].clone();
                native_file_provenance(&mut expected, input["tool"].as_str().unwrap(), "file.txt", input["original"].as_str().unwrap_or("").as_bytes());
                assert_eq!(normalized, expected, "case {index}");
            }
        }
        let actual=fs::read(&path).ok().map(|bytes|json!({"bytes":STANDARD.encode(bytes),"mode":fs::metadata(&path).unwrap().permissions().mode()&0o777}));
        if matches!(index, 25 | 26) {
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                input["original"].as_str().unwrap()
            );
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o750
            );
        } else {
            assert_eq!(json!(actual), row["final"], "final {index}");
        }
    }
    assert_eq!(corrected, 2);
    assert_eq!(failure_corrected, 8);
}
fn assert_preview_applies(old: &str, new: &str, hunks: &Value) {
    let mut lines: Vec<String> = old.split('\n').map(str::to_string).collect();
    for hunk in hunks.as_array().unwrap().iter().rev() {
        let start = hunk["fromLine"].as_u64().unwrap() as usize - 1;
        let diff = hunk["diff"].as_str().unwrap();
        let mut removed = vec![];
        let mut added = vec![];
        for line in diff.split('\n').skip(1) {
            let (kind, text) = line.split_at(1);
            if kind != "+" {
                removed.push(text.to_string());
            }
            if kind != "-" {
                added.push(text.to_string());
            }
        }
        assert_eq!(&lines[start..start + removed.len()], removed);
        lines.splice(start..start + removed.len(), added);
    }
    assert_eq!(lines.join("\n"), new);
}
#[test]
fn prepared_writes_cannot_cross_scopes_bypass_approval_or_overwrite_competing_changes() {
    let (dir, ws, files, registry, scope) = setup();
    fs::write(dir.path().join("file"), "original").unwrap();
    let a = call(
        &registry,
        "a",
        "write",
        json!({"path":"file","content":"first"}),
    );
    let b = call(
        &registry,
        "b",
        "append",
        json!({"path":"file","content":"second"}),
    );
    let pa = files.prepare(&a, &scope).unwrap();
    let pb = files.prepare(&b, &scope).unwrap();
    let ta = authorize(a, &pa, &registry, &scope, dir.path().to_str().unwrap());
    let tb = authorize(b, &pb, &registry, &scope, dir.path().to_str().unwrap());
    files
        .execute(
            pa,
            ta,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(1),
            },
        )
        .unwrap();
    assert!(matches!(
        files.execute(
            pb,
            tb,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(1)
            }
        ),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("file")).unwrap(),
        "first"
    );
    let c = call(
        &registry,
        "c",
        "write",
        json!({"path":"nested/new","content":"new"}),
    );
    let p = files.prepare(&c, &scope).unwrap();
    let t = authorize(c, &p, &registry, &scope, dir.path().to_str().unwrap());
    assert!(!dir.path().join("nested").exists());
    let mut stale = scope.clone();
    stale.epoch += 1;
    assert!(matches!(
        files.execute(
            p,
            t,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &stale,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(1)
            }
        ),
        Err(DomainError::PermissionDenied)
    ));
    assert!(!dir.path().join("nested").exists());
    let c = call(
        &registry,
        "external",
        "write",
        json!({"path":"file","content":"overwrite"}),
    );
    let p = files.prepare(&c, &scope).unwrap();
    let t = authorize(c, &p, &registry, &scope, dir.path().to_str().unwrap());
    fs::write(dir.path().join("file"), "external change").unwrap();
    assert!(matches!(
        files.execute(
            p,
            t,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(1)
            }
        ),
        Err(DomainError::Conflict)
    ));
    assert_eq!(ws.snapshot("file", 1024).unwrap().bytes, b"external change");
}
#[test]
fn stale_preview_and_prepublication_cancellation_leave_all_original_bytes_intact() {
    let (dir, ws, files, registry, scope) = setup();
    fs::write(dir.path().join("file"), "one\ntwo\n").unwrap();
    let c = call(
        &registry,
        "preview",
        "editPreview",
        json!({"path":"file","oldText":"one","newText":"ONE"}),
    );
    let p = files.prepare(&c, &scope).unwrap();
    let t = authorize(c, &p, &registry, &scope, dir.path().to_str().unwrap());
    files
        .execute(
            p,
            t,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(1),
            },
        )
        .unwrap();
    fs::write(dir.path().join("file"), "one\nexternal\n").unwrap();
    let c = call(
        &registry,
        "apply",
        "editApply",
        json!({"path":"file","edits":[{"oldText":"one","newText":"ONE"}]}),
    );
    assert!(matches!(
        files.prepare(&c, &scope),
        Err(DomainError::Conflict)
    ));
    let before = ws.snapshot("file", 1024).unwrap();
    for cancel_at in 0..3 {
        let count = AtomicUsize::new(0);
        assert_eq!(
            ws.replace_checked("file", &before.revision, b"destroy", || {
                if count.fetch_add(1, Ordering::SeqCst) == cancel_at {
                    Err(DomainError::Cancelled)
                } else {
                    Ok(())
                }
            }),
            Err(DomainError::Cancelled)
        );
        assert_eq!(ws.snapshot("file", 1024).unwrap().bytes, before.bytes);
    }
    for cancel_at in 0..3 {
        let count = AtomicUsize::new(0);
        assert!(matches!(
            ws.create_checked("new", b"partial", false, || {
                if count.fetch_add(1, Ordering::SeqCst) == cancel_at {
                    Err(DomainError::Cancelled)
                } else {
                    Ok(())
                }
            }),
            Err(DomainError::Cancelled)
        ));
        assert!(!dir.path().join("new").exists());
    }
    assert!(fs::read_dir(dir.path()).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".ag1-")
    }));
    assert!(ag1_core::tools::preview::preview("file", Some("one"), "", "bad").is_err());
}
#[test]
fn symlinks_competing_creators_setid_and_parent_replacement_fail_closed() {
    let (dir, ws, files, registry, scope) = setup();
    fs::write(dir.path().join("target"), "local original").unwrap();
    symlink("target", dir.path().join("alias")).unwrap();
    let alias = call(
        &registry,
        "internal-alias",
        "write",
        json!({"path":"alias","content":"alias overwrite"}),
    );
    assert!(matches!(
        files.prepare_queued(
            &alias,
            &scope,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(1)
        ),
        Err(DomainError::PermissionDenied)
    ));
    assert_eq!(
        fs::read_to_string(dir.path().join("target")).unwrap(),
        "local original"
    );
    assert!(
        fs::symlink_metadata(dir.path().join("alias"))
            .unwrap()
            .is_symlink()
    );
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "foreign").unwrap();
    symlink(outside.path(), dir.path().join("link")).unwrap();
    for path in [
        "link/file".to_string(),
        outside.path().join("file").display().to_string(),
        "../escape".into(),
    ] {
        let c = call(
            &registry,
            "path",
            "write",
            json!({"path":path,"content":"bad"}),
        );
        assert!(matches!(
            files.prepare(&c, &scope),
            Err(DomainError::PermissionDenied)
        ));
    }
    ws.create_checked("new", b"first", false, || Ok(()))
        .unwrap();
    assert!(matches!(
        ws.create_checked("new", b"second", false, || Ok(())),
        Err(DomainError::Conflict)
    ));
    fs::set_permissions(dir.path().join("new"), fs::Permissions::from_mode(0o4750)).unwrap();
    let old = ws.snapshot("new", 1024).unwrap();
    assert_eq!(
        ws.replace("new", &old.revision, b"bad"),
        Err(DomainError::PermissionDenied)
    );
    assert_eq!(fs::read(dir.path().join("new")).unwrap(), b"first");
    fs::create_dir(dir.path().join("parent")).unwrap();
    let count = AtomicUsize::new(0);
    let result = ws.create_checked("parent/target", b"bad", false, || {
        if count.fetch_add(1, Ordering::SeqCst) == 3 {
            fs::rename(dir.path().join("parent"), dir.path().join("moved")).unwrap();
            fs::create_dir(dir.path().join("parent")).unwrap();
        }
        Ok(())
    });
    assert!(matches!(result, Err(DomainError::Conflict)));
    assert!(!dir.path().join("parent/target").exists());
    assert!(!dir.path().join("moved/target").exists());
    assert_eq!(fs::read(outside.path().join("file")).unwrap(), b"foreign");
    // A directory target is refused without leaving a transaction temp file.
    fs::create_dir(dir.path().join("directory-target")).unwrap();
    let directory = call(
        &registry,
        "directory",
        "write",
        json!({"path":"directory-target","content":"bad"}),
    );
    assert!(files.prepare(&directory, &scope).is_err());
    assert!(dir.path().join("directory-target").is_dir());
    for parent in [
        dir.path().to_path_buf(),
        dir.path().join("moved"),
        dir.path().join("parent"),
    ] {
        assert!(fs::read_dir(parent).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".ag1-")
        }));
    }
}

#[test]
fn native_read_windows_match_sdk_bytes_progress_and_inspection_references() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/read-oracle.json")).unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 15);
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let (dir, _ws, files, mut registry, scope) = setup();
        registry
            .activate(&["read".into()], &["read".into()].into_iter().collect())
            .unwrap();
        fs::write(
            dir.path().join("file.txt"),
            row["input"]["text"].as_str().unwrap(),
        )
        .unwrap();
        let call = call(
            &registry,
            &format!("read-{i}"),
            "read",
            row["input"]["args"].clone(),
        );
        let prepared = files.prepare(&call, &scope);
        if let Some(error) = row["output"]["error"].as_str() {
            assert!(
                matches!(prepared,Err(DomainError::InvalidInput(ref s)) if s==error),
                "{i}"
            );
            continue;
        }
        let p = prepared.unwrap();
        let t = authorize(call, &p, &registry, &scope, dir.path().to_str().unwrap());
        let result = files
            .execute(
                p,
                t,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(1),
                },
            )
            .unwrap();
        let mut expected = row["output"]["result"].clone();
        native_file_provenance(&mut expected, "read", "file.txt", row["input"]["text"].as_str().unwrap().as_bytes());
        assert_eq!(result.result, expected, "read {i}");
        assert!(result.published.is_none());
    }
}

static IMAGE_FIXTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn native_image_attachments_preserve_originals_and_match_reference_resize_dimensions_and_pixels() {
    let _serial = IMAGE_FIXTURE.lock().unwrap();
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/image-oracle.json")).unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 14);
    for row in oracle["rows"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/tools/images")
            .join(row["name"].as_str().unwrap());
        let bytes = fs::read(path).unwrap();
        let output =
            ag1_platform::image_tools::read_with_resize(&bytes, row["resize"].as_bool().unwrap())
                .unwrap();
        if row["name"] == "invalid.png" {
            assert!(output.is_none());
            assert!(
                row["output"]["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .starts_with("�PNG")
            );
            continue;
        }
        let result = output.unwrap();
        let expected = &row["output"]["result"];
        assert_eq!(
            result["content"][0], expected["content"][0],
            "{} {}",
            row["name"], row["resize"]
        );
        if matches!(row["name"].as_str().unwrap(), "wide.png" | "tall.png") && row["resize"] == true
        {
            let native = STANDARD
                .decode(result["content"][1]["data"].as_str().unwrap())
                .unwrap();
            let sdk = STANDARD
                .decode(expected["content"][1]["data"].as_str().unwrap())
                .unwrap();
            let native = image::load_from_memory(&native).unwrap().into_rgba8();
            let sdk = image::load_from_memory(&sdk).unwrap().into_rgba8();
            assert_eq!(native.dimensions(), sdk.dimensions());
            let difference: u64 = native
                .as_raw()
                .iter()
                .zip(sdk.as_raw())
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            assert!(
                difference <= native.as_raw().len() as u64,
                "resized channel MAE exceeds 1: {}",
                difference as f64 / native.as_raw().len() as f64
            );
            assert!(result["content"][1]["data"].as_str().unwrap().len() < 9 * 1024 * 1024 / 2);
        } else {
            assert_eq!(result, *expected);
        }
    }
    assert!(
        ag1_platform::image_tools::read(b"ordinary text")
            .unwrap()
            .is_none()
    );
    // A crafted huge dimension header must not allocate its claimed pixel count.
    let mut bomb = fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/tools/images/small.png"),
    )
    .unwrap();
    for (width, height) in [(100000u32, 100000u32), (5000u32, 4000u32)] {
        bomb[16..20].copy_from_slice(&width.to_be_bytes());
        bomb[20..24].copy_from_slice(&height.to_be_bytes());
        // Valid IHDR CRC ensures the dimensional limit, rather than a checksum error,
        // prevents decoding/allocation of the advertised image.
        let mut crc = !0u32;
        for byte in &bomb[12..29] {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
            }
        }
        bomb[29..33].copy_from_slice(&(!crc).to_be_bytes());
        assert!(
            ag1_platform::image_tools::read(&bomb).unwrap().unwrap()["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Image omitted")
        );
    }
}

#[test]
fn real_file_execution_uses_durable_intent_and_replays_without_a_second_write() {
    use ag1_core::{
        Clock, SessionId,
        context::features::Features,
        tools::{
            approval::ExecutionFence,
            executor::{self, AdapterOutcome, Effect, Execution, NoHooks},
            result::ResultBudget,
        },
    };
    use ag1_storage::{Store, context::ContextStore};
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            12
        }
    }
    let (dir, ws, files, registry, scope) = setup();
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[3; 32])
        .unwrap();
    fs::write(dir.path().join("file"), "before").unwrap();
    let make_artifacts = || {
        ContextStore::new(
            &store,
            SessionId::new("fixture").unwrap(),
            "fixture",
            "durable-write",
            timestamp,
            Features::default(),
        )
        .unwrap()
    };
    let call = call(
        &registry,
        "durable-write",
        "write",
        json!({"path":"file","content":"after"}),
    );
    let p = files.prepare(&call, &scope).unwrap();
    let stamp = p.target_revision().to_owned();
    let t = authorize(
        call.clone(),
        &p,
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    let result = executor::execute(
        &store,
        &mut make_artifacts(),
        t,
        Execution {
            registry: &registry,
            fence: ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: &stamp,
                now: 12,
            },
            budget: ResultBudget::default(),
            clock: &FixedClock,
        },
        &mut NoHooks,
        |ticket| {
            let record =
                executor::recover(&store, &scope.workspace, &scope.session, "durable-write")?
                    .unwrap();
            assert_eq!(record.status, executor::Status::Intent);
            assert_eq!(fs::read(dir.path().join("file")).unwrap(), b"before");
            let out = files.execute(
                p,
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(1),
                },
            )?;
            Ok(AdapterOutcome {
                result: out.result,
                effect: if out.published.is_some() {
                    Effect::Committed
                } else {
                    Effect::None
                },
                artifact_page: false,
            })
        },
        |_| {},
    )
    .unwrap();
    assert_eq!(result.record.effect, Effect::Committed);
    let first_revision = ws.snapshot("file", 1024).unwrap().revision;
    // Recovery replays the previous result using its original target revision. It does not prepare/apply again.
    let policy: Policy =
        serde_json::from_value(json!({"mode":"disabled","cwd":dir.path()})).unwrap();
    let Preflight::Ready(ticket) = Authorizer::new("replay")
        .unwrap()
        .preflight(
            call,
            ag1_core::tools::approval::PreflightContext {
                scope: &scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: &stamp,
                now: 12,
                ttl: 100,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    else {
        panic!()
    };
    let replay = executor::execute(
        &store,
        &mut make_artifacts(),
        ticket,
        Execution {
            registry: &registry,
            fence: ExecutionFence {
                scope: &scope,
                policy_revision: 1,
                target_revision: &stamp,
                now: 12,
            },
            budget: ResultBudget::default(),
            clock: &FixedClock,
        },
        &mut NoHooks,
        |_| panic!("duplicate real file write"),
        |_| panic!("duplicate event"),
    )
    .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.result, result.result);
    assert_eq!(ws.snapshot("file", 1024).unwrap().revision, first_revision);
}

#[test]
fn native_directory_listings_match_locale_order_limits_and_dotfile_behavior() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/list-oracle.json")).unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 7);
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let (dir, _ws, files, mut registry, scope) = setup();
        registry
            .activate(&["ls".into()], &["ls".into()].into_iter().collect())
            .unwrap();
        for entry in row["input"]["entries"].as_array().unwrap() {
            let entry = entry.as_str().unwrap();
            let path = dir.path().join(entry);
            if entry.ends_with('/') {
                fs::create_dir_all(path).unwrap();
            } else {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "fixture").unwrap();
            }
        }
        let call = call(&registry, "list", "ls", row["input"]["args"].clone());
        let policy: Policy =
            serde_json::from_value(json!({"mode":"interactive","cwd":dir.path()})).unwrap();
        let target = files.list_target_revision();
        let preflight = Authorizer::new("list")
            .unwrap()
            .preflight(
                call,
                ag1_core::tools::approval::PreflightContext {
                    scope: &scope,
                    policy: &policy,
                    policy_revision: 1,
                    target_revision: &target,
                    now: 10,
                    ttl: 100,
                    untrusted_extensions: false,
                },
            )
            .unwrap();
        let Preflight::Ready(ticket) = preflight else {
            panic!("contained ls should run without an approval prompt")
        };
        let result = files
            .execute_list(
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(1),
                },
            )
            .unwrap();
        assert_eq!(result.result, row["output"], "list {i}");
        assert!(result.published.is_none());
    }
}

#[test]
fn owned_search_process_cwd_remains_the_pinned_directory_when_its_path_is_replaced() {
    use ag1_platform::process::{ProcessSpec, ProcessSupervisor};
    let (dir, ws, _files, _registry, _scope) = setup();
    let spools = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("search")).unwrap();
    let pinned = ws.pin_directory("search").unwrap();
    fs::rename(dir.path().join("search"), dir.path().join("original")).unwrap();
    symlink(outside.path(), dir.path().join("search")).unwrap();
    assert!(pinned.verify_identity().is_err());
    let supervisor =
        ProcessSupervisor::new(std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")))
            .unwrap();
    let mut child = supervisor
        .spawn(
            ProcessSpec {
                resident: false,
                executable: env!("CARGO_BIN_EXE_ag1-platform-fixture").into(),
                argv: vec!["echo".into()],
                cwd: pinned.process_cwd(),
                env: Default::default(),
                timeout_ms: 1000,
                grace_ms: 100,
            },
            &spools.path().join("cwd.log"),
        )
        .unwrap();
    assert!(child.wait(Duration::from_secs(2)).unwrap().success());
    let output = String::from_utf8(child.tail(1024).unwrap().bytes).unwrap();
    assert!(output.contains(&format!("cwd={}\n", dir.path().join("original").display())));
    assert!(!output.contains(outside.path().to_str().unwrap()));
}

fn search_ticket(
    call: ValidatedCall,
    target: &str,
    registry: &Registry,
    scope: &Scope,
    cwd: &str,
) -> AuthorizedCall {
    let policy: Policy = serde_json::from_value(json!({"mode":"interactive","cwd":cwd})).unwrap();
    let mut auth = Authorizer::new("search-fixture-incarnation").unwrap();
    match auth
        .preflight(
            call,
            ag1_core::tools::approval::PreflightContext {
                scope,
                policy: &policy,
                policy_revision: 1,
                target_revision: target,
                now: 10,
                ttl: 10000,
                untrusted_extensions: false,
            },
        )
        .unwrap()
    {
        Preflight::Ready(ticket) => ticket,
        Preflight::Approval(request) => auth
            .answer(
                &request.binding,
                true,
                registry,
                ag1_core::tools::approval::ExecutionFence {
                    scope,
                    policy_revision: 1,
                    target_revision: target,
                    now: 11,
                },
            )
            .unwrap()
            .unwrap(),
        Preflight::Rejected(reason) => panic!("{reason}"),
    }
}
#[test]
fn native_search_matches_26_reference_cases_with_explicit_lone_cr_correction() {
    use ag1_platform::{process::ProcessSupervisor, search_tools::SearchService};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/search-oracle.json"
    ))
    .unwrap();
    let (dir, ws, _files, mut registry, scope) = setup();
    fs::create_dir(dir.path().join(".git")).unwrap();
    for (path, text) in oracle["files"].as_object().unwrap() {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text.as_str().unwrap()).unwrap();
    }
    registry
        .activate(
            &["grep".into(), "find".into()],
            &["grep".into(), "find".into()].into(),
        )
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor = Arc::new(
        ProcessSupervisor::new(std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")))
            .unwrap(),
    );
    let search = SearchService::new(
        ws,
        supervisor,
        "/usr/bin/rg".as_ref(),
        "/usr/bin/fd".as_ref(),
        spools.path(),
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let mut count = 0;
    for name in ["grep", "find"] {
        for (i, row) in oracle[name].as_array().unwrap().iter().enumerate() {
            let call = call(
                &registry,
                &format!("{name}-{i}"),
                name,
                row["input"].clone(),
            );
            let ticket = search_ticket(
                call,
                &search.target_revision(),
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            let result = search.execute(
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &cancel,
                    deadline: Instant::now() + Duration::from_secs(5),
                },
            );
            count += 1;
            if let Some(error) = row["output"]["error"].as_str() {
                let result = result.err().unwrap();
                assert!(
                    matches!(result, DomainError::InvalidInput(ref text) if text == error),
                    "{name} {i}: {result:?}"
                );
            } else {
                let mut expected = row["output"]["result"].clone();
                if name == "grep" && [0, 5, 7].contains(&i) {
                    // The SDK normalizes lone CRs into extra rows, then uses rg's LF-only
                    // line numbers. It labels `two` as the hit although the match is `needle`.
                    let text = expected["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .replace("mixed.txt:2: two", "mixed.txt:2: needle")
                        .replace("mixed.txt-1- one\n", "mixed.txt-1- one\\rtwo\n")
                        .replace("mixed.txt-3- needle", "mixed.txt-3- ");
                    expected["content"][0]["text"] = json!(text);
                }
                assert_eq!(
                    result
                        .unwrap_or_else(|e| panic!("{name} {i}: {e:?}"))
                        .result,
                    expected,
                    "{name} {i}"
                );
            }
        }
    }
    assert_eq!(count, 26);
}

#[test]
fn search_rejects_stale_sources_escape_cancel_and_transport_overflow() {
    use ag1_platform::{process::ProcessSupervisor, search_tools::SearchService};
    let (dir, ws, _files, mut registry, scope) = setup();
    registry
        .activate(
            &["grep".into(), "find".into()],
            &["grep".into(), "find".into()].into(),
        )
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor = Arc::new(
        ProcessSupervisor::new(std::path::Path::new(env!("CARGO_BIN_EXE_ag1-process-host")))
            .unwrap(),
    );
    let search = SearchService::new(
        ws,
        supervisor,
        "/usr/bin/rg".as_ref(),
        "/usr/bin/fd".as_ref(),
        spools.path(),
    )
    .unwrap();
    let run = |id: &str,
               name: &str,
               args: Value,
               cancel: bool,
               duration: Duration,
               barrier: &dyn Fn()| {
        let call = call(&registry, id, name, args);
        let ticket = search_ticket(
            call,
            &search.target_revision(),
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        let cancel = AtomicBool::new(cancel);
        search.execute_with_barrier(
            ticket,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &cancel,
                deadline: Instant::now() + duration,
            },
            barrier,
        )
    };
    fs::write(dir.path().join("source.txt"), "original\n").unwrap();
    let duration = Duration::from_secs(10);
    assert!(matches!(
        run(
            "stale-miss",
            "grep",
            json!({"pattern":"needle","path":"source.txt"}),
            false,
            duration,
            &|| {
                fs::write(dir.path().join("source.txt"), "needle\n").unwrap();
            }
        ),
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        run(
            "stale-hit",
            "grep",
            json!({"pattern":"needle"}),
            false,
            duration,
            &|| {
                fs::write(dir.path().join("source.txt"), "changed\n").unwrap();
            }
        ),
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        run(
            "cancel",
            "grep",
            json!({"pattern":"x"}),
            true,
            duration,
            &|| {}
        ),
        Err(DomainError::Cancelled)
    ));
    assert!(matches!(
        run(
            "timeout",
            "grep",
            json!({"pattern":"x"}),
            false,
            Duration::ZERO,
            &|| {}
        ),
        Err(DomainError::TimedOut)
    ));
    assert!(matches!(
        run(
            "escape",
            "grep",
            json!({"pattern":"x", "path":"../../etc/passwd"}),
            false,
            duration,
            &|| {}
        ),
        Err(DomainError::PermissionDenied)
    ));
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("private.txt"), "private needle").unwrap();
    symlink(
        outside.path().join("private.txt"),
        dir.path().join("linked.txt"),
    )
    .unwrap();
    assert!(
        run(
            "link",
            "grep",
            json!({"pattern":"needle","path":"linked.txt"}),
            false,
            duration,
            &|| {}
        )
        .is_err()
    );
    let result = run(
        "directory-link",
        "grep",
        json!({"pattern":"private needle"}),
        false,
        duration,
        &|| {},
    )
    .unwrap()
    .result;
    assert_eq!(result["details"]["matchCount"], 0);
    assert!(result["details"].get("inspectionEvidence").is_none());
    assert!(matches!(
        run(
            "context-cap",
            "grep",
            json!({"pattern":"x","context":101}),
            false,
            duration,
            &|| {}
        ),
        Err(DomainError::LimitExceeded(_))
    ));
    assert!(matches!(
        run(
            "pattern-cap",
            "grep",
            json!({"pattern":"x".repeat(4097)}),
            false,
            duration,
            &|| {}
        ),
        Err(DomainError::LimitExceeded(_))
    ));
    // Input exceeds the kernel pipe; progress requires simultaneous writes and stdout draining.
    fs::write(dir.path().join("large.txt"), "no hit here\n".repeat(100000)).unwrap();
    let result = run(
        "large-stdin",
        "grep",
        json!({"pattern":"missing", "path":"large.txt"}),
        false,
        duration,
        &|| {},
    )
    .unwrap()
    .result;
    assert_eq!(result["details"]["inspectionEvidence"]["kind"], "no_match");
    // A very dense match record would expand into millions of submatch objects.
    // The transport rejects it while the OS also bounds the child address space.
    fs::write(dir.path().join("dense.txt"), "x".repeat(2 * 1024 * 1024)).unwrap();
    assert!(
        run(
            "dense-record",
            "grep",
            json!({"pattern":"x","path":"dense.txt"}),
            false,
            duration,
            &|| {}
        )
        .is_err()
    );
    assert_eq!(
        fs::read_dir(spools.path()).unwrap().count(),
        0,
        "completed and failed search spools are scoped to their operation"
    );
}

#[test]
fn native_git_diff_matches_14_reference_cases_and_preserves_literal_paths() {
    use ag1_platform::{git::GitService, git_tools::GitTools};
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/git-oracle.json")).unwrap();
    let (dir, ws, _files, mut registry, scope) = setup();
    let git = |args: &[&str]| {
        let result = std::process::Command::new("/usr/bin/git")
            .args(args)
            .current_dir(dir.path())
            .env_clear()
            .envs([
                ("PATH", "/usr/bin:/bin"),
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_CONFIG_GLOBAL", "/dev/null"),
                ("GIT_AUTHOR_NAME", "Fixture"),
                ("GIT_AUTHOR_EMAIL", "fixture@localhost"),
                ("GIT_COMMITTER_NAME", "Fixture"),
                ("GIT_COMMITTER_EMAIL", "fixture@localhost"),
                ("GIT_AUTHOR_DATE", "2020-01-01T00:00:00Z"),
                ("GIT_COMMITTER_DATE", "2020-01-01T00:00:00Z"),
                ("LC_ALL", "C"),
            ])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    for (name, text) in oracle["initial"].as_object().unwrap() {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text.as_str().unwrap()).unwrap();
    }
    git(&["add", "."]);
    git(&["commit", "-m", "fixture"]);
    git(&["branch", "base"]);
    for (name, text) in oracle["changed"].as_object().unwrap() {
        fs::write(dir.path().join(name), text.as_str().unwrap()).unwrap();
    }
    registry
        .activate(&["diff".into()], &["diff".into()].into())
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let service = GitService::new(
        env!("CARGO_BIN_EXE_ag1-process-host").as_ref(),
        "/usr/bin/git".as_ref(),
        spools.path(),
    )
    .unwrap();
    let diff = GitTools::new(ws, service);
    let run = |id: &str, args: Value| {
        let call = call(&registry, id, "diff", args);
        let ticket = search_ticket(
            call,
            &diff.target_revision(),
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        diff.execute(
            ticket,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(5),
            },
        )
    };
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let mut expected = row["output"].clone();
        if i == 6 {
            expected = oracle["rows"][11]["output"].clone();
        }
        if i == 8 {
            expected["details"]["truncated"] = json!(true);
            expected["content"][0]["text"] = json!(
                expected["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .replace("\"truncated\": false", "\"truncated\": true")
            );
        }
        assert_eq!(
            run(&format!("git-{i}"), row["input"].clone())
                .unwrap()
                .result,
            expected,
            "Git case {i}"
        );
    }
    assert!(matches!(
        run("outside", json!({"from":"../../outside"})),
        Err(DomainError::PermissionDenied)
    ));
    assert!(matches!(
        run(
            "ref-injection",
            json!({"againstBranch":"main; touch breach"})
        ),
        Err(DomainError::InvalidInput(_))
    ));
    let name = "$(touch breach).txt";
    fs::write(dir.path().join(name), "old\n").unwrap();
    git(&["add", "--", name]);
    fs::write(dir.path().join(name), "new\n").unwrap();
    let result = run("literal-shell-text", json!({"from":name}))
        .unwrap()
        .result;
    assert_eq!(result["details"]["added"], 1);
    assert!(!dir.path().join("breach").exists());
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}

#[test]
fn search_process_memory_limit_is_inherited_without_changing_default_process_specs() {
    use ag1_platform::process::{ProcessSpec, ProcessSupervisor};
    let (dir, ws, _, _, _) = setup();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap();
    let pinned = ws.pin_directory("").unwrap();
    let mut child = supervisor
        .spawn_protocol_limited(
            ProcessSpec {
                executable: env!("CARGO_BIN_EXE_ag1-platform-fixture").into(),
                argv: vec!["address-space".into()],
                cwd: pinned.process_cwd(),
                env: Default::default(),
                timeout_ms: 1000,
                grace_ms: 100,
                resident: false,
            },
            &spools.path().join("limits.log"),
            1024 * 1024 * 1024,
        )
        .unwrap();
    child.close_input();
    let mut bytes = vec![];
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let part = child.read_stdout(1024).unwrap();
        let empty = part.is_empty();
        bytes.extend(part);
        if empty && child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(String::from_utf8(bytes).unwrap(), "1073741824 1073741824\n");
    assert!(dir.path().exists());
}

#[test]
fn native_shell_pipes_status_full_output_and_replay_use_durable_artifacts() {
    use ag1_core::{
        Clock, SessionId,
        context::features::Features,
        tools::{
            executor::{self, Effect, Execution, ExecutionArtifacts, NoHooks},
            result::ResultBudget,
        },
    };
    use ag1_platform::{
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    };
    use ag1_storage::{Store, context::ContextStore};
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            12
        }
    }
    let (dir, ws, _, mut registry, scope) = setup();
    registry
        .activate(&["bash".into()], &["bash".into()].into())
        .unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[4; 32])
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let shell = ShellService::new(
        ws,
        supervisor,
        spools.path(),
        ShellConfig {
            executable: "/bin/bash".into(),
            env: std::collections::BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            command_prefix: None,
            command_template: None,
            sandbox: Sandbox::Off,
            bubblewrap: None,
            output_bytes: 2 * 1024 * 1024,
            address_space: Some(1024 * 1024 * 1024),
        },
    )
    .unwrap();
    let cases = [
        ("ok", "printf 'hello\\n'", None, false, "hello\n"),
        ("pwd", "pwd", None, false, ""),
        ("linecount", "seq 3000", None, false, "full output artifact"),
        (
            "whitespace",
            "printf 'line1\\n\\tindented\\n'",
            None,
            false,
            "line1\n\tindented\n",
        ),
        (
            "sanitize",
            "printf 'hello \\033[31mred\\033[0m \\033]52;c;aGVsbG8=\\007\\001\\002done\\n'",
            None,
            false,
            "hello red done",
        ),
        (
            "sanitized-large",
            "printf 'HEAD\\n'; for i in {1..5000}; do printf 'payload line\\n'; done; printf 'TAIL\\n\\033[31mred\\033[0m\\n'",
            None,
            false,
            "full output artifact",
        ),
        (
            "inspection",
            "ls() { for i in {0..99}; do printf 'file-%s.ts\\n' \"$i\"; done; }; ls",
            None,
            false,
            "complete pre-compaction output",
        ),
        (
            "persist",
            "printf 'a\\nb\\nc\\n' > game.js",
            None,
            false,
            "Persisted 6B (4 lines) at game.js.",
        ),
        (
            "heredoc",
            "cat > cut.js << 'ENDOFFILE'\npartial\n",
            None,
            false,
            "here-document was cut off",
        ),
        ("stderr", "printf 'stderr\\n' >&2", None, false, "stderr\n"),
        (
            "pipeline",
            "false | true",
            None,
            true,
            "Command exited with code 1",
        ),
        (
            "exit137",
            "exit 137",
            None,
            true,
            "Command exited with code 137",
        ),
        (
            "signal",
            "kill -KILL $$",
            None,
            true,
            "Command was killed by a signal",
        ),
        (
            "timeout",
            "printf partial; sleep 1",
            Some(0.05),
            true,
            "Command timed out after 0.05 seconds",
        ),
        (
            "cancelled",
            "printf partial; sleep 1",
            None,
            true,
            "Command aborted",
        ),
        (
            "large",
            "/usr/bin/head -c 1000000 /dev/zero | /usr/bin/tr '\\0' x",
            None,
            false,
            "full output artifact",
        ),
    ];
    for (id, command, timeout, is_error, expected) in cases {
        let cancel = AtomicBool::new(false);
        let mut args = json!({"command":command});
        if let Some(timeout) = timeout {
            args["timeout"] = json!(timeout);
        }
        let original = call(&registry, id, "bash", args);
        let plan = shell.prepare(&original, &registry, &scope).unwrap();
        let stamp = plan.target_revision().to_owned();
        let prepared = plan.call().clone();
        let ticket = search_ticket(
            prepared.clone(),
            &stamp,
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        let mut artifacts = ContextStore::new(
            &store,
            SessionId::new("fixture").unwrap(),
            "fixture",
            id,
            timestamp,
            Features::default(),
        )
        .unwrap();
        let mut events = vec![];
        let outcome = executor::execute_with_artifacts(
            &store,
            &mut artifacts,
            ticket,
            Execution {
                registry: &registry,
                fence: ag1_core::tools::approval::ExecutionFence {
                    scope: &scope,
                    policy_revision: 1,
                    target_revision: &stamp,
                    now: 12,
                },
                budget: ResultBudget::default(),
                clock: &FixedClock,
            },
            &mut NoHooks,
            |ticket, artifacts| {
                shell.execute(
                    plan,
                    ticket,
                    ShellExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &cancel,
                        deadline: Instant::now() + Duration::from_secs(5),
                    },
                    artifacts,
                    |_| {
                        if id == "cancelled" {
                            cancel.store(true, Ordering::Release);
                        }
                    },
                )
            },
            |e| events.push(e),
        )
        .unwrap();
        assert_eq!(outcome.record.effect, Effect::Unknown);
        assert_eq!(events.len(), 2);
        let native = artifacts
            .read_json(outcome.record.native_result.as_ref().unwrap())
            .unwrap();
        assert_eq!(native["isError"] == true, is_error, "{id}");
        assert!(
            native["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains(expected),
            "{id}: {native}"
        );
        assert_eq!(native["details"]["fullOutputRetained"], true);
        let raw = artifacts
            .read_json(
                &serde_json::from_value(native["details"]["rawOutputArtifact"].clone()).unwrap(),
            )
            .unwrap();
        assert_eq!(raw["complete"], true);
        let bytes = STANDARD.decode(raw["data"].as_str().unwrap()).unwrap();
        if id == "large" {
            assert_eq!(bytes.len(), 1000000);
            assert!(bytes.iter().all(|b| *b == b'x'));
            assert_eq!(
                outcome.result["details"]["toolResultProjection"]["artifacts"][0]["id"],
                native["details"]["fullOutputArtifact"]["id"]
            );
            assert_eq!(
                outcome.result["details"]["toolResultProjection"]["artifacts"][0]["bytes"],
                1000000
            );
            assert!(outcome.result["content"][0]["text"].as_str().unwrap().len() < 12000);
        } else if id == "ok" {
            assert_eq!(bytes, b"hello\n");
        }
        if id == "pwd" {
            assert_eq!(
                native["content"][0]["text"].as_str().unwrap().trim(),
                dir.path().to_str().unwrap()
            );
        }
        if id == "linecount" {
            assert_eq!(
                bytes,
                (1..=3000)
                    .map(|n| format!("{n}\n"))
                    .collect::<String>()
                    .as_bytes()
            );
            assert_eq!(native["details"]["truncation"]["truncatedBy"], "lines");
            let id = native["details"]["fullOutputArtifact"]["id"]
                .as_str()
                .unwrap();
            let head = store.page("fixture", "fixture", id, 0, 32).unwrap();
            let tail = store
                .page("fixture", "fixture", id, bytes.len() as u64 - 32, 32)
                .unwrap();
            assert!(head.bytes.starts_with(b"1\n2\n3\n"));
            assert!(tail.bytes.ends_with(b"2998\n2999\n3000\n"));
            assert_eq!(
                native["details"]["fullOutputArtifact"]["bytes"],
                bytes.len()
            );
        }
        if id == "sanitize" {
            let text = native["content"][0]["text"].as_str().unwrap();
            assert!(
                !text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            );
            assert!(bytes.contains(&0x1b), "raw custody retains terminal bytes");
        }
        if matches!(id, "sanitized-large" | "inspection") {
            let artifact = &native["details"]["fullOutputArtifact"];
            let mut full = Vec::new();
            while full.len() < artifact["bytes"].as_u64().unwrap() as usize {
                let page = store
                    .page(
                        "fixture",
                        "fixture",
                        artifact["id"].as_str().unwrap(),
                        full.len() as u64,
                        32 * 1024,
                    )
                    .unwrap();
                assert!(!page.bytes.is_empty());
                full.extend(page.bytes);
            }
            let text = String::from_utf8(full).unwrap();
            if id == "sanitized-large" {
                assert_eq!(
                    text,
                    format!("HEAD\n{}TAIL\nred\n", "payload line\n".repeat(5000))
                );
            } else {
                assert_eq!(
                    text,
                    (0..100)
                        .map(|i| format!("file-{i}.ts\n"))
                        .collect::<String>()
                );
                assert!(
                    !native["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("file-99.ts")
                );
            }
            assert_eq!(artifact["bytes"], text.len());
        }
        if id == "persist" {
            assert_eq!(fs::read(dir.path().join("game.js")).unwrap(), b"a\nb\nc\n");
        }
        if id == "heredoc" {
            assert!(
                native["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("write tool")
            );
            assert_eq!(fs::read(dir.path().join("cut.js")).unwrap(), b"partial\n");
        }

        let replay = executor::replay(&store, &artifacts, &prepared, &scope)
            .unwrap()
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.result, outcome.result);
    }
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}

#[test]
fn native_jobs_match_reference_poll_logs_and_deliver_stable_durable_notices() {
    use ag1_core::{Clock, SessionId, context::features::Features};
    use ag1_platform::{
        job_tools::JobService,
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    };
    use ag1_storage::{Store, context::ContextStore};
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            10000
        }
    }
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/job-oracle.json")).unwrap();
    let (dir, ws, _, mut registry, scope) = setup();
    registry
        .activate(&["job".into()], &["job".into()].into())
        .unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(data.path()).unwrap());
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[5; 32])
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let shell = Arc::new(
        ShellService::new(
            ws,
            supervisor,
            spools.path(),
            ShellConfig {
                executable: "/bin/bash".into(),
                env: std::collections::BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                command_prefix: None,
                command_template: None,
                sandbox: Sandbox::Off,
                bubblewrap: None,
                output_bytes: 2 * 1024 * 1024,
                address_space: Some(1024 * 1024 * 1024),
            },
        )
        .unwrap(),
    );
    let jobs = JobService::new(
        store.clone(),
        shell.clone(),
        Arc::new(FixedClock),
        scope.clone(),
        "job-owner",
    )
    .unwrap();
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "job-fixture",
        timestamp,
        Features::default(),
    )
    .unwrap();
    let mut ids: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let pid_pattern = regex::Regex::new(r"\(pid \d+\)").unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let mut input = row["input"].clone();
        if let Some(id) = input["id"].as_str().and_then(|id| ids.get(id)).cloned() {
            input["id"] = json!(id);
        }
        let call = call(&registry, &format!("job-{i}"), "job", input);
        let plan = if call.args()["action"] == "start" {
            Some(shell.prepare_job(&call, &registry, &scope).unwrap())
        } else {
            None
        };
        let prepared = plan.as_ref().map(|p| p.call().clone()).unwrap_or(call);
        let ticket = search_ticket(
            prepared,
            jobs.target_revision(),
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        let outcome = jobs.execute(
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
            &mut artifacts,
        );
        if let Some(error) = row["output"]["error"].as_str() {
            let mut error = error.to_string();
            for (expected, actual) in &ids {
                error = error.replace(expected, actual);
            }
            assert!(
                matches!(outcome,Err(DomainError::InvalidInput(ref actual)) if actual==&error),
                "job {i}: {}",
                outcome.err().map(|e| format!("{e:?}")).unwrap_or_default()
            );
        } else {
            let mut result = outcome.unwrap().result;
            if result["details"]["action"] == "start" {
                ids.insert(
                    row["output"]["result"]["details"]["id"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    result["details"]["id"].as_str().unwrap().to_owned(),
                );
                // Host timer drains owned processes and retains output before making notices visible.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    jobs.refresh(&mut artifacts).unwrap();
                    if jobs.notices().unwrap().len() == ids.len() {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            if let Some(details) = result["details"].as_object_mut() {
                details.remove("resultArtifact");
            }
            let mut encoded = result.to_string();
            for (expected, actual) in &ids {
                encoded = encoded.replace(actual, expected);
            }
            encoded = pid_pattern.replace_all(&encoded, "(pid 42)").into_owned();
            assert_eq!(
                serde_json::from_str::<Value>(&encoded).unwrap(),
                row["output"]["result"],
                "job {i}"
            );
        }
    }
    let notices = jobs.notices().unwrap();
    assert_eq!(notices.len(), 2);
    assert_eq!(jobs.notices().unwrap()[0].id, notices[0].id);
    jobs.acknowledge_notice(&notices[0].id).unwrap();
    jobs.acknowledge_notice(&notices[0].id).unwrap();
    assert_eq!(jobs.notices().unwrap().len(), 1);
    drop(jobs);
    let reopened = JobService::new(
        store.clone(),
        shell,
        Arc::new(FixedClock),
        scope,
        "job-owner-restarted",
    )
    .unwrap();
    assert_eq!(reopened.recover_orphans().unwrap(), 0);
    assert_eq!(reopened.notices().unwrap().len(), 1);
    reopened.acknowledge_notice(&notices[1].id).unwrap();
    assert!(reopened.notices().unwrap().is_empty());
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}

#[test]
fn shell_preparation_cancel_quota_and_missing_sandbox_never_claim_unperformed_work() {
    use ag1_core::{SessionId, context::features::Features, tools::executor::ExecutionArtifacts};
    use ag1_platform::{
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    };
    use ag1_storage::{Store, context::ContextStore};
    let (dir, ws, _, mut registry, scope) = setup();
    registry
        .activate(&["bash".into()], &["bash".into()].into())
        .unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[6; 32])
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let config = ShellConfig {
        executable: "/bin/bash".into(),
        env: std::collections::BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        command_prefix: None,
        command_template: None,
        sandbox: Sandbox::Off,
        bubblewrap: None,
        output_bytes: 1024,
        address_space: Some(1024 * 1024 * 1024),
    };
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "shell-edge",
        timestamp,
        Features::default(),
    )
    .unwrap();
    let mut prefixed = config.clone();
    prefixed.command_prefix = Some("printf prepared > marker".into());
    let shell = ShellService::new(ws.clone(), supervisor.clone(), spools.path(), prefixed).unwrap();
    let original = call(
        &registry,
        "prefix",
        "bash",
        json!({"command":"printf main"}),
    );
    let plan = shell.prepare(&original, &registry, &scope).unwrap();
    assert!(!dir.path().join("marker").exists());
    let ticket = search_ticket(
        original.clone(),
        plan.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    let cancel = AtomicBool::new(false);
    let execution = || ShellExecution {
        registry: &registry,
        scope: &scope,
        policy_revision: 1,
        now: || 12,
        cancel: &cancel,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    assert!(matches!(
        shell.execute(plan, ticket, execution(), &mut artifacts, |_| {}),
        Err(DomainError::Conflict)
    ));
    assert!(!dir.path().join("marker").exists());
    let plan = shell.prepare(&original, &registry, &scope).unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    let result = shell
        .execute(plan, ticket, execution(), &mut artifacts, |_| {})
        .unwrap()
        .result;
    assert_eq!(fs::read(dir.path().join("marker")).unwrap(), b"prepared");
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Persisted 8B (1 line) at marker.")
    );
    for (index, (prefix, command, expected)) in [
        (Some("export TEST_VAR=hello"), "echo $TEST_VAR", "hello"),
        (
            Some("echo prefix-output"),
            "echo command-output",
            "prefix-output\ncommand-output",
        ),
        (None, "echo no-prefix", "no-prefix"),
        (None, "echo $TEST_LOCAL_BASH_OPS", "from-local-ops"),
        (None, "printf '\\033[31mred\\033[0m\\r\\n'", "red"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut configured = config.clone();
        configured.command_prefix = prefix.map(str::to_owned);
        configured
            .env
            .insert("TEST_LOCAL_BASH_OPS".into(), "from-local-ops".into());
        let reusable =
            ShellService::new(ws.clone(), supervisor.clone(), spools.path(), configured).unwrap();
        let call = call(
            &registry,
            &format!("reusable-{index}"),
            "bash",
            json!({"command":command}),
        );
        let plan = reusable.prepare(&call, &registry, &scope).unwrap();
        let ticket = search_ticket(
            plan.call().clone(),
            plan.target_revision(),
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        let result = reusable
            .execute(plan, ticket, execution(), &mut artifacts, |_| {})
            .unwrap()
            .result;
        assert_eq!(
            result["content"][0]["text"].as_str().unwrap().trim(),
            expected
        );
        assert_eq!(result["details"]["termination"], "exit");
        assert_ne!(result["isError"], true);
    }
    assert!(matches!(
        WorkspaceService::new()
            .unwrap()
            .register(&dir.path().join("absent-cwd")),
        Err(DomainError::NotFound)
    ));
    let mut invalid_shell = config.clone();
    invalid_shell.executable = dir.path().join("absent-shell");
    let unavailable =
        ShellService::new(ws.clone(), supervisor.clone(), spools.path(), invalid_shell).unwrap();
    let failed_call = call(
        &registry,
        "spawn-failure",
        "bash",
        json!({"command":"printf no > unspawned"}),
    );
    let plan = unavailable
        .prepare(&failed_call, &registry, &scope)
        .unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    assert!(matches!(
        unavailable.execute(plan, ticket, execution(), &mut artifacts, |_| {}),
        Err(DomainError::NotFound)
    ));
    assert!(!dir.path().join("unspawned").exists());
    let shell = ShellService::new(
        ws.clone(),
        supervisor.clone(),
        spools.path(),
        config.clone(),
    )
    .unwrap();
    let call = call(
        &registry,
        "quota",
        "bash",
        json!({"command":"head -c 1000000 /dev/zero | tr '\\0' x"}),
    );
    let plan = shell.prepare(&call, &registry, &scope).unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    let result = shell
        .execute(plan, ticket, execution(), &mut artifacts, |_| {})
        .unwrap()
        .result;
    assert_eq!(result["details"]["fullOutputRetained"], false);
    assert_eq!(result["details"]["termination"], "output_limit");
    assert_eq!(result["isError"], true);
    let raw = artifacts
        .read_json(&serde_json::from_value(result["details"]["rawOutputArtifact"].clone()).unwrap())
        .unwrap();
    assert_eq!(raw["bytes"], 1024);
    assert_eq!(raw["complete"], false);
    let plan = shell.prepare(&call, &registry, &scope).unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    cancel.store(true, Ordering::Release);
    assert!(matches!(
        shell.execute(plan, ticket, execution(), &mut artifacts, |_| {}),
        Err(DomainError::Cancelled)
    ));
    cancel.store(false, Ordering::Release);
    let mut missing = config;
    missing.sandbox = Sandbox::Strict;
    let sandboxed = ShellService::new(ws, supervisor, spools.path(), missing).unwrap();
    assert!(matches!(
        sandboxed.prepare(&call, &registry, &scope),
        Err(DomainError::Unavailable(_))
    ));
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}

#[test]
fn owned_jobs_kill_quota_and_restart_never_adopt_historical_pids_or_foreign_scopes() {
    use ag1_core::{
        Clock, SessionId, context::features::Features, storage::StoragePort,
        tools::executor::ExecutionArtifacts,
    };
    use ag1_platform::{
        job_tools::JobService,
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    };
    use ag1_storage::{Store, context::ContextStore};
    use sha2::{Digest, Sha256};
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            10000
        }
    }
    let (dir, ws, _, mut registry, scope) = setup();
    registry
        .activate(&["job".into()], &["job".into()].into())
        .unwrap();
    let data = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(data.path()).unwrap());
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[7; 32])
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let shell = Arc::new(
        ShellService::new(
            ws,
            supervisor,
            spools.path(),
            ShellConfig {
                executable: "/bin/bash".into(),
                env: std::collections::BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
                command_prefix: None,
                command_template: None,
                sandbox: Sandbox::Off,
                bubblewrap: None,
                output_bytes: 1024,
                address_space: Some(1024 * 1024 * 1024),
            },
        )
        .unwrap(),
    );
    let jobs = JobService::new(
        store.clone(),
        shell.clone(),
        Arc::new(FixedClock),
        scope.clone(),
        "job-owner",
    )
    .unwrap();
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "job-edges",
        timestamp,
        Features::default(),
    )
    .unwrap();
    let run = |jobs: &JobService<Store>,
               id: &str,
               args: Value,
               request_scope: &Scope,
               artifacts: &mut ContextStore<'_>| {
        let call = call(&registry, id, "job", args);
        let plan = if call.args()["action"] == "start" {
            Some(shell.prepare_job(&call, &registry, request_scope).unwrap())
        } else {
            None
        };
        let prepared = plan.as_ref().map(|p| p.call().clone()).unwrap_or(call);
        let ticket = search_ticket(
            prepared,
            jobs.target_revision(),
            &registry,
            request_scope,
            dir.path().to_str().unwrap(),
        );
        jobs.execute(
            plan,
            ticket,
            ShellExecution {
                registry: &registry,
                scope: request_scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(5),
            },
            artifacts,
        )
    };
    // Two callers reach one owned service together; neither durable start can overwrite the other.
    let barrier = std::sync::Barrier::new(2);
    let concurrent = std::thread::scope(|threads| {
        let handles: Vec<_> = ["one", "two"]
            .into_iter()
            .map(|word| {
                let run = &run;
                let jobs = &jobs;
                let store = &store;
                let scope = &scope;
                let barrier = &barrier;
                threads.spawn(move || {
                    let mut artifacts = ContextStore::new(
                        store,
                        SessionId::new("fixture").unwrap(),
                        "fixture",
                        &format!("concurrent-{word}"),
                        timestamp,
                        Features::default(),
                    )
                    .unwrap();
                    barrier.wait();
                    run(
                        jobs,
                        &format!("concurrent-{word}"),
                        json!({"action":"start","command":format!("printf {word}")}),
                        scope,
                        &mut artifacts,
                    )
                    .unwrap()
                    .result["details"]["id"]
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_ne!(concurrent[0], concurrent[1]);
    let deadline = Instant::now() + Duration::from_secs(5);
    while jobs.notices().unwrap().len() < 2 {
        jobs.refresh(&mut artifacts).unwrap();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let listed = run(
        &jobs,
        "concurrent-list",
        json!({"action":"list"}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    for (id, word) in concurrent.iter().zip(["one", "two"]) {
        assert!(listed["content"][0]["text"].as_str().unwrap().contains(id));
        let result = run(
            &jobs,
            &format!("concurrent-poll-{word}"),
            json!({"action":"poll","id":id}),
            &scope,
            &mut artifacts,
        )
        .unwrap()
        .result;
        assert_eq!(result["details"]["status"], "exited");
        assert!(
            result["content"][0]["text"]
                .as_str()
                .unwrap()
                .ends_with(word)
        );
    }
    for notice in jobs.notices().unwrap() {
        jobs.acknowledge_notice(&notice.id).unwrap();
    }
    assert!(jobs.notices().unwrap().is_empty());
    let start = run(
        &jobs,
        "start-kill",
        json!({"action":"start","command":"printf before; sleep 10; printf after"}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    let id = start["details"]["id"].as_str().unwrap();
    let running = run(
        &jobs,
        "running-poll",
        json!({"action":"poll","id":id}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    assert_eq!(running["details"]["status"], "running");
    assert_eq!(
        running["content"][0]["text"]
            .as_str()
            .unwrap()
            .lines()
            .count(),
        1
    );
    let mut foreign = scope.clone();
    foreign.session = "other-session".into();
    // Compaction preserves the service's process ownership, but an approval
    // issued for the earlier request context must still fail at consumption.
    let later = Scope {
        epoch: scope.epoch + 1,
        ..scope.clone()
    };
    let old_call = call(
        &registry,
        "stale-job-kill",
        "job",
        json!({"action":"kill","id":id}),
    );
    let old_ticket = search_ticket(
        old_call,
        jobs.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    assert!(matches!(
        jobs.execute(
            None,
            old_ticket,
            ShellExecution {
                registry: &registry,
                scope: &later,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(5),
            },
            &mut artifacts
        ),
        Err(DomainError::Conflict)
    ));
    assert_eq!(
        run(
            &jobs,
            "fresh-later-context",
            json!({"action":"poll","id":id}),
            &later,
            &mut artifacts
        )
        .unwrap()
        .result["details"]["status"],
        "running"
    );
    assert!(matches!(
        run(
            &jobs,
            "foreign-kill",
            json!({"action":"kill","id":id}),
            &foreign,
            &mut artifacts
        ),
        Err(DomainError::PermissionDenied)
    ));
    let killed = run(
        &jobs,
        "kill",
        json!({"action":"kill","id":id}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    assert_eq!(killed["details"]["status"], "exited");
    let polled = run(
        &jobs,
        "after-kill",
        json!({"action":"poll","id":id}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    assert!(
        polled["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Command aborted")
    );
    let start = run(
        &jobs,
        "start-orphan",
        json!({"action":"start","command":"sleep 10"}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    let orphan = start["details"]["id"].as_str().unwrap().to_owned();
    drop(jobs);
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
    // A restored PID field can name this test process. Recovery must never signal/adopt it.
    let key = format!(
        "jobs-{:x}",
        Sha256::digest(json!([scope.workspace, scope.session]).to_string())
    );
    let document = store
        .load_metadata("background-jobs", &key)
        .unwrap()
        .unwrap();
    let mut book: Value = serde_json::from_slice(&document.bytes).unwrap();
    book["jobs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|j| j["id"] == orphan)
        .unwrap()["pid"] = json!(std::process::id());
    store
        .commit_metadata(
            "background-jobs",
            &key,
            Some(&document.sha256),
            &serde_json::to_vec(&book).unwrap(),
        )
        .unwrap();
    let restarted = JobService::new(
        store.clone(),
        shell.clone(),
        Arc::new(FixedClock),
        scope.clone(),
        "new-job-owner",
    )
    .unwrap();
    assert_eq!(restarted.recover_orphans().unwrap(), 1);
    assert!(matches!(
        run(
            &restarted,
            "historical-kill",
            json!({"action":"kill","id":orphan}),
            &scope,
            &mut artifacts
        ),
        Err(DomainError::InvalidInput(_))
    ));
    let polled = run(
        &restarted,
        "historical-poll",
        json!({"action":"poll","id":orphan}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    assert_eq!(polled["details"]["status"], "unknown");
    assert!(
        polled["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("outcome unknown")
    );
    let start = run(
        &restarted,
        "job-quota",
        json!({"action":"start","command":"head -c 1000000 /dev/zero | tr '\\0' x"}),
        &scope,
        &mut artifacts,
    )
    .unwrap()
    .result;
    let quota = start["details"]["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        let result = run(
            &restarted,
            "quota-poll",
            json!({"action":"poll","id":quota}),
            &scope,
            &mut artifacts,
        )
        .unwrap()
        .result;
        if result["details"]["status"] == "exited" {
            break result;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("output quota")
    );
    let raw = artifacts
        .read_json(&serde_json::from_value(result["details"]["resultArtifact"].clone()).unwrap())
        .unwrap();
    assert_eq!(raw["details"]["fullOutputRetained"], false);
    for i in 0..20 {
        run(
            &restarted,
            &format!("bounded-job-{i}"),
            json!({"action":"start","command":"sleep 5"}),
            &scope,
            &mut artifacts,
        )
        .unwrap();
    }
    assert!(matches!(
        run(
            &restarted,
            "overflow-job",
            json!({"action":"start","command":"printf overflow > overflow"}),
            &scope,
            &mut artifacts
        ),
        Err(DomainError::QueueFull)
    ));
    assert!(!dir.path().join("overflow").exists());
    drop(restarted);
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}

#[test]
fn configured_bubblewrap_is_checked_with_real_namespace_and_workspace_write_probes() {
    use ag1_core::{SessionId, context::features::Features, tools::shell::quote};
    use ag1_platform::{
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService, sandbox_path},
    };
    use ag1_storage::{Store, context::ContextStore};
    let (dir, ws, _, mut registry, scope) = setup();
    registry
        .activate(&["bash".into()], &["bash".into()].into())
        .unwrap();
    let outside = tempfile::tempdir_in(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.local"),
    )
    .unwrap();
    let output = outside.path().join("must-not-write");
    let config_path = format!(
        "{}:/usr/bin:.:/bin:/usr/bin",
        dir.path().join("tools").display()
    );
    let filtered = sandbox_path(dir.path(), Some(&config_path));
    assert!(!filtered.contains(dir.path().to_str().unwrap()));
    assert_eq!(filtered.matches("/usr/bin").count(), 1);
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-05T00:00:00.000Z";
    store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
    store
        .register_origin("fixture", "fixture", &[8; 32])
        .unwrap();
    let spools = tempfile::tempdir().unwrap();
    let supervisor =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let shell = ShellService::new(
        ws,
        supervisor,
        spools.path(),
        ShellConfig {
            executable: "/bin/bash".into(),
            env: std::collections::BTreeMap::from([("PATH".into(), config_path)]),
            command_prefix: None,
            command_template: None,
            sandbox: Sandbox::Strict,
            bubblewrap: Some("/usr/bin/bwrap".into()),
            output_bytes: 64 * 1024,
            address_space: Some(1024 * 1024 * 1024),
        },
    )
    .unwrap();
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "sandbox-real",
        timestamp,
        Features::default(),
    )
    .unwrap();
    let private_tmp = std::path::PathBuf::from(format!(
        "/tmp/ag1-sandbox-private-{}-{}",
        std::process::id(),
        dir.path().file_name().unwrap().to_str().unwrap()
    ));
    assert!(!private_tmp.exists());
    let original = call(
        &registry,
        "sandbox-real",
        "bash",
        json!({"command":format!("readlink /proc/self/ns/net; for descriptor in /proc/self/fd/*; do readlink \"$descriptor\"; done; printf inside > inside; printf private > {}; printf outside > {}; false | tail -n 1",quote(private_tmp.to_str().unwrap()),quote(output.to_str().unwrap()))}),
    );
    let plan = shell.prepare(&original, &registry, &scope).unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    let result = shell
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
            &mut artifacts,
            |_| {},
        )
        .unwrap()
        .result;
    assert!(
        !output.exists(),
        "sandbox must never fall back to unrestricted execution"
    );
    assert!(
        !private_tmp.exists(),
        "sandbox /tmp writes must not reach the host"
    );
    let text = result["content"][0]["text"].as_str().unwrap();
    if dir.path().join("inside").exists() {
        assert_eq!(result["isError"], true);
        assert!(
            text.contains("Command exited with code 1"),
            "inner pipeline must retain pipefail: {text}"
        );
        assert_eq!(fs::read(dir.path().join("inside")).unwrap(), b"inside");
        let network = fs::read_link("/proc/self/ns/net").unwrap();
        assert!(
            !text.lines().any(|line| line == network.to_str().unwrap()),
            "strict mode must have its own network namespace"
        );
        assert!(
            !text
                .lines()
                .any(|line| line.contains(dir.path().to_str().unwrap())),
            "workspace fd must not leak into the sandbox command"
        );
        assert!(
            text.contains("Read-only file system") || text.contains("No such file or directory"),
            "{text}"
        );
        println!(
            "sandbox probe: kernel namespace active, workspace writable, outside write rejected"
        );
    } else {
        assert!(
            text.contains("No permissions to create new namespace")
                || text.contains("Creating new namespace failed")
                || text.contains("Failed to make / slave"),
            "unexpected sandbox failure: {text}"
        );
        assert_eq!(result["isError"], true);
        println!(
            "sandbox probe: namespace unavailable on this host; command rejected with no fallback"
        );
    }
}

#[test]
fn job_intent_and_start_ack_faults_never_launch_twice() {
    use ag1_core::{Clock, SessionId, context::features::Features};
    use ag1_platform::{
        job_tools::JobService,
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    };
    use ag1_storage::{
        Boundary, FaultInjector, StorageError, StorageResult, Store, context::ContextStore,
    };
    struct FixedClock;
    impl Clock for FixedClock {
        fn now_ms(&self) -> u64 {
            10000
        }
    }
    struct Fault {
        point: Boundary,
        remaining: AtomicUsize,
    }
    impl FaultInjector for Fault {
        fn check(&self, point: Boundary) -> StorageResult<()> {
            if point == self.point
                && self
                    .remaining
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                    .ok()
                    == Some(1)
            {
                Err(StorageError::Injected(point))
            } else {
                Ok(())
            }
        }
    }
    for point in [
        Boundary::BeforeCommit,
        Boundary::AfterCommit,
        Boundary::BeforeAck,
    ] {
        for nth in [1, 2] {
            let (dir, ws, _, mut registry, scope) = setup();
            registry
                .activate(&["job".into()], &["job".into()].into())
                .unwrap();
            let data = tempfile::tempdir().unwrap();
            let fault = Arc::new(Fault {
                point,
                remaining: AtomicUsize::new(0),
            });
            let clock = Arc::new(FixedClock);
            let store =
                Arc::new(Store::open_with(data.path(), clock.clone(), fault.clone()).unwrap());
            let timestamp = "2026-09-05T00:00:00.000Z";
            store.create_session("fixture",json!({"type":"session","version":3,"id":"fixture","cwd":dir.path(),"timestamp":timestamp}).to_string().as_bytes()).unwrap();
            store
                .register_origin("fixture", "fixture", &[9; 32])
                .unwrap();
            let spools = tempfile::tempdir().unwrap();
            let supervisor = Arc::new(
                ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap(),
            );
            let shell = Arc::new(
                ShellService::new(
                    ws,
                    supervisor,
                    spools.path(),
                    ShellConfig {
                        executable: "/bin/bash".into(),
                        env: std::collections::BTreeMap::from([(
                            "PATH".into(),
                            "/usr/bin:/bin".into(),
                        )]),
                        command_prefix: None,
                        command_template: None,
                        sandbox: Sandbox::Off,
                        bubblewrap: None,
                        output_bytes: 1024,
                        address_space: Some(1024 * 1024 * 1024),
                    },
                )
                .unwrap(),
            );
            let jobs = JobService::new(
                store.clone(),
                shell.clone(),
                clock.clone(),
                scope.clone(),
                "fault-owner",
            )
            .unwrap();
            let mut artifacts = ContextStore::new(
                &store,
                SessionId::new("fixture").unwrap(),
                "fixture",
                "job-fault",
                timestamp,
                Features::default(),
            )
            .unwrap();
            let call = call(
                &registry,
                "job-fault",
                "job",
                json!({"action":"start","command":"printf x >> effects"}),
            );
            let plan = shell.prepare_job(&call, &registry, &scope).unwrap();
            let ticket = search_ticket(
                plan.call().clone(),
                jobs.target_revision(),
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            fault.remaining.store(nth, Ordering::SeqCst);
            let result = jobs.execute(
                Some(plan),
                ticket,
                ShellExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(5),
                },
                &mut artifacts,
            );
            assert!(result.is_err(), "{point:?} {nth}");
            fault.remaining.store(0, Ordering::SeqCst);
            if nth == 1 {
                assert!(!dir.path().join("effects").exists());
            } else {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    jobs.refresh(&mut artifacts).unwrap();
                    if !jobs.notices().unwrap().is_empty() {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert_eq!(fs::read(dir.path().join("effects")).unwrap(), b"x");
            }
            drop(jobs);
            let reopened =
                JobService::new(store.clone(), shell, clock, scope, "restarted-fault-owner")
                    .unwrap();
            reopened.recover_orphans().unwrap();
            reopened.refresh(&mut artifacts).unwrap();
            if nth == 1 {
                assert!(!dir.path().join("effects").exists());
            } else {
                assert_eq!(fs::read(dir.path().join("effects")).unwrap(), b"x");
                assert_eq!(reopened.notices().unwrap().len(), 1);
            }
            assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn large_read_windows_match_seven_sdk_cases_with_bounded_retention_and_stream_freshness() {
    use ag1_core::{evidence::Freshness, tools::read::TextWindow};
    use ag1_platform::{source::SourceService, workspace::MAX_STREAM_READ};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/stream-read-oracle.json"
    ))
    .unwrap();
    for (i, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let (dir, ws, files, mut registry, scope) = setup();
        registry
            .activate(&["read".into()], &["read".into()].into())
            .unwrap();
        let input = &row["input"];
        let unit = if let Some(encoded) = input["unitBase64"].as_str() {
            STANDARD.decode(encoded).unwrap()
        } else {
            input["unit"].as_str().unwrap().as_bytes().to_vec()
        };
        let mut bytes = input["prefix"].as_str().unwrap_or("").as_bytes().to_vec();
        bytes.extend(unit.repeat(input["repeat"].as_u64().unwrap() as usize));
        bytes.extend(input["suffix"].as_str().unwrap_or("").as_bytes());
        assert!(bytes.len() > 8 * 1024 * 1024);
        fs::write(dir.path().join("large.txt"), &bytes).unwrap();
        drop(bytes);
        let call = call(
            &registry,
            &format!("stream-{i}"),
            "read",
            input["args"].clone(),
        );
        let plan = files.prepare(&call, &scope);
        if let Some(error) = row["output"]["error"].as_str() {
            assert!(
                matches!(plan,Err(DomainError::InvalidInput(ref actual)) if actual==error),
                "large read {i}"
            );
            continue;
        }
        let plan = plan.unwrap();
        let ticket = authorize(
            call.clone(),
            &plan,
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        let result = files
            .execute(
                plan,
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(5),
                },
            )
            .unwrap()
            .result;
        // The SDK omits evidence above its 8MiB snapshot ceiling. Native streaming
        // adds a byte-consistent reference; presentation/progress remain identical.
        let mut presentation = result.clone();
        presentation["details"]
            .as_object_mut()
            .unwrap()
            .remove("inspectionEvidence");
        assert!(
            presentation == row["output"]["result"],
            "large read {i}: presentation/progress mismatch"
        );
        let mut window = TextWindow::new(&input["args"]).unwrap();
        let mut peak = 0;
        ws.stream("large.txt", MAX_STREAM_READ, |chunk| {
            window.push(chunk)?;
            peak = peak.max(window.resident_bytes());
            Ok(())
        })
        .unwrap();
        assert!(peak <= 512 * 1024, "bounded retained sample: {peak}");
        assert_eq!(window.finish().unwrap()["content"], result["content"]);
        if i == 1 {
            let reference = result["details"]["inspectionEvidence"]["source"]
                .as_str()
                .unwrap();
            let mut source = SourceService::new(ws.clone());
            assert_eq!(source.validate(reference).status, Freshness::Current);
            let pending = files.prepare(&call, &scope).unwrap();
            let ticket = authorize(
                call.clone(),
                &pending,
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            use std::io::Write;
            fs::OpenOptions::new()
                .append(true)
                .open(dir.path().join("large.txt"))
                .unwrap()
                .write_all(b"changed")
                .unwrap();
            assert_eq!(source.validate(reference).status, Freshness::Stale);
            assert!(matches!(
                files.execute(
                    pending,
                    ticket,
                    ag1_platform::file_tools::FileExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &AtomicBool::new(false),
                        deadline: Instant::now() + Duration::from_secs(5)
                    }
                ),
                Err(DomainError::Conflict)
            ));
        }
    }
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 7);
}

#[test]
fn streamed_source_rejects_mutation_cancellation_special_files_and_oversize_input() {
    use ag1_platform::workspace::MAX_STREAM_READ;
    let (dir, ws, _, _, _) = setup();
    fs::write(dir.path().join("source"), b"before\n").unwrap();
    assert!(matches!(
        ws.stream_with_barrier(
            "source",
            MAX_STREAM_READ,
            |_| Ok(()),
            || fs::write(dir.path().join("source"), b"after!\n").unwrap()
        ),
        Err(DomainError::Conflict)
    ));
    assert!(matches!(
        ws.stream("source", MAX_STREAM_READ, |_| Err(DomainError::Cancelled)),
        Err(DomainError::Cancelled)
    ));
    assert!(ws.stream("..", MAX_STREAM_READ, |_| Ok(())).is_err());
    let file = fs::File::create(dir.path().join("oversize")).unwrap();
    file.set_len(MAX_STREAM_READ + 1).unwrap();
    assert!(matches!(
        ws.stream("oversize", MAX_STREAM_READ, |_| panic!(
            "oversized file must not be streamed"
        )),
        Err(DomainError::LimitExceeded(_))
    ));
}

#[test]
fn extended_image_formats_orientation_size_limits_and_cancellation_are_bounded() {
    use sha2::{Digest, Sha256};
    let _serial = IMAGE_FIXTURE.lock().unwrap();
    let base =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/tools/images");
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/image-extended-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 8);
    for row in oracle["rows"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        let bytes = if name == "padded-small.png" {
            let mut bytes = fs::read(base.join("small.png")).unwrap();
            bytes.resize(bytes.len() + 4 * 1024 * 1024, 0);
            bytes
        } else {
            fs::read(base.join(name)).unwrap()
        };
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), row["input_sha256"]);
        let result =
            ag1_platform::image_tools::read_with_resize(&bytes, row["resize"].as_bool().unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(result["content"][0]["text"], row["text"], "{name}");
        assert_eq!(result["content"][1]["mimeType"], row["mime"], "{name}");
        let output = STANDARD
            .decode(result["content"][1]["data"].as_str().unwrap())
            .unwrap();
        if row["unchanged"] == true {
            assert_eq!(output, bytes);
            assert_eq!(
                format!("{:x}", Sha256::digest(&output)),
                row["image_sha256"]
            );
        } else {
            assert!(output.len().div_ceil(3) * 4 < 9 * 1024 * 1024 / 2);
            let actual = image::load_from_memory(&output).unwrap().into_rgba8();
            let reference = image::load_from_memory(
                &fs::read(base.join(row["reference_file"].as_str().unwrap())).unwrap(),
            )
            .unwrap()
            .into_rgba8();
            assert_eq!(actual.dimensions(), reference.dimensions());
            assert_eq!(json!([actual.width(), actual.height()]), row["dimensions"]);
            let difference: u64 = actual
                .as_raw()
                .iter()
                .zip(reference.as_raw())
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            assert!(
                difference <= 2 * actual.as_raw().len() as u64,
                "{name}: decoded/resized channel MAE exceeds 2"
            );
        }
    }
    let mut oversized = fs::read(base.join("small.png")).unwrap();
    oversized.resize(8 * 1024 * 1024 + 1, 0);
    assert!(matches!(
        ag1_platform::image_tools::read(&oversized),
        Err(DomainError::LimitExceeded(_))
    ));
    let wide = fs::read(base.join("wide.png")).unwrap();
    for stage in [1, 3, 4, 5, 6, 7] {
        let mut calls = 0;
        let result = ag1_platform::image_tools::read_with_control(&wide, true, &mut || {
            calls += 1;
            if calls == stage {
                Err(DomainError::Cancelled)
            } else {
                Ok(())
            }
        });
        assert!(
            matches!(result, Err(DomainError::Cancelled)),
            "stage {stage}"
        );
        assert_eq!(calls, stage, "cancellation must stop subsequent stages");
    }
    assert!(matches!(
        ag1_platform::image_tools::read_with_control(&wide, true, &mut || Err(
            DomainError::TimedOut
        )),
        Err(DomainError::TimedOut)
    ));
    let small = fs::read(base.join("small.png")).unwrap();
    let mut stage = 0;
    ag1_platform::image_tools::read_with_control(&small, true, &mut || {
        stage += 1;
        if stage == 2 {
            let copy = small.clone();
            let concurrent = std::thread::spawn(move || ag1_platform::image_tools::read(&copy))
                .join()
                .unwrap();
            assert!(
                matches!(concurrent, Err(DomainError::QueueFull)),
                "only one decoder can run"
            );
        }
        Ok(())
    })
    .unwrap();
    let (dir, _, files, mut registry, scope) = setup();
    registry
        .activate(&["read".into()], &["read".into()].into())
        .unwrap();
    fs::write(dir.path().join("image.png"), &wide).unwrap();
    let call = call(
        &registry,
        "image-cancel",
        "read",
        json!({"path":"image.png"}),
    );
    let mut count = 0;
    let result = files.prepare_with_control(&call, &scope, &mut || {
        count += 1;
        if count == 4 {
            Err(DomainError::Cancelled)
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(DomainError::Cancelled)));
    assert_eq!(count, 4);
    // Cancellation releases the decoder permit for the next admitted image.
    assert!(
        ag1_platform::image_tools::read(&fs::read(base.join("small.png")).unwrap())
            .unwrap()
            .is_some()
    );
}

#[test]
fn loaded_resource_reads_match_sdk_and_grant_no_other_workspace_access() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/resource-read-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 4);
    for (index, row) in oracle["rows"].as_array().unwrap().iter().enumerate() {
        let fixture = tempfile::tempdir().unwrap();
        let project = fixture.path().join("project");
        fs::create_dir(&project).unwrap();
        for (name, contents) in oracle["files"].as_object().unwrap() {
            let path = fixture.path().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents.as_str().unwrap()).unwrap();
        }
        let service = WorkspaceService::new().unwrap();
        let workspace = service.register(&project).unwrap();
        let skill = service.register(&fixture.path().join("skill")).unwrap();
        let files = FileService::new(workspace.clone());
        if row["input"]["granted"] == true {
            assert_eq!(files.set_read_roots(0, vec![skill]).unwrap(), 1);
        }
        let mut registry = Registry::builtins().unwrap();
        registry
            .activate(&["read".into()], &["read".into()].into_iter().collect())
            .unwrap();
        let scope = Scope {
            workspace: workspace.id().as_str().into(),
            session: "fixture".into(),
            epoch: 1,
            revision: 1,
        };
        let path = row["input"]["path"]
            .as_str()
            .unwrap()
            .replace("/fixture/", &format!("{}/", fixture.path().display()));
        let call = call(
            &registry,
            &format!("resource-{index}"),
            "read",
            json!({"path":path}),
        );
        let prepared = files.prepare(&call, &scope);
        if row["output"]["denied"] == true {
            assert!(matches!(prepared, Err(DomainError::PermissionDenied)));
            continue;
        }
        let p = prepared.unwrap();
        assert!(!p.will_mutate());
        assert!(
            p.proposed_result()["details"]
                .get("inspectionEvidence")
                .is_none()
        );
        let ticket = authorize(call, &p, &registry, &scope, project.to_str().unwrap());
        let outcome = files
            .execute(
                p,
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(1),
                },
            )
            .unwrap();
        assert_eq!(outcome.result, row["output"]["result"], "resource {index}");
        assert!(outcome.published.is_none());
    }
}

#[test]
fn external_resource_read_plans_revalidate_revocation_content_scope_and_control() {
    let (project, _workspace, files, mut registry, scope) = setup();
    let resource = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(resource.path().join("SKILL.md"), "instructions\n").unwrap();
    fs::write(outside.path().join("secret"), "private\n").unwrap();
    symlink(
        outside.path().join("secret"),
        resource.path().join("escape"),
    )
    .unwrap();
    let root = WorkspaceService::new()
        .unwrap()
        .register(resource.path())
        .unwrap();
    assert_eq!(files.set_read_roots(0, vec![root.clone()]).unwrap(), 1);
    assert!(matches!(
        files.set_read_roots(0, vec![]),
        Err(DomainError::Conflict)
    ));
    registry
        .activate(
            &["read".into(), "write".into(), "edit".into()],
            &["read".into(), "write".into(), "edit".into()]
                .into_iter()
                .collect(),
        )
        .unwrap();
    let path = resource.path().join("SKILL.md");
    for (name, args) in [
        ("write", json!({"path":path,"content":"changed"})),
        (
            "edit",
            json!({"path":path,"oldText":"instructions","newText":"changed"}),
        ),
    ] {
        let c = call(&registry, name, name, args);
        assert!(matches!(
            files.prepare(&c, &scope),
            Err(DomainError::PermissionDenied)
        ));
    }
    assert!(
        matches!(
            files.relative_path(path.to_str().unwrap()),
            Err(DomainError::PermissionDenied)
        ),
        "listing/search path resolution retains workspace containment"
    );
    let escaped = call(
        &registry,
        "escape",
        "read",
        json!({"path":resource.path().join("escape")}),
    );
    assert!(files.prepare(&escaped, &scope).is_err());
    let read = call(&registry, "resource", "read", json!({"path":path}));
    let prepare = || files.prepare(&read, &scope).unwrap();
    let authorize_read = |p: &PreparedFile| {
        authorize(
            read.clone(),
            p,
            &registry,
            &scope,
            project.path().to_str().unwrap(),
        )
    };
    let run = |p, ticket, current_scope: &Scope, cancel: &AtomicBool| {
        files.execute(
            p,
            ticket,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: current_scope,
                policy_revision: 1,
                now: || 12,
                cancel,
                deadline: Instant::now() + Duration::from_secs(1),
            },
        )
    };
    let p = prepare();
    let ticket = authorize_read(&p);
    fs::write(&path, "replaced\n").unwrap();
    assert!(matches!(
        run(p, ticket, &scope, &AtomicBool::new(false)),
        Err(DomainError::Conflict)
    ));
    let p = prepare();
    let ticket = authorize_read(&p);
    let mut other_scope = scope.clone();
    other_scope.epoch += 1;
    assert!(matches!(
        run(p, ticket, &other_scope, &AtomicBool::new(false)),
        Err(DomainError::PermissionDenied)
    ));
    let p = prepare();
    let ticket = authorize_read(&p);
    assert!(matches!(
        run(p, ticket, &scope, &AtomicBool::new(true)),
        Err(DomainError::Cancelled)
    ));
    assert!(matches!(
        files.prepare_with_control(&read, &scope, &mut || Err(DomainError::TimedOut)),
        Err(DomainError::TimedOut)
    ));
    let p = prepare();
    let ticket = authorize_read(&p);
    assert_eq!(files.set_read_roots(1, vec![]).unwrap(), 2);
    assert!(matches!(
        run(p, ticket, &scope, &AtomicBool::new(false)),
        Err(DomainError::PermissionDenied)
    ));
    assert!(matches!(
        files.prepare(&read, &scope),
        Err(DomainError::PermissionDenied)
    ));
    assert_eq!(files.set_read_roots(2, vec![root]).unwrap(), 3);
    let p = prepare();
    let ticket = authorize_read(&p);
    assert_eq!(
        run(p, ticket, &scope, &AtomicBool::new(false))
            .unwrap()
            .result["content"][0]["text"],
        "replaced\n"
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "private\n"
    );
}

#[test]
fn selected_safety_symlink_case_is_rejected_during_native_file_preparation() {
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/safety-cases-oracle.json"
    ))
    .unwrap();
    let rows: Vec<_> = oracle["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| !row["symlinks"].as_array().unwrap().is_empty())
        .collect();
    assert_eq!(rows.len(), 1);
    let row = rows[0];
    assert_eq!(
        row["name"],
        "tool safety policy blocks file mutations that traverse a symlink outside the workspace"
    );
    let (dir, _workspace, files, registry, scope) = setup();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret.txt"), "untouched outside file").unwrap();
    for link in row["symlinks"].as_array().unwrap() {
        assert_eq!(link["target"], "/outside");
        let relative = link["path"]
            .as_str()
            .unwrap()
            .strip_prefix("/fixture/")
            .unwrap();
        symlink(outside.path(), dir.path().join(relative)).unwrap();
    }
    assert_eq!(row["calls"].as_array().unwrap().len(), 1);
    for original in row["calls"].as_array().unwrap() {
        assert_eq!(original["output"]["allow"], false);
        assert!(original["output"].get("approvalRequest").is_none());
        let input = &original["input"];
        let call = call(
            &registry,
            "symlink-policy",
            input["tool"].as_str().unwrap(),
            input["args"].clone(),
        );
        assert!(
            files.prepare(&call, &scope).is_err(),
            "A rejected source path must not reach policy approval or publication"
        );
    }
    assert_eq!(
        fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
        "untouched outside file"
    );
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
}

#[test]
fn queued_dispatch_captures_after_prior_edits_and_shares_turns_with_write() {
    for second_is_write in [false, true] {
        let (dir, _, files, registry, scope) = setup();
        fs::write(dir.path().join("file"), "alpha\nbeta\ngamma\n").unwrap();
        let cancel = AtomicBool::new(false);
        let deadline = Instant::now() + Duration::from_secs(5);
        let first = call(
            &registry,
            "first",
            "edit",
            json!({"path":"file","edits":[{"oldText":"alpha","newText":"ALPHA"}]}),
        );
        let prepared = files
            .prepare_queued(&first, &scope, &cancel, deadline)
            .unwrap();
        let second = if second_is_write {
            call(
                &registry,
                "second",
                "write",
                json!({"path":"./file","content":"replacement\n"}),
            )
        } else {
            call(
                &registry,
                "second",
                "edit",
                json!({"path":"./file","edits":[{"oldText":"beta","newText":"BETA"}]}),
            )
        };
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        std::thread::scope(|threads| {
            let second_worker = threads.spawn(|| {
                let prepared = files
                    .prepare_queued(&second, &scope, &cancel, deadline)
                    .unwrap();
                entered_tx.send(()).unwrap();
                let approved = authorize(
                    second.clone(),
                    &prepared,
                    &registry,
                    &scope,
                    dir.path().to_str().unwrap(),
                );
                files
                    .execute(
                        prepared,
                        approved,
                        ag1_platform::file_tools::FileExecution {
                            registry: &registry,
                            scope: &scope,
                            policy_revision: 1,
                            now: || 12,
                            cancel: &cancel,
                            deadline,
                        },
                    )
                    .unwrap();
            });
            // Holding the first proposal does not hold the workspace claim or
            // another file's queue. No mutation occurs before its approval.
            assert!(entered_rx.recv_timeout(Duration::from_millis(30)).is_err());
            assert_eq!(
                fs::read_to_string(dir.path().join("file")).unwrap(),
                "alpha\nbeta\ngamma\n"
            );
            let unrelated = call(
                &registry,
                "other",
                "write",
                json!({"path":"other","content":"separate"}),
            );
            let other_prepared = files
                .prepare_queued(&unrelated, &scope, &cancel, deadline)
                .unwrap();
            let approved = authorize(
                unrelated,
                &other_prepared,
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            files
                .execute(
                    other_prepared,
                    approved,
                    ag1_platform::file_tools::FileExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &cancel,
                        deadline,
                    },
                )
                .unwrap();
            assert_eq!(
                fs::read_to_string(dir.path().join("other")).unwrap(),
                "separate"
            );
            let approved = authorize(
                first,
                &prepared,
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            files
                .execute(
                    prepared,
                    approved,
                    ag1_platform::file_tools::FileExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &cancel,
                        deadline,
                    },
                )
                .unwrap();
            entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
            second_worker.join().unwrap();
        });
        assert_eq!(
            fs::read_to_string(dir.path().join("file")).unwrap(),
            if second_is_write {
                "replacement\n"
            } else {
                "ALPHA\nBETA\ngamma\n"
            }
        );
    }
}

#[test]
fn queued_proposal_rejection_cancellation_and_deadlines_release_turns_without_writes() {
    let (dir, ws, files, registry, scope) = setup();
    fs::write(dir.path().join("file"), "original\n").unwrap();
    let cancel = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(5);
    let write = call(
        &registry,
        "write",
        "write",
        json!({"path":"file","content":"replacement\n"}),
    );
    let first = files
        .prepare_queued(&write, &scope, &cancel, deadline)
        .unwrap();
    assert!(matches!(
        files.prepare_queued(
            &write,
            &scope,
            &cancel,
            Instant::now() + Duration::from_millis(20)
        ),
        Err(DomainError::TimedOut)
    ));
    assert!(matches!(
        files.prepare_queued(&write, &scope, &AtomicBool::new(true), deadline),
        Err(DomainError::Cancelled)
    ));
    drop(first); // Host rejects the approval: no publication, next turn can enter.
    let second = files
        .prepare_queued(&write, &scope, &cancel, deadline)
        .unwrap();
    let approved = authorize(
        write.clone(),
        &second,
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    assert!(matches!(
        files.execute(
            second,
            approved,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(true),
                deadline,
            }
        ),
        Err(DomainError::Cancelled)
    ));
    let third = files
        .prepare_queued(&write, &scope, &cancel, deadline)
        .unwrap();
    let approved = authorize(
        write.clone(),
        &third,
        &registry,
        &scope,
        dir.path().to_str().unwrap(),
    );
    let other_service = FileService::new(ws);
    assert!(matches!(
        other_service.execute(
            third,
            approved,
            ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &cancel,
                deadline,
            }
        ),
        Err(DomainError::PermissionDenied)
    ));
    let last = files
        .prepare_queued(&write, &scope, &cancel, deadline)
        .unwrap();
    drop(last);
    assert_eq!(
        fs::read_to_string(dir.path().join("file")).unwrap(),
        "original\n"
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn read_filename_fallbacks_match_sdk_and_bind_the_selected_file_revision() {
    let oracle: Value =
        serde_json::from_str(include_str!("../../../../fixtures/tools/path-oracle.json")).unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 8);
    for row in oracle["rows"].as_array().unwrap() {
        let (dir, ws, files, mut registry, scope) = setup();
        registry
            .activate(&["read".into()], &["read".into()].into())
            .unwrap();
        let input = &row["input"];
        let stored = input["stored"].as_str().unwrap();
        let requested = input["requested"].as_str().unwrap();
        let name = input["name"].as_str().unwrap();
        fs::write(dir.path().join(stored), name).unwrap();
        if let Some(other) = input["other"].as_str() {
            fs::write(dir.path().join(other), "not selected").unwrap();
        }
        for absolute in [false, true] {
            let requested = if absolute {
                dir.path().join(requested).to_str().unwrap().to_owned()
            } else {
                requested.into()
            };
            let read = call(&registry, "fallback", "read", json!({"path":requested}));
            let prepared = files.prepare(&read, &scope).unwrap();
            let source = ws.snapshot(row["output"].as_str().unwrap(), 1024).unwrap();
            assert_eq!(prepared.source_revision(), Some(&source.revision));
            let evidence = ag1_core::evidence::SourceReference::new(
                row["output"].as_str().unwrap(),
                &source.revision.sha256,
            )
            .unwrap();
            assert_eq!(
                prepared.proposed_result()["details"]["inspectionEvidence"]["source"],
                evidence.reference()
            );
            let ticket = authorize(
                read,
                &prepared,
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            let result = files
                .execute(
                    prepared,
                    ticket,
                    ag1_platform::file_tools::FileExecution {
                        registry: &registry,
                        scope: &scope,
                        policy_revision: 1,
                        now: || 12,
                        cancel: &AtomicBool::new(false),
                        deadline: Instant::now() + Duration::from_secs(1),
                    },
                )
                .unwrap();
            assert_eq!(result.result["content"][0]["text"], name);
        }
        let read = call(
            &registry,
            "stale-fallback",
            "read",
            json!({"path":requested}),
        );
        let prepared = files.prepare(&read, &scope).unwrap();
        let ticket = authorize(
            read,
            &prepared,
            &registry,
            &scope,
            dir.path().to_str().unwrap(),
        );
        fs::write(dir.path().join(stored), "changed after preparation").unwrap();
        assert!(matches!(
            files.execute(
                prepared,
                ticket,
                ag1_platform::file_tools::FileExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(1)
                }
            ),
            Err(DomainError::Conflict)
        ));
    }
}

#[test]
fn repaired_workspace_paths_share_containment_and_fallbacks_cannot_follow_symlinks() {
    let (dir, _, files, mut registry, scope) = setup();
    registry
        .activate(
            &["read".into(), "write".into()],
            &["read".into(), "write".into()].into(),
        )
        .unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("nested/child.txt"), "child").unwrap();
    let malformed = format!(
        "{}/nested",
        dir.path().to_str().unwrap().trim_start_matches('/')
    );
    assert_eq!(files.relative_path(&malformed).unwrap(), "nested");
    for input in [
        format!("{malformed}/child.txt"),
        "nested/child.txt".into(),
        dir.path().join("nested/child.txt").to_str().unwrap().into(),
    ] {
        assert_eq!(files.relative_path(&input).unwrap(), "nested/child.txt");
        let read = call(&registry, "repair", "read", json!({"path":input}));
        assert_eq!(
            files.prepare(&read, &scope).unwrap().proposed_result()["content"][0]["text"],
            "child"
        );
    }
    assert!(matches!(
        files.relative_path("/different-root/child.txt"),
        Err(DomainError::PermissionDenied)
    ));
    assert_ne!(
        files
            .relative_path(&format!(
                "{}-other/nested",
                dir.path().to_str().unwrap().trim_start_matches('/')
            ))
            .unwrap(),
        "nested"
    );
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("secret"), "outside secret").unwrap();
    symlink(
        outside.path().join("secret"),
        dir.path().join("Capture d’écran.txt"),
    )
    .unwrap();
    symlink(outside.path(), dir.path().join("dire\u{301}ctory")).unwrap();
    for path in ["Capture d'écran.txt", "diréctory/secret"] {
        let read = call(&registry, "fallback-link", "read", json!({"path":path}));
        assert!(matches!(
            files.prepare(&read, &scope),
            Err(DomainError::PermissionDenied)
        ));
    }
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "outside secret"
    );
    // Filename fallback belongs only to reads: a write uses its literal path.
    fs::remove_file(dir.path().join("Capture d’écran.txt")).unwrap();
    fs::write(dir.path().join("Capture d’écran.txt"), "keep curly file").unwrap();
    let write = call(
        &registry,
        "literal-write",
        "write",
        json!({"path":"Capture d'écran.txt","content":"new literal"}),
    );
    assert!(
        files
            .prepare(&write, &scope)
            .unwrap()
            .source_revision()
            .is_none()
    );
    let read = call(
        &registry,
        "cancelled-fallback",
        "read",
        json!({"path":"Capture d'écran.txt"}),
    );
    assert!(matches!(
        files.prepare_with_control(&read, &scope, &mut || Err(DomainError::Cancelled)),
        Err(DomainError::Cancelled)
    ));
}

#[test]
fn rejected_foreign_paths_offer_bounded_workspace_recovery_without_following_links() {
    let (dir, _, files, _, _) = setup();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("outside-only.txt"), "must not inspect").unwrap();
    symlink(outside.path(), dir.path().join("outside")).unwrap();
    for subdir in ["a", "b", "c", "d"] {
        fs::create_dir(dir.path().join(subdir)).unwrap();
        fs::write(dir.path().join(subdir).join("Cyber_harness.md"), "# spec\n").unwrap();
    }
    let hint = files
        .rejection_hint(
            &DomainError::PermissionDenied,
            "/home/runner/work/cyber-h/advanced-cyber-harness/Cyber_harness.md",
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
    assert!(hint.contains("Path escapes the workspace"));
    assert!(hint.contains(&format!("Workspace root: {}", dir.path().display())));
    assert!(hint.contains("a/Cyber_harness.md, b/Cyber_harness.md, c/Cyber_harness.md"));
    assert!(!hint.contains("d/Cyber_harness.md"));
    assert!(hint.len() <= 8192);
    let hint = files
        .rejection_hint(
            &DomainError::PermissionDenied,
            outside.path().join("outside-only.txt").to_str().unwrap(),
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
    assert!(!hint.contains("Files with this name"));
    assert!(
        files
            .rejection_hint(
                &DomainError::PermissionDenied,
                "a/Cyber_harness.md",
                &mut || Ok(())
            )
            .unwrap()
            .is_none()
    );
    assert!(
        files
            .rejection_hint(&DomainError::Cancelled, "/outside", &mut || Ok(()))
            .unwrap()
            .is_none()
    );
    let mut checks = 0;
    assert!(matches!(
        files.rejection_hint(
            &DomainError::PermissionDenied,
            "/outside/Cyber_harness.md",
            &mut || {
                checks += 1;
                if checks == 3 {
                    Err(DomainError::Cancelled)
                } else {
                    Ok(())
                }
            }
        ),
        Err(DomainError::Cancelled)
    ));
    assert_eq!(
        fs::read_to_string(outside.path().join("outside-only.txt")).unwrap(),
        "must not inspect"
    );
}

#[test]
fn selected_original_file_cases_match_sdk_results_bytes_modes_and_rejections() {
    let _image_fixture = IMAGE_FIXTURE.lock().unwrap();
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-files-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 42);
    for row in oracle["rows"].as_array().unwrap() {
        for (index, input) in row["calls"].as_array().unwrap().iter().enumerate() {
            let (dir, _ws, files, mut registry, scope) = setup();
            registry
                .activate(
                    &["read".into(), "write".into(), "edit".into()],
                    &["read".into(), "write".into(), "edit".into()].into(),
                )
                .unwrap();
            let args: Value = serde_json::from_str(
                &input["args"]
                    .to_string()
                    .replace("/fixture", dir.path().to_str().unwrap()),
            )
            .unwrap();
            let path = std::path::PathBuf::from(args["path"].as_str().unwrap());
            assert!(path.starts_with(dir.path()));
            if input["before"]["directory"] == true {
                fs::create_dir_all(&path).unwrap();
            } else if let Some(bytes) = input["before"]["bytes"].as_str() {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, STANDARD.decode(bytes).unwrap()).unwrap();
                fs::set_permissions(
                    &path,
                    fs::Permissions::from_mode(input["before"]["mode"].as_u64().unwrap() as u32),
                )
                .unwrap();
            }
            let state = || {
                if !path.exists() {
                    Value::Null
                } else if path.is_dir() {
                    json!({"directory":true})
                } else {
                    json!({"bytes":STANDARD.encode(fs::read(&path).unwrap()),"mode":fs::metadata(&path).unwrap().permissions().mode()&0o777})
                }
            };
            let result = (|| {
                let call = registry.prepare(
                    "selected-file",
                    input["output"]["result"]["details"]["inspectionEvidence"]["toolCallId"]
                        .as_str()
                        .unwrap_or(&format!("selected-{index}")),
                    input["tool"].as_str().unwrap(),
                    &args,
                    true,
                )?;
                let prepared = files.prepare(&call, &scope)?;
                assert_eq!(state(), input["before"], "preparation: {}", row["name"]);
                let ticket = authorize(
                    call,
                    &prepared,
                    &registry,
                    &scope,
                    dir.path().to_str().unwrap(),
                );
                files
                    .execute(
                        prepared,
                        ticket,
                        ag1_platform::file_tools::FileExecution {
                            registry: &registry,
                            scope: &scope,
                            policy_revision: 1,
                            now: || 12,
                            cancel: &AtomicBool::new(false),
                            deadline: Instant::now() + Duration::from_secs(3),
                        },
                    )
                    .map(|outcome| outcome.result)
            })();
            let output = match result {
                Ok(result) => json!({"result":result}),
                Err(DomainError::InvalidInput(error)) => json!({"error":error}),
                Err(error) => panic!("{}: {error:?}", row["name"]),
            };
            let output: Value = serde_json::from_str(
                &output
                    .to_string()
                    .replace(dir.path().to_str().unwrap(), "/fixture"),
            )
            .unwrap();
            let mut expected = input["output"].clone();
            if let Some(bytes) = input["before"]["bytes"].as_str()
                && let Some(result) = expected.get_mut("result") {
                let relative = path.strip_prefix(dir.path()).unwrap().to_str().unwrap();
                native_file_provenance(result, input["tool"].as_str().unwrap(), relative, &STANDARD.decode(bytes).unwrap());
            }
            assert_eq!(output, expected, "{}", row["name"]);
            assert_eq!(state(), input["after"], "final bytes/mode: {}", row["name"]);
        }
    }
}

#[test]
fn selected_original_search_and_listing_cases_match_sdk_order_metadata_and_hints() {
    use ag1_platform::{process::ProcessSupervisor, search_tools::SearchService};
    let oracle: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/tools/selected-search-oracle.json"
    ))
    .unwrap();
    assert_eq!(oracle["rows"].as_array().unwrap().len(), 27);
    for row in oracle["rows"].as_array().unwrap() {
        for input in row["calls"].as_array().unwrap() {
            let (dir, ws, files, mut registry, scope) = setup();
            registry
                .activate(
                    &["grep".into(), "find".into(), "ls".into()],
                    &["grep".into(), "find".into(), "ls".into()].into(),
                )
                .unwrap();
            for (path, entry) in input["before"].as_object().unwrap() {
                let path = dir.path().join(path);
                if entry["directory"] == true {
                    fs::create_dir_all(path).unwrap();
                } else {
                    fs::create_dir_all(path.parent().unwrap()).unwrap();
                    fs::write(
                        &path,
                        STANDARD.decode(entry["bytes"].as_str().unwrap()).unwrap(),
                    )
                    .unwrap();
                }
            }
            let spools = tempfile::tempdir().unwrap();
            let search = SearchService::new(
                ws,
                Arc::new(
                    ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref())
                        .unwrap(),
                ),
                "/usr/bin/rg".as_ref(),
                "/usr/bin/fd".as_ref(),
                spools.path(),
            )
            .unwrap();
            let args: Value = serde_json::from_str(
                &input["args"]
                    .to_string()
                    .replace("/fixture", dir.path().to_str().unwrap()),
            )
            .unwrap();
            let call = call(
                &registry,
                input["callId"].as_str().unwrap(),
                input["tool"].as_str().unwrap(),
                args,
            );
            let listing = input["tool"] == "ls";
            let target = if listing {
                files.list_target_revision()
            } else {
                search.target_revision()
            };
            let ticket = search_ticket(
                call,
                &target,
                &registry,
                &scope,
                dir.path().to_str().unwrap(),
            );
            let execution = ag1_platform::file_tools::FileExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(3),
            };
            let result = if listing {
                files.execute_list(ticket, execution)
            } else {
                search.execute(ticket, execution)
            };
            let result = match result {
                Ok(outcome) => json!({"result":outcome.result}),
                Err(DomainError::InvalidInput(error)) => json!({"error":error}),
                Err(DomainError::PermissionDenied) if input["args"]["path"] == "/" => {
                    let hint = search
                        .rejection_hint(&DomainError::PermissionDenied, "/", &mut || Ok(()))
                        .unwrap()
                        .unwrap();
                    assert!(hint.contains(
                        "workspace-relative path like ., scripts, src, or packages/*/src"
                    ));
                    assert!(hint.contains(dir.path().to_str().unwrap()));
                    assert!(
                        input["output"]["error"]
                            .as_str()
                            .unwrap()
                            .contains("workspace-relative path like ., scripts, src, or packages")
                    );
                    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
                    continue;
                }
                Err(error) => panic!("{}: {error:?}", row["name"]),
            };
            let result: Value = serde_json::from_str(
                &result
                    .to_string()
                    .replace(dir.path().to_str().unwrap(), "/fixture"),
            )
            .unwrap();
            if row["name"]
                == "Coding Agent Tools find tool should emit compact result-limit notices"
            {
                // fd stops after two matches; its parallel filesystem traversal
                // does not promise which two arrive first. The pinned SDK test
                // requires the limit notice, not one captured traversal order.
                let text = result["result"]["content"][0]["text"].as_str().unwrap();
                let paths: Vec<_> = text
                    .strip_suffix("\n\n[2 results limit; next limit=4]")
                    .unwrap()
                    .split('\n')
                    .collect();
                assert_eq!(paths.len(), 2);
                assert_ne!(paths[0], paths[1]);
                assert!(
                    paths
                        .iter()
                        .all(|p| ["alpha.txt", "beta.txt", "gamma.txt"].contains(p))
                );
                assert_eq!(
                    result,
                    json!({"result":{"content":[{"type":"text","text":text}],
                        "details":{"resultCount":2,"firstMatchPath":paths[0],
                            "firstMatchIsDirectory":false,"resultLimitReached":2}}})
                );
            } else {
                assert_eq!(result, input["output"], "{}", row["name"]);
            }
            assert_eq!(input["before"], input["after"]);
            for (path, entry) in input["before"].as_object().unwrap() {
                let path = dir.path().join(path);
                if entry["directory"] == true {
                    assert!(path.is_dir());
                } else {
                    assert_eq!(STANDARD.encode(fs::read(path).unwrap()), entry["bytes"]);
                }
            }
            assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
        }
    }
}

#[test]
fn native_sandbox_settings_and_argv_preserve_host_grants_and_literal_arguments() {
    use ag1_core::{SessionId, context::features::Features};
    use ag1_platform::{
        process::ProcessSupervisor,
        shell_tools::{Sandbox, ShellConfig, ShellExecution, ShellService},
    };
    use ag1_storage::{Store, context::ContextStore};
    for (raw, expected) in [
        (Some("off"), Some(Sandbox::Off)),
        (Some(" STRICT "), Some(Sandbox::Strict)),
        (Some("Workspace"), Some(Sandbox::Workspace)),
        (None, None),
        (Some(""), None),
        (Some("bogus"), None),
    ] {
        assert_eq!(Sandbox::parse(raw), expected);
    }
    let (dir, _, _, mut registry, mut scope) = setup();
    let root = dir.path().join("workspace ' ; printf not-executed");
    fs::create_dir_all(root.join("bin")).unwrap();
    let ws = WorkspaceService::new().unwrap().register(&root).unwrap();
    scope.workspace = ws.id().as_str().into();
    registry
        .activate(&["bash".into()], &["bash".into()].into())
        .unwrap();
    let link = dir.path().join("linked-bin");
    symlink(root.join("bin"), &link).unwrap();
    let wrapper = dir.path().join("argv-fixture");
    fs::write(
        &wrapper,
        "#!/bin/bash\nprintf 'RUSTUP=%s\\n' \"$RUSTUP_HOME\"\nprintf '%s\\n' \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let spools = tempfile::tempdir().unwrap();
    let processes =
        Arc::new(ProcessSupervisor::new(env!("CARGO_BIN_EXE_ag1-process-host").as_ref()).unwrap());
    let config = ShellConfig {
        executable: "/bin/bash".into(),
        env: [
            (
                "PATH".into(),
                format!(
                    "{}:{}:/opt/jdk/bin:/usr/bin",
                    root.join("bin").display(),
                    link.display()
                ),
            ),
            ("RUSTUP_HOME".into(), "/fixture/rustup".into()),
        ]
        .into(),
        command_prefix: None,
        command_template: None,
        sandbox: Sandbox::Off,
        bubblewrap: Some(wrapper),
        output_bytes: 65536,
        address_space: Some(1024 * 1024 * 1024),
    };
    assert_eq!(
        config.clone().with_sandbox_from_environment(None).sandbox,
        Sandbox::Off
    );
    let mut environment = config.clone();
    environment
        .env
        .insert("AG1_BASH_SANDBOX".into(), " STRICT ".into());
    assert_eq!(
        environment
            .clone()
            .with_sandbox_from_environment(None)
            .sandbox,
        Sandbox::Strict
    );
    assert_eq!(
        environment
            .clone()
            .with_sandbox_from_environment(Some(Sandbox::Off))
            .sandbox,
        Sandbox::Off
    );
    environment
        .env
        .insert("AG1_BASH_SANDBOX".into(), "invalid".into());
    assert_eq!(
        environment
            .clone()
            .with_sandbox_from_environment(None)
            .sandbox,
        Sandbox::Off
    );
    environment
        .env
        .insert("AG1_BASH_SANDBOX_COMMAND".into(), "   ".into());
    assert!(
        environment
            .clone()
            .with_sandbox_from_environment(None)
            .command_template
            .is_none()
    );
    environment.env.insert(
        "AG1_BASH_SANDBOX_COMMAND".into(),
        "sandbox --cwd {cwd} -- bash -lc {command}".into(),
    );
    let templated = ShellService::new(
        ws.clone(),
        processes.clone(),
        spools.path(),
        environment.with_sandbox_from_environment(None),
    )
    .unwrap();
    let input = call(
        &registry,
        "template",
        "bash",
        json!({"command":"printf 'hello'","timeout":3}),
    );
    let prepared = templated.prepare(&input, &registry, &scope).unwrap();
    assert_eq!(
        prepared.call().args()["command"],
        ag1_core::tools::shell::sandbox_template(
            "sandbox --cwd {cwd} -- bash -lc {command}",
            "printf 'hello'",
            root.to_str().unwrap()
        )
        .unwrap()
    );
    assert_eq!(prepared.call().args()["timeout"], 3);
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    let timestamp = "2026-09-05T00:00:00.000Z";
    store
        .create_session(
            "fixture",
            json!({"type":"session","version":3,"id":"fixture","cwd":root,"timestamp":timestamp})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    store
        .register_origin("fixture", "fixture", &[18; 32])
        .unwrap();
    let mut stamps = std::collections::BTreeSet::new();
    for (index, (mode, network)) in [
        (Sandbox::Workspace, true),
        (Sandbox::Workspace, false),
        (Sandbox::Strict, true),
        (Sandbox::Strict, false),
    ]
    .into_iter()
    .enumerate()
    {
        let mut config = config.clone();
        config.sandbox = mode;
        let shell = ShellService::new_with_network(
            ws.clone(),
            processes.clone(),
            spools.path(),
            config,
            network,
        )
        .unwrap();
        let command = "printf 'hi'; echo 'literal $(not-executed)'";
        let input = call(
            &registry,
            &format!("argv-{index}"),
            "bash",
            json!({"command":command}),
        );
        let plan = shell.prepare(&input, &registry, &scope).unwrap();
        assert!(stamps.insert(plan.target_revision().to_owned()));
        let ticket = search_ticket(
            plan.call().clone(),
            plan.target_revision(),
            &registry,
            &scope,
            root.to_str().unwrap(),
        );
        let mut artifacts = ContextStore::new(
            &store,
            SessionId::new("fixture").unwrap(),
            "fixture",
            &format!("argv-{index}"),
            timestamp,
            Features::default(),
        )
        .unwrap();
        let result = shell
            .execute(
                plan,
                ticket,
                ShellExecution {
                    registry: &registry,
                    scope: &scope,
                    policy_revision: 1,
                    now: || 12,
                    cancel: &AtomicBool::new(false),
                    deadline: Instant::now() + Duration::from_secs(3),
                },
                &mut artifacts,
                |_| {},
            )
            .unwrap()
            .result;
        let text = result["content"][0]["text"].as_str().unwrap();
        let argv: Vec<_> = text.lines().collect();
        assert_eq!(argv[0], "RUSTUP=/fixture/rustup");
        assert_eq!(
            argv.contains(&"--unshare-net"),
            mode == Sandbox::Strict || !network
        );
        for flag in [
            "--unshare-ipc",
            "--unshare-pid",
            "--unshare-user",
            "--die-with-parent",
            "--ro-bind",
            "--tmpfs",
            "--proc",
            "--dev",
            "--bind-fd",
        ] {
            assert!(argv.contains(&flag), "{text}");
        }
        let bind = argv.iter().position(|a| *a == "--bind-fd").unwrap();
        assert!(argv[bind + 1].parse::<u32>().is_ok());
        assert_eq!(argv[bind + 2], root.to_str().unwrap());
        for (flag, values) in [
            ("--ro-bind", vec!["/", "/"]),
            ("--tmpfs", vec!["/tmp"]),
            ("--proc", vec!["/proc"]),
            ("--dev", vec!["/dev"]),
            ("--dir", vec!["/tmp/ag1-home"]),
            ("--chdir", vec![root.to_str().unwrap()]),
        ] {
            let index = argv.iter().position(|a| *a == flag).unwrap();
            assert_eq!(&argv[index + 1..index + 1 + values.len()], values);
        }
        assert!(bind > argv.iter().position(|a| *a == "--tmpfs").unwrap());
        assert_eq!(
            argv.iter()
                .filter(|a| **a == root.to_str().unwrap())
                .count(),
            2
        );
        assert_eq!(argv.last(), Some(&command));
        assert_eq!(
            &argv[argv.len() - 7..argv.len() - 1],
            ["/bin/bash", "--noprofile", "--norc", "-o", "pipefail", "-c"]
        );
        let home = argv.iter().position(|a| *a == "HOME").unwrap();
        assert_eq!(argv[home + 1], "/tmp/ag1-home");
        let path = argv.iter().position(|a| *a == "PATH").unwrap();
        assert!(argv[path + 1].contains("/opt/jdk/bin"));
        assert!(!argv[path + 1].contains(root.to_str().unwrap()));
        assert!(!argv[path + 1].contains(link.to_str().unwrap()));
    }
    let mut requested = config.clone();
    requested.sandbox = Sandbox::Workspace;
    let sandbox =
        ShellService::new(ws.clone(), processes.clone(), spools.path(), requested).unwrap();
    let call = call(
        &registry,
        "sandbox-availability",
        "bash",
        json!({"command":"printf no > fallback-must-not-run"}),
    );
    let plan = sandbox.prepare(&call, &registry, &scope).unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        root.to_str().unwrap(),
    );
    fs::remove_file(config.bubblewrap.as_ref().unwrap()).unwrap();
    assert!(matches!(
        sandbox.prepare(&call, &registry, &scope),
        Err(DomainError::Unavailable(_))
    ));
    let mut artifacts = ContextStore::new(
        &store,
        SessionId::new("fixture").unwrap(),
        "fixture",
        "sandbox-availability",
        timestamp,
        Features::default(),
    )
    .unwrap();
    assert!(matches!(
        sandbox.execute(
            plan,
            ticket,
            ShellExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(3)
            },
            &mut artifacts,
            |_| {}
        ),
        Err(DomainError::Unavailable(_))
    ));
    // A present but failing backend cannot silently run the command without isolation.
    fs::write(
        config.bubblewrap.as_ref().unwrap(),
        "#!/bin/bash\nexit 19\n",
    )
    .unwrap();
    fs::set_permissions(
        config.bubblewrap.as_ref().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let plan = sandbox.prepare(&call, &registry, &scope).unwrap();
    let ticket = search_ticket(
        plan.call().clone(),
        plan.target_revision(),
        &registry,
        &scope,
        root.to_str().unwrap(),
    );
    let result = sandbox
        .execute(
            plan,
            ticket,
            ShellExecution {
                registry: &registry,
                scope: &scope,
                policy_revision: 1,
                now: || 12,
                cancel: &AtomicBool::new(false),
                deadline: Instant::now() + Duration::from_secs(3),
            },
            &mut artifacts,
            |_| {},
        )
        .unwrap()
        .result;
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Command exited with code 19")
    );
    assert!(!root.join("fallback-must-not-run").exists());
    assert_eq!(fs::read_dir(spools.path()).unwrap().count(), 0);
}
