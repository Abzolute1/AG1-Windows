#![cfg(target_os = "linux")]
use ag1_core::DomainError;
use ag1_platform::{projects::godot::*, resource_fs::ScanControl, workspace::WorkspaceService};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};
fn control(c: &AtomicBool) -> ScanControl<'_> {
    ScanControl {
        cancel: c,
        deadline: Instant::now() + Duration::from_secs(3),
    }
}
fn hello(path: &std::path::Path) -> Envelope {
    Envelope {
        protocol_version: 1,
        kind: "host.hello".into(),
        correlation_id: Some("hello-1".into()),
        session_nonce: Some("0123456789abcdef0123456789abcdef".into()),
        payload: json!({"schema_sha256":SCHEMA_SHA256,"project_path":path,"engine_build":"4.7.2.ai-first","engine_commit":"fixture","capabilities":["host.request.v1","sidecar.request.v1"]}),
    }
}
#[test]
fn pinned_protocol_fixture_and_rejections() {
    let fixture = include_bytes!(
        "../../../../reference/ag1-executable/packages/godot-agent-runtime/protocol/fixtures/host_hello.jsonl"
    );
    let decoded = decode(fixture.strip_suffix(b"\n").unwrap_or(fixture)).unwrap();
    assert_eq!(decoded.protocol_version, 1);
    for replacement in [
        json!({"protocol_version":2,"kind":"host.hello","payload":{}}),
        json!({"protocol_version":1,"kind":"unknown","payload":{}}),
        json!({"protocol_version":1,"kind":"host.hello","payload":{},"session_nonce":"short"}),
        json!({"protocol_version":1,"kind":"host.hello","payload":{},"extra":1}),
    ] {
        assert!(decode(&serde_json::to_vec(&replacement).unwrap()).is_err());
    }
    assert!(decode(b"{}\n{}").is_err());
    assert!(decode(&vec![b' '; 1048577]).is_err());
}
#[test]
fn handshake_binds_version_hash_nonce_and_project() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let e = hello(dir.path());
    assert!(handshake(&e, &workspace).is_ok());
    for field in [
        "schema_sha256",
        "project_path",
        "capabilities",
        "engine_commit",
    ] {
        let mut bad = e.clone();
        bad.payload[field] = Value::Null;
        assert!(handshake(&bad, &workspace).is_err());
    }
    let mut bad = e.clone();
    bad.session_nonce = None;
    assert!(handshake(&bad, &workspace).is_err());
}
fn broker_host(mut stream: UnixStream, path: std::path::PathBuf, bad_nonce: bool) {
    stream.write_all(&encode(&hello(&path)).unwrap()).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(
        decode(line.trim_end().as_bytes()).unwrap().kind,
        "sidecar.hello"
    );
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap() == 0 {
            break;
        }
        let req = decode(line.trim_end().as_bytes()).unwrap();
        let response = Envelope {
            protocol_version: 1,
            kind: "host.response".into(),
            session_nonce: if bad_nonce {
                Some("wrong-nonce-123456".into())
            } else {
                req.session_nonce
            },
            correlation_id: req.correlation_id,
            payload: json!({"ok":true,"result":{"transaction_id":"tx-1","method":req.payload["method"],"params":req.payload["params"]}}),
        };
        stream.write_all(&encode(&response).unwrap()).unwrap();
    }
}
#[test]
fn transactions_allowlist_and_build_mode_preserve_exact_payload() {
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let (a, b) = UnixStream::pair().unwrap();
    let p = dir.path().to_path_buf();
    let host = std::thread::spawn(move || broker_host(b, p, false));
    let c = AtomicBool::new(false);
    let mut broker = Broker::accept(a, &ws, &control(&c)).unwrap();
    assert!(
        broker
            .request("shell.exec", json!({}), true, &control(&c))
            .is_err()
    );
    assert!(
        broker
            .request("godot.transaction.begin", json!({}), false, &control(&c))
            .is_err()
    );
    assert!(
        broker
            .request(
                "godot.file.write.text",
                json!({"transaction_id":"tx-1","operation_id":"op"}),
                true,
                &control(&c)
            )
            .is_err()
    );
    broker
        .request(
            "godot.transaction.begin",
            json!({"transaction_id":"tx-1"}),
            true,
            &control(&c),
        )
        .unwrap();
    let params = json!({"transaction_id":"tx-1","operation_id":"op-1","expected_revision":"a".repeat(64),"path":"res://new.gd","text":"extends Node"});
    let result = broker
        .request("godot.file.write.text", params.clone(), true, &control(&c))
        .unwrap();
    assert_eq!(result["params"], params);
    broker
        .request(
            "godot.transaction.commit",
            json!({"transaction_id":"tx-1"}),
            true,
            &control(&c),
        )
        .unwrap();
    assert!(
        broker
            .request("godot.file.write.text", params, true, &control(&c))
            .is_err()
    );
    broker.close();
    host.join().unwrap();
}
#[test]
fn nonce_failure_and_parent_eof_close_without_replay() {
    for wrong in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let ws = WorkspaceService::new()
            .unwrap()
            .register(dir.path())
            .unwrap();
        let (a, mut b) = UnixStream::pair().unwrap();
        let p = dir.path().to_path_buf();
        let host = std::thread::spawn(move || {
            if wrong {
                broker_host(b, p, true)
            } else {
                b.write_all(&encode(&hello(&p)).unwrap()).unwrap();
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let c = AtomicBool::new(false);
        let mut broker = Broker::accept(a, &ws, &control(&c)).unwrap();
        assert!(
            broker
                .request("godot.project.get_state", json!({}), false, &control(&c))
                .is_err()
        );
        assert!(matches!(
            broker.request("godot.project.get_state", json!({}), false, &control(&c)),
            Err(DomainError::Unavailable(_))
        ));
        broker.close();
        host.join().unwrap();
    }
}
#[test]
fn watchdog_deadline_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let (a, _b) = UnixStream::pair().unwrap();
    let c = AtomicBool::new(false);
    assert!(matches!(
        Broker::accept(
            a,
            &ws,
            &ScanControl {
                cancel: &c,
                deadline: Instant::now() + Duration::from_millis(30)
            }
        ),
        Err(DomainError::TimedOut)
    ));
}
#[test]
fn visual_hash_path_vision_and_fork_acl() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let image = image::DynamicImage::new_rgb8(2, 2);
    let mut png = std::io::Cursor::new(vec![]);
    image.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let png = png.into_inner();
    let sha = format!("{:x}", Sha256::digest(&png));
    let path = format!(".godot/ai/visual/blobs/{sha}.png");
    ws.create_checked(&path, &png, true, || Ok(())).unwrap();
    let result = json!({"sha256":sha,"private_path":format!("res://{path}"),"bytes":png.len(),"width":2,"height":2});
    assert_eq!(capture(&ws, &result, true).unwrap(), png);
    assert!(capture(&ws, &result, false).is_err());
    let mut bad = result.clone();
    bad["private_path"] = json!("res://../../secret");
    assert!(capture(&ws, &bad, true).is_err());
    bad = result.clone();
    bad["width"] = json!(3);
    assert!(capture(&ws, &bad, true).is_err());
    let store = ag1_storage::Store::open(&dir.path().join("store")).unwrap();
    for id in ["parent", "other"] {
        store.create_session(id,&serde_json::to_vec(&json!({"type":"session","version":3,"id":id,"cwd":"/fixture","timestamp":"2026-09-07T00:00:00.000Z"})).unwrap()).unwrap();
    }
    store.register_origin("origin", "parent", &[7; 32]).unwrap();
    let record = ag1_platform::projects::preview::retain(
        &store,
        "parent",
        "origin",
        &png,
        "godot-visual",
        "Inspect fixture",
        "editor_2d",
    )
    .unwrap();
    let id = record["id"].as_str().unwrap();
    assert!(store.artifact_record("other", "origin", id).is_err());
    store.fork("parent", "child", None).unwrap();
    assert!(store.artifact_record("child", "origin", id).is_ok());
}
#[test]
fn stage_preserves_symlinks_modes_hashes_and_failed_destination_cleanup() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("source")).unwrap();
    std::fs::create_dir(dir.path().join("stages")).unwrap();
    let service = WorkspaceService::new().unwrap();
    let source = service.register(&dir.path().join("source")).unwrap();
    let parent = service.register(&dir.path().join("stages")).unwrap();
    let bytes = b"owned fake engine; never execute";
    let rev = source
        .create_checked("bin/llama-server", bytes, true, || Ok(()))
        .unwrap();
    source
        .set_mode_checked("bin/llama-server", &rev, 0o755)
        .unwrap();
    source
        .create_local_link("bin/engine-link", "llama-server")
        .unwrap();
    let sha = format!("{:x}", Sha256::digest(bytes));
    let manifest = json!({"formatVersion":1,"backend":"fixture","sourceRepo":source.root(),"sourceCommit":"fixture","serverSha256":sha,"serverPath":"bin/llama-server","libraryPath":"bin","externalDependencies":[],"files":[{"path":"bin/llama-server","kind":"file","mode":493,"size":bytes.len(),"sha256":sha},{"path":"bin/engine-link","kind":"symlink","mode":511,"target":"llama-server"}]});
    verify_stage(&source, &manifest).unwrap();
    let c = AtomicBool::new(false);
    let stage = copy_stage(&source, &parent, "copied", &manifest, &control(&c)).unwrap();
    verify_stage(&stage, &manifest).unwrap();
    assert_eq!(
        stage.link_snapshot("bin/engine-link").unwrap().0,
        "llama-server"
    );
    assert!(copy_stage(&source, &parent, "copied", &manifest, &control(&c)).is_err());
    let old = source.snapshot("bin/llama-server", 65536).unwrap();
    source
        .replace("bin/llama-server", &old.revision, b"tampered")
        .unwrap();
    assert!(copy_stage(&source, &parent, "failed", &manifest, &control(&c)).is_err());
    assert!(!parent.root().join("failed").exists());
    assert!(verify_stage(&stage, &manifest).is_ok());
}

#[test]
fn request_watchdog_closes_peer_before_broker_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceService::new()
        .unwrap()
        .register(dir.path())
        .unwrap();
    let (a, mut b) = UnixStream::pair().unwrap();
    let path = dir.path().to_path_buf();
    let host = std::thread::spawn(move || {
        b.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        b.write_all(&encode(&hello(&path)).unwrap()).unwrap();
        let mut reader = std::io::BufReader::new(b);
        let mut line = String::new();
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
        line.clear();
        std::io::BufRead::read_line(&mut reader, &mut line).unwrap();
        assert!(line.contains("host.request"));
        line.clear();
        assert_eq!(
            std::io::BufRead::read_line(&mut reader, &mut line).unwrap(),
            0
        );
    });
    let c = AtomicBool::new(false);
    let mut broker = Broker::accept(a, &ws, &control(&c)).unwrap();
    assert!(matches!(
        broker.request(
            "godot.project.get_state",
            json!({}),
            false,
            &ScanControl {
                cancel: &c,
                deadline: Instant::now() + Duration::from_millis(30)
            }
        ),
        Err(DomainError::TimedOut)
    ));
    host.join().unwrap();
    assert!(matches!(
        broker.request("godot.project.get_state", json!({}), false, &control(&c)),
        Err(DomainError::Unavailable(_))
    ));
}
